//! MPRIS service. Media keys, the XFCE volume plugin, and desktop shells
//! reach the player through the session bus, so no key grabbing of our own
//! exists. The bus is served on a dedicated thread that polls the player and
//! pushes property changes; nothing here is exercised by tests, which drive
//! the player directly through the fake sink.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use mpris_server::{Metadata, PlaybackStatus, Player as BusPlayer, Time, TrackId};
use tauri::AppHandle;

use crate::player::{Player, PlayerSnapshot};

/// The cadence at which the bus mirrors the player. Clients poll Position
/// themselves; a quarter second keeps every reported property fresh.
const UPDATE_INTERVAL: Duration = Duration::from_millis(250);

/// A forward jump larger than this between two updates cannot come from
/// playback at 1x rate, so it is reported as a seek. Rewinds always are.
const SEEK_SLACK_MS: u64 = 1_000;

/// Starts serving MPRIS in the background. A missing session bus or any
/// other failure only disables the bus; playback and the window go on.
pub fn serve(app: AppHandle, player: Arc<Player>) {
    let thread = std::thread::Builder::new()
        .name("mpris".to_owned())
        .spawn(move || run(app, player));
    if let Err(error) = thread {
        eprintln!("could not start the MPRIS service: {error}");
    }
}

fn run(app: AppHandle, player: Arc<Player>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("MPRIS runtime unavailable: {error}");
            return;
        }
    };
    let tasks = tokio::task::LocalSet::new();
    tasks.block_on(&runtime, async move {
        if let Err(error) = serve_on_bus(&app, &player).await {
            eprintln!("MPRIS unavailable: {error}");
        }
    });
}

async fn serve_on_bus(app: &AppHandle, player: &Arc<Player>) -> mpris_server::zbus::Result<()> {
    let bus = BusPlayer::builder("linkamp")
        .identity("Linkamp")
        .desktop_entry("linkamp")
        .can_quit(true)
        .can_raise(true)
        .can_play(true)
        .can_pause(true)
        .can_seek(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_control(true)
        .supported_uri_schemes(["file"])
        .supported_mime_types(["audio/mpeg", "audio/flac", "audio/x-flac"])
        .build()
        .await?;

    // The command side. Every call reaches the player the same way the
    // window's do, so state changes travel back to the window unchanged.
    {
        let app = app.clone();
        bus.connect_raise(move |_| crate::show_main(&app));
    }
    {
        let app = app.clone();
        bus.connect_quit(move |_| app.exit(0));
    }
    {
        let player = Arc::clone(player);
        bus.connect_play(move |_| {
            player.play();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_pause(move |_| {
            player.pause();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_play_pause(move |_| {
            player.toggle();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_stop(move |_| {
            player.stop();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_next(move |_| {
            player.next();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_previous(move |_| {
            player.previous();
        });
    }
    {
        let player = Arc::clone(player);
        bus.connect_seek(move |_, offset| {
            player.seek(offset.as_micros() / 1_000);
        });
    }
    // The XFCE volume plugin drives Linkamp's own app volume through this,
    // the same multiplier the window's slider sets. The system volume is
    // never touched. A negative volume clamps to zero inside the player, as
    // the MPRIS rules require.
    {
        let player = Arc::clone(player);
        bus.connect_set_volume(move |_, volume| {
            player.set_volume(volume as f32);
        });
    }
    // SetPosition carries the track id the bus last advertised; a stale id
    // from an earlier track is ignored, as the MPRIS rules require.
    let advertised = Rc::new(RefCell::new(TrackId::NO_TRACK));
    {
        let player = Arc::clone(player);
        let advertised = Rc::clone(&advertised);
        bus.connect_set_position(move |_, track_id, position| {
            if *track_id == *advertised.borrow() && position.as_micros() >= 0 {
                player.seek_to_ms(position.as_micros() as u64 / 1_000);
            }
        });
    }

    tokio::task::spawn_local(bus.run());

    let mut reported = ReportedState::default();
    let mut updates = tokio::time::interval(UPDATE_INTERVAL);
    loop {
        updates.tick().await;
        let snapshot = player.snapshot();
        reported.apply(&bus, &advertised, &snapshot).await?;
    }
}

/// What the bus already says, so only real changes are pushed.
#[derive(Default)]
struct ReportedState {
    status: Option<PlaybackStatus>,
    track: Option<String>,
    position_ms: u64,
    volume: Option<f64>,
}

impl ReportedState {
    async fn apply(
        &mut self,
        bus: &BusPlayer,
        advertised: &RefCell<TrackId>,
        snapshot: &PlayerSnapshot,
    ) -> mpris_server::zbus::Result<()> {
        let status = match (snapshot.playing, snapshot.current_index) {
            (true, Some(_)) => PlaybackStatus::Playing,
            (false, Some(_)) => PlaybackStatus::Paused,
            (_, None) => PlaybackStatus::Stopped,
        };
        if self.status != Some(status) {
            bus.set_playback_status(status).await?;
            self.status = Some(status);
        }

        // The Can* properties describe the queue, not the momentary state.
        let has_queue = !snapshot.queue.is_empty();
        let has_more = snapshot.queue.len() > 1;
        if bus.can_play() != has_queue {
            bus.set_can_play(has_queue).await?;
        }
        if bus.can_pause() != has_queue {
            bus.set_can_pause(has_queue).await?;
        }
        if bus.can_go_next() != has_more {
            bus.set_can_go_next(has_more).await?;
        }
        if bus.can_go_previous() != has_queue {
            bus.set_can_go_previous(has_queue).await?;
        }

        let track = snapshot
            .current_index
            .map(|index| format!("{}#{}", snapshot.queue[index].path.display(), index));
        if track != self.track {
            let metadata = metadata_for(snapshot);
            *advertised.borrow_mut() = metadata.trackid().unwrap_or(TrackId::NO_TRACK);
            bus.set_metadata(metadata).await?;
            self.track = track;
            self.position_ms = 0;
        }

        let position_ms = snapshot.position_ms;
        let same_track = self.track.is_some();
        if same_track
            && (position_ms < self.position_ms || position_ms > self.position_ms + SEEK_SLACK_MS)
        {
            // A rewind, or a jump no 1x playback could make, is a seek.
            bus.seeked(Time::from_millis(position_ms as i64)).await?;
        }
        self.position_ms = position_ms;
        // Position is polled, never announced as a property change.
        bus.set_position(Time::from_millis(position_ms as i64));

        // The bus mirrors the app volume, so the desktop's volume control
        // shows and changes the same multiplier the window's slider does.
        let volume = f64::from(snapshot.volume);
        if self.volume != Some(volume) {
            bus.set_volume(volume).await?;
            self.volume = Some(volume);
        }
        Ok(())
    }
}

fn metadata_for(snapshot: &PlayerSnapshot) -> Metadata {
    let Some(index) = snapshot.current_index else {
        return Metadata::builder().trackid(TrackId::NO_TRACK).build();
    };
    let Some(entry) = snapshot.queue.get(index) else {
        return Metadata::builder().trackid(TrackId::NO_TRACK).build();
    };
    let trackid =
        TrackId::try_from(format!("/dev/linkamp/track/{index}")).unwrap_or(TrackId::NO_TRACK);
    Metadata::builder()
        .trackid(trackid)
        .title(entry.title.clone())
        .artist(entry.artist.clone())
        .album(entry.album.clone())
        .length(Time::from_millis(entry.duration_ms as i64))
        .build()
}
