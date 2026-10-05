use std::fs;
use std::path::{Path, PathBuf};

use linkamp_lib::library::{LibraryIndex, Track};
use linkamp_lib::player::{FakeOutputSink, NoDeviceOutputSink, Player, QueueEntry};
use std::time::Duration;
use tempfile::TempDir;

fn fixture(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("silence.{extension}"))
}

fn audio_file(directory: &Path, name: &str, extension: &str) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join(format!("{name}.{extension}"));
    fs::copy(fixture(extension), &path).unwrap();
    path
}

fn entry(path: PathBuf, title: &str) -> QueueEntry {
    QueueEntry {
        path,
        title: title.to_owned(),
        artist: Some("Test Artist".to_owned()),
        album: "Test Album".to_owned(),
        duration_ms: 150,
    }
}

/// An entry whose tagged duration runs far past the fixture's real audio, so
/// a seek clamped to the tag still lands beyond the end of the stream.
fn long_entry(path: PathBuf, title: &str) -> QueueEntry {
    QueueEntry {
        duration_ms: 5_000,
        ..entry(path, title)
    }
}

fn track(path: PathBuf, title: &str) -> Track {
    Track {
        path,
        title: title.to_owned(),
        artist: Some("Test Artist".to_owned()),
        album: "Test Album".to_owned(),
        genre: None,
        year: None,
        track_number: None,
        duration_ms: 150,
    }
}

#[test]
fn selected_track_replaces_the_queue_with_the_visible_remainder() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let two = audio_file(&album, "two", "flac");
    let three = audio_file(&album, "three", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();

    let snapshot = player
        .play_tracks(
            vec![
                track(one, "One"),
                track(two.clone(), "Two"),
                track(three, "Three"),
            ],
            &two,
        )
        .unwrap();

    assert!(snapshot.playing);
    assert_eq!(snapshot.current_index, Some(0));
    assert_eq!(
        snapshot
            .queue
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>(),
        ["Two", "Three"]
    );
    assert_eq!(
        sink.pulls(),
        0,
        "the fake sink advances only when the test asks it to"
    );
}

#[test]
fn a_second_activation_replaces_a_live_queue() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let two = audio_file(&album, "two", "flac");
    let replacement = audio_file(&album, "replacement", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink).unwrap();
    player
        .play_entries(vec![entry(one.clone(), "One"), entry(two, "Two")], &one)
        .unwrap();

    let snapshot = player
        .play_entries(
            vec![entry(replacement.clone(), "Replacement")],
            &replacement,
        )
        .unwrap();

    assert_eq!(snapshot.queue.len(), 1);
    assert_eq!(snapshot.queue[0].title, "Replacement");
    assert_eq!(snapshot.current_index, Some(0));
    assert!(snapshot.playing);
}

#[test]
fn fake_sink_drives_mp3_to_flac_advancement_and_pause() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let mp3 = audio_file(&album, "first", "mp3");
    let flac = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(mp3.clone(), "MP3"), entry(flac, "FLAC")], &mp3)
        .unwrap();

    let first_audio = sink.pull(1_000);
    assert_eq!(first_audio.len(), 2_000);
    assert_eq!(
        player.snapshot().current_index,
        Some(0),
        "the MP3 decoder supplied frames"
    );

    player.pause();
    assert!(sink.pull(2_000).iter().all(|sample| *sample == 0.0));
    assert_eq!(
        player.snapshot().current_index,
        Some(0),
        "pause does not consume decoded audio"
    );
    player.play();

    for _ in 0..20 {
        sink.pull(500);
        if player.snapshot().current_index == Some(1) {
            break;
        }
    }
    let on_flac = player.snapshot();
    assert_eq!(on_flac.current_index, Some(1));
    assert!(
        on_flac.playing,
        "the FLAC starts without stopping the live queue"
    );
}

#[test]
fn next_previous_and_queue_head_previous_are_listener_visible() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let two = audio_file(&album, "two", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink).unwrap();
    player
        .play_entries(vec![entry(one.clone(), "One"), entry(two, "Two")], &one)
        .unwrap();

    assert_eq!(player.next().current_index, Some(1));
    assert_eq!(player.previous().current_index, Some(0));
    let head = player.previous();
    assert_eq!(head.current_index, Some(0));
    assert!(head.playing);
}

#[test]
fn player_commands_remain_usable_without_an_audio_device() {
    let temp = TempDir::new().unwrap();
    let path = fixture("mp3");
    let player = Player::open(
        temp.path().join("library.db"),
        NoDeviceOutputSink {
            sample_rate: 48_000,
            channels: 2,
        },
    )
    .unwrap();

    assert!(
        player
            .play_entries(vec![entry(path.clone(), "Offline output")], &path)
            .unwrap()
            .playing
    );
    assert!(!player.pause().playing);
    assert!(player.play().playing);
    assert_eq!(player.previous().current_index, Some(0));
}

#[test]
fn a_library_rescan_does_not_rebuild_or_stop_the_live_queue() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(&music.join("Album"), "one", "mp3");
    audio_file(&music.join("Album"), "two", "flac");
    let database = temp.path().join("library.db");
    let library = LibraryIndex::open(&database, temp.path().join("covers"), Some(&music)).unwrap();
    library.scan().unwrap();
    let album_id = library.snapshot().unwrap().albums[0].id.clone();
    let rows = library.tracks_for_album(&album_id).unwrap();
    let first = rows[0].path.clone();
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(&database, sink.clone()).unwrap();
    player.play_tracks(rows, &first).unwrap();
    sink.pull(500);
    let before = player.snapshot();

    library.scan().unwrap();

    let after = player.snapshot();
    assert_eq!(after.queue, before.queue);
    assert_eq!(after.current_index, before.current_index);
    assert!(after.playing);
}

/// Elapsed milliseconds the player reports after `frames` frames at 44.1 kHz,
/// the same arithmetic the player uses.
fn elapsed_ms(frames: u64) -> u64 {
    frames * 1_000 / 44_100
}

#[test]
fn position_advances_with_playback_and_a_pause_freezes_it() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();

    sink.pull(1_000);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(1_000));

    player.pause();
    sink.pull(2_000);
    assert_eq!(
        player.snapshot().position_ms,
        elapsed_ms(1_000),
        "silence written during a pause does not move the position"
    );

    player.play();
    sink.pull(500);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(1_500));
}

#[test]
fn a_forward_seek_continues_the_same_track_from_the_new_position() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();
    sink.pull(500);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(500));

    // 11 ms of playback plus a 49 ms offset lands at 60 ms, inside the entry.
    let snapshot = player.seek(49);

    assert_eq!(snapshot.current_index, Some(0));
    assert!(snapshot.playing);
    // 60 ms is exactly 2,646 frames at 44.1 kHz, so the read-back is exact.
    assert_eq!(snapshot.position_ms, 60);
    sink.pull(354);
    let resumed = player.snapshot();
    assert_eq!(resumed.current_index, Some(0));
    assert!(resumed.playing);
    assert_eq!(resumed.position_ms, elapsed_ms(2_646 + 354));
}

#[test]
fn a_rewind_seek_clamps_to_the_start_of_the_track() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();
    sink.pull(2_000);
    assert!(player.snapshot().position_ms > 0);

    let snapshot = player.seek(-600_000);

    assert_eq!(snapshot.current_index, Some(0));
    assert!(snapshot.playing);
    assert_eq!(snapshot.position_ms, 0);
}

#[test]
fn a_seek_while_paused_moves_where_playback_resumes() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();
    sink.pull(500);
    player.pause();

    let snapshot = player.seek_to_ms(50);

    assert_eq!(snapshot.position_ms, 50);
    sink.pull(1_000);
    assert_eq!(
        player.snapshot().position_ms,
        50,
        "a paused player holds the sought position"
    );
    player.play();
    sink.pull(100);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(2_205 + 100));
}

#[test]
fn a_seek_past_the_end_lands_on_the_next_track() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let two = audio_file(&album, "two", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![long_entry(one, "One"), long_entry(two, "Two")],
            &album.join("one.mp3"),
        )
        .unwrap();
    sink.pull(100);
    assert_eq!(player.snapshot().current_index, Some(0));

    let snapshot = player.seek(3_600_000);

    assert_eq!(snapshot.current_index, Some(1));
    assert!(
        snapshot.playing,
        "a seek past the end acts like reaching the end of the track"
    );
    assert_eq!(snapshot.position_ms, 0);
    sink.pull(200);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(200));
}

#[test]
fn stop_resets_the_position_and_play_restarts_the_same_entry() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();
    sink.pull(1_000);
    assert!(player.snapshot().position_ms > 0);

    let stopped = player.stop();

    assert!(!stopped.playing);
    assert_eq!(stopped.current_index, Some(0));
    assert_eq!(stopped.queue.len(), 1);
    assert_eq!(stopped.position_ms, 0);
    sink.pull(500);
    assert_eq!(
        player.snapshot().position_ms,
        0,
        "a stopped player does not advance"
    );

    let resumed = player.play();
    assert!(resumed.playing);
    sink.pull(1_000);
    assert_eq!(player.snapshot().position_ms, elapsed_ms(1_000));
}

#[test]
fn seeks_and_stops_reach_the_window_like_other_state_changes() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink).unwrap();
    let (events, receive) = std::sync::mpsc::channel();
    player.set_event_handler(move |snapshot| {
        let _ = events.send(snapshot);
    });
    player
        .play_entries(vec![entry(one, "One")], &album.join("one.mp3"))
        .unwrap();
    let started = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("starting playback reaches the window");
    assert!(started.playing);

    player.seek_to_ms(60);
    let sought = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("a seek reports the new position to the window");
    assert_eq!(sought.position_ms, 60);
    assert!(sought.playing);

    player.stop();
    let stopped = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("a stop reports the new state to the window");
    assert!(!stopped.playing);
    assert_eq!(stopped.position_ms, 0);
}
