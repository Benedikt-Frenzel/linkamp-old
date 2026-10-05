pub mod library;
pub mod player;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use library::{AlbumCover, LibraryIndex, LibrarySnapshot, Track};
use notify::event::ModifyKind;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use player::{Player, PlayerSnapshot, QueueEntry, RepeatMode};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State, WindowEvent};

const STATE_FILE: &str = "shell.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellState {
    always_on_top: bool,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            always_on_top: false,
        }
    }
}

fn state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join(STATE_FILE))
}

fn read_state(app: &tauri::AppHandle) -> ShellState {
    let Ok(path) = state_path(app) else {
        return ShellState::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return ShellState::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_state(app: &tauri::AppHandle, state: &ShellState) -> Result<(), String> {
    let path = state_path(app)?;
    let text = serde_json::to_string(state).map_err(|err| err.to_string())?;
    fs::write(path, text).map_err(|err| err.to_string())
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn set_always_on_top(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window
        .set_always_on_top(on)
        .map_err(|err| err.to_string())?;
    write_state(window.app_handle(), &ShellState { always_on_top: on })
}

#[tauri::command]
fn always_on_top(app: tauri::AppHandle) -> bool {
    read_state(&app).always_on_top
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn set_fullscreen(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_fullscreen(on).map_err(|err| err.to_string())
}

#[tauri::command]
fn library_snapshot(index: State<'_, Arc<LibraryIndex>>) -> Result<LibrarySnapshot, String> {
    index.snapshot().map_err(|err| err.to_string())
}

#[tauri::command]
fn watched_folders(index: State<'_, Arc<LibraryIndex>>) -> Result<Vec<PathBuf>, String> {
    index.watched_folders().map_err(|err| err.to_string())
}

fn scan_library_in_background(index: Arc<LibraryIndex>, app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let progress = app.clone();
        let result = index.scan_notifying(&|| {
            let _ = progress.emit("library-changed", ());
        });
        if let Err(error) = result {
            eprintln!("library scan failed: {error}");
        }
        let _ = app.emit("library-changed", ());
    });
}

enum LibraryWatchMessage {
    Watch(PathBuf),
    Unwatch(PathBuf),
    FilesChanged,
}

type LibraryWatch = std::sync::mpsc::Sender<LibraryWatchMessage>;

fn is_library_change(kind: EventKind) -> bool {
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        // Metadata includes atime updates from reading a file. Those must not
        // schedule another scan. Content edits and renames are Modify variants
        // other than Metadata, so they still do.
        EventKind::Modify(modify) => !matches!(modify, ModifyKind::Metadata(_)),
        _ => false,
    }
}

/// The watcher thread outlives the window, so hiding to the tray does not stop it.
/// Tests never start it; they call `LibraryIndex::scan` directly.
fn start_library_watcher(index: Arc<LibraryIndex>, app: tauri::AppHandle) -> LibraryWatch {
    let (sender, receiver) = std::sync::mpsc::channel::<LibraryWatchMessage>();
    let handle = sender.clone();
    std::thread::spawn(move || {
        let events = sender;
        let mut watcher = match RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| match event {
                Ok(event) if is_library_change(event.kind) => {
                    let _ = events.send(LibraryWatchMessage::FilesChanged);
                }
                Ok(_) => {}
                Err(error) => eprintln!("library file watcher: {error}"),
            },
            // A symlink to a directory must not be followed into a cycle.
            Config::default().with_follow_symlinks(false),
        ) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("library file watcher unavailable: {error}");
                None
            }
        };
        if let Some(watcher) = watcher.as_mut() {
            if let Ok(folders) = index.watched_folders() {
                for folder in folders {
                    let _ = watcher.watch(&folder, RecursiveMode::Recursive);
                }
            }
        }
        while let Ok(message) = receiver.recv() {
            match message {
                LibraryWatchMessage::Watch(folder) => {
                    if let Some(watcher) = watcher.as_mut() {
                        let _ = watcher.watch(&folder, RecursiveMode::Recursive);
                    }
                }
                LibraryWatchMessage::Unwatch(folder) => {
                    if let Some(watcher) = watcher.as_mut() {
                        let _ = watcher.unwatch(&folder);
                    }
                }
                LibraryWatchMessage::FilesChanged => {
                    // Wait out the burst, including events this scan would otherwise retrigger.
                    let mut quiet = false;
                    while !quiet {
                        match receiver.recv_timeout(Duration::from_millis(400)) {
                            Ok(LibraryWatchMessage::Watch(folder)) => {
                                if let Some(watcher) = watcher.as_mut() {
                                    let _ = watcher.watch(&folder, RecursiveMode::Recursive);
                                }
                            }
                            Ok(LibraryWatchMessage::Unwatch(folder)) => {
                                if let Some(watcher) = watcher.as_mut() {
                                    let _ = watcher.unwatch(&folder);
                                }
                            }
                            Ok(LibraryWatchMessage::FilesChanged) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => quiet = true,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let progress = app.clone();
                    let result = index.scan_notifying(&|| {
                        let _ = progress.emit("library-changed", ());
                    });
                    if let Err(error) = result {
                        eprintln!("library scan failed: {error}");
                    }
                    let _ = app.emit("library-changed", ());
                }
            }
        }
    });
    handle
}

#[tauri::command]
fn add_watched_folder(
    index: State<'_, Arc<LibraryIndex>>,
    watch: State<'_, LibraryWatch>,
    app: tauri::AppHandle,
    folder: PathBuf,
) -> Result<bool, String> {
    let changed = index
        .add_watched_folder(&folder)
        .map_err(|err| err.to_string())?;
    if changed {
        let _ = app.emit("library-changed", ());
        if let Ok(canonical) = folder.canonicalize() {
            let _ = watch.send(LibraryWatchMessage::Watch(canonical));
        }
        scan_library_in_background(Arc::clone(index.inner()), app);
    }
    Ok(changed)
}

#[tauri::command]
fn remove_watched_folder(
    index: State<'_, Arc<LibraryIndex>>,
    watch: State<'_, LibraryWatch>,
    app: tauri::AppHandle,
    folder: PathBuf,
) -> Result<bool, String> {
    let changed = index
        .remove_watched_folder(&folder)
        .map_err(|err| err.to_string())?;
    if changed {
        let _ = app.emit("library-changed", ());
        let canonical = folder.canonicalize().unwrap_or(folder);
        let _ = watch.send(LibraryWatchMessage::Unwatch(canonical));
    }
    Ok(changed)
}

#[tauri::command]
fn album_tracks(
    index: State<'_, Arc<LibraryIndex>>,
    album_id: String,
) -> Result<Vec<Track>, String> {
    index
        .tracks_for_album(&album_id)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn artist_tracks(
    index: State<'_, Arc<LibraryIndex>>,
    artist: Option<String>,
) -> Result<Vec<Track>, String> {
    index
        .tracks_for_artist(artist.as_deref())
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn genre_tracks(
    index: State<'_, Arc<LibraryIndex>>,
    genre: Option<String>,
) -> Result<Vec<Track>, String> {
    index
        .tracks_for_genre(genre.as_deref())
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn year_tracks(
    index: State<'_, Arc<LibraryIndex>>,
    year: Option<u32>,
) -> Result<Vec<Track>, String> {
    index.tracks_for_year(year).map_err(|err| err.to_string())
}

#[tauri::command]
fn search_tracks(index: State<'_, Arc<LibraryIndex>>, query: String) -> Result<Vec<Track>, String> {
    index.search(&query).map_err(|err| err.to_string())
}

#[tauri::command]
fn album_cover(
    index: State<'_, Arc<LibraryIndex>>,
    album_id: String,
) -> Result<Option<AlbumCover>, String> {
    index.album_cover(&album_id).map_err(|err| err.to_string())
}

#[tauri::command]
fn player_snapshot(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.snapshot()
}

#[tauri::command]
fn play_tracks(
    player: State<'_, Arc<Player>>,
    tracks: Vec<QueueEntry>,
    start_path: PathBuf,
) -> Result<PlayerSnapshot, String> {
    player
        .play_entries(tracks, &start_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn player_play(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.play()
}

#[tauri::command]
fn player_pause(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.pause()
}

#[tauri::command]
fn player_toggle(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.toggle()
}

#[tauri::command]
fn player_next(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.next()
}

#[tauri::command]
fn player_previous(player: State<'_, Arc<Player>>) -> PlayerSnapshot {
    player.previous()
}

#[tauri::command]
fn player_set_shuffle(
    player: State<'_, Arc<Player>>,
    on: bool,
) -> Result<PlayerSnapshot, String> {
    Ok(player.set_shuffle(on))
}

#[tauri::command]
fn player_set_repeat(
    player: State<'_, Arc<Player>>,
    mode: RepeatMode,
) -> Result<PlayerSnapshot, String> {
    Ok(player.set_repeat(mode))
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .invoke_handler(tauri::generate_handler![
            set_always_on_top,
            always_on_top,
            quit,
            set_fullscreen,
            library_snapshot,
            watched_folders,
            add_watched_folder,
            remove_watched_folder,
            album_tracks,
            artist_tracks,
            genre_tracks,
            year_tracks,
            search_tracks,
            album_cover,
            player_snapshot,
            play_tracks,
            player_play,
            player_pause,
            player_toggle,
            player_next,
            player_previous,
            player_set_shuffle,
            player_set_repeat
        ])
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            fs::create_dir_all(&data_directory)?;
            let cache_directory = app.path().app_cache_dir()?.join("covers");
            let default_music =
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Music"));
            let library = Arc::new(
                LibraryIndex::open(
                    data_directory.join("library.db"),
                    cache_directory,
                    default_music.as_deref(),
                )
                .map_err(|err| {
                    std::io::Error::other(format!("failed to open library index: {err}"))
                })?,
            );
            app.manage(Arc::clone(&library));

            let player = Arc::new(
                Player::open(
                    data_directory.join("library.db"),
                    player::system_output_sink(),
                )
                .map_err(|error| {
                    std::io::Error::other(format!("failed to open player: {error}"))
                })?,
            );
            let player_events = app.handle().clone();
            player.set_event_handler(move |snapshot| {
                let _ = player_events.emit("player-changed", snapshot);
            });
            app.manage(Arc::clone(&player));

            let watch = start_library_watcher(Arc::clone(&library), app.handle().clone());
            app.manage(watch);
            scan_library_in_background(Arc::clone(&library), app.handle().clone());

            let state = read_state(app.handle());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(state.always_on_top);
            }

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let play = MenuItem::with_id(app, "play", "Play", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
            let next = MenuItem::with_id(app, "next", "Next", true, None::<&str>)?;
            let previous = MenuItem::with_id(app, "previous", "Previous", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu =
                Menu::with_items(app, &[&show, &play, &pause, &next, &previous, &quit_item])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Linkamp")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    "play" | "pause" | "next" | "previous" => {
                        if let Some(player) = app.try_state::<Arc<Player>>() {
                            match event.id.as_ref() {
                                "play" => {
                                    let _ = player.play();
                                }
                                "pause" => {
                                    player.pause();
                                }
                                "next" => {
                                    let _ = player.next();
                                }
                                "previous" => {
                                    let _ = player.previous();
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Linkamp");

    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            // Hide-to-tray asks the process to exit without a code. Quit sets one.
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
