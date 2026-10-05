use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::TimeBase;

use crate::library::Track;

/// The callback used by an output sink to ask the player for interleaved samples.
pub type RenderCallback = Box<dyn FnMut(&mut [f32]) + Send>;

/// An opened output stream. Keeping this value alive keeps the stream open.
pub trait RunningOutput: Send + Sync {}

/// The only audio-output seam in the player. The application supplies CPAL and
/// tests supply `FakeOutputSink`, so tests never open an audio device.
pub trait OutputSink: Send + 'static {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    fn start(self: Box<Self>, render: RenderCallback) -> Result<Box<dyn RunningOutput>>;
}

#[derive(Debug)]
pub struct PlayerError(String);

impl fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlayerError {}

impl From<rusqlite::Error> for PlayerError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, PlayerError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub missing: bool,
}

impl From<Track> for QueueEntry {
    fn from(track: Track) -> Self {
        Self {
            path: track.path,
            title: track.title,
            artist: track.artist,
            album: track.album,
            duration_ms: track.duration_ms,
            missing: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl Default for RepeatMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub queue: Vec<QueueEntry>,
    pub current_index: Option<usize>,
    pub playing: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Elapsed time inside the current entry. It advances only while samples
    /// reach the sink, so a pause freezes it, and it restarts with each track.
    pub position_ms: u64,
    /// The app volume as a linear multiplier, 0.0 to 1.0. It is applied to
    /// the samples of Linkamp's own stream and never touches the system
    /// volume; zero writes silence while the stream stays open.
    pub volume: f32,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            current_index: None,
            playing: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            position_ms: 0,
            volume: DEFAULT_VOLUME,
        }
    }
}

/// The volume a fresh player starts with, before any persisted state exists.
const DEFAULT_VOLUME: f32 = 0.8;

/// How often plain playback (no state change) emits a position snapshot to the
/// window: a few times per second, never per buffer.
const POSITION_EVENT_INTERVAL_MS: u64 = 250;

/// Seconds of the next queue entry kept decoded and converted to the stream
/// format, so a track transition never waits on the disk or a decoder. Bounded
/// to a few seconds of audio, never whole files, so memory stays flat no
/// matter how long the queue is.
const DECODE_AHEAD_SECONDS: usize = 3;

struct Core {
    state: PlayerSnapshot,
    current: Option<PreparedTrack>,
    upcoming: Option<PreparedTrack>,
    /// The queue in its original, unshuffled order. Turning shuffle off
    /// restores the playback queue from this snapshot.
    original_queue: Vec<QueueEntry>,
    /// Number of output frames rendered for the current entry since it was
    /// opened. Used by the Previous restart rule.
    frames_in_current: u64,
    /// Above this many frames, Previous restarts the current entry instead of
    /// jumping back. Defaults to a few seconds at the output rate.
    previous_restart_threshold_frames: u64,
    shuffle_rng: ShuffleRng,
    sample_rate: u32,
    channels: usize,
    connection: Connection,
}

impl Core {
    fn snapshot(&self) -> PlayerSnapshot {
        let mut snapshot = self.state.clone();
        for entry in &mut snapshot.queue {
            entry.missing = !entry.path.is_file();
        }
        snapshot.position_ms = self.frames_to_ms(self.frames_in_current);
        snapshot
    }

    fn frames_to_ms(&self, frames: u64) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        frames * 1000 / u64::from(self.sample_rate)
    }

    fn ms_to_frames(&self, ms: u64) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        ms.saturating_mul(u64::from(self.sample_rate)) / 1000
    }

    fn open_current(&mut self) -> bool {
        let Some(index) = self.state.current_index else {
            return false;
        };
        let Some(entry) = self.state.queue.get(index) else {
            return false;
        };
        // A position restored from disk must seek the freshly opened track
        // before its first frame reaches the sink; otherwise the listener
        // would hear the start of the entry before landing on the saved spot.
        let start_frame = self.frames_in_current;
        match PreparedTrack::open(&entry.path, index, self.sample_rate, self.channels) {
            Ok(mut track) => {
                if start_frame > 0 && !track.seek_to(start_frame) {
                    eprintln!(
                        "could not seek {} to frame {start_frame}",
                        entry.path.display()
                    );
                    self.current = None;
                    return false;
                }
                self.current = Some(track);
                self.prepare_upcoming();
                true
            }
            Err(error) => {
                eprintln!("could not decode {}: {error}", entry.path.display());
                self.current = None;
                false
            }
        }
    }

    /// Opens the entry after the current one ahead of time, so crossing the
    /// track boundary costs nothing at the boundary itself.
    fn prepare_upcoming(&mut self) {
        let Some(index) = self.state.current_index else {
            self.upcoming = None;
            return;
        };
        let upcoming_index = index + 1;
        if self
            .upcoming
            .as_ref()
            .is_some_and(|track| track.queue_index == upcoming_index)
        {
            return;
        }
        self.upcoming = None;
        let Some(entry) = self.state.queue.get(upcoming_index) else {
            return;
        };
        match PreparedTrack::open(&entry.path, upcoming_index, self.sample_rate, self.channels) {
            Ok(track) => self.upcoming = Some(track),
            // A failed prepare is retried when the boundary is reached.
            Err(error) => eprintln!("could not prepare {}: {error}", entry.path.display()),
        }
    }

    /// Moves to the next queue entry. An entry prepared by decode-ahead
    /// becomes the current one without opening anything, and entries that
    /// cannot be opened are skipped. Returns false at the end of the queue,
    /// where playback stops and the queue stays in place.
    fn advance(&mut self) -> bool {
        if self.state.repeat == RepeatMode::One && self.state.current_index.is_some() {
            // Replay the current entry from the top.
            self.current = None;
            self.upcoming = None;
            self.frames_in_current = 0;
            return self.open_current();
        }
        let mut next = self.state.current_index.map_or(0, |index| index + 1);
        let mut wrapped = false;
        loop {
            if next >= self.state.queue.len() {
                if self.state.repeat == RepeatMode::All
                    && !self.state.queue.is_empty()
                    && !wrapped
                {
                    // Wrap to the top of the queue, once.
                    next = 0;
                    wrapped = true;
                    continue;
                }
                // Repeat Off at the end, or every remaining entry is missing:
                // stop with the queue in place. A None current index is the
                // stopped sentinel; Play starts from the top.
                self.state.playing = false;
                self.state.current_index = None;
                self.current = None;
                self.upcoming = None;
                self.frames_in_current = 0;
                return false;
            }
            let prepared = self
                .upcoming
                .take()
                .filter(|track| track.queue_index == next);
            self.state.current_index = Some(next);
            self.current = prepared;
            self.frames_in_current = 0;
            if self.current.is_some() || self.open_current() {
                self.prepare_upcoming();
                return true;
            }
            // A missing or unreadable entry is skipped.
            next += 1;
        }
    }

    fn previous(&mut self) {
        if self.state.queue.is_empty() {
            self.state.current_index = None;
            self.state.playing = false;
            self.current = None;
            self.upcoming = None;
            self.frames_in_current = 0;
            return;
        }
        // A stopped queue (current_index is None) jumps back to the last entry
        // so the listener lands on what just played.
        let current = self
            .state
            .current_index
            .unwrap_or(self.state.queue.len() - 1);
        if current == 0 || self.frames_in_current > self.previous_restart_threshold_frames {
            // At the head, or further than a few seconds in: restart the
            // current entry.
            self.state.current_index = Some(current);
            self.current = None;
            self.upcoming = None;
            self.frames_in_current = 0;
            if self.state.playing {
                self.open_current();
            }
            return;
        }
        self.state.current_index = Some(current - 1);
        self.current = None;
        self.upcoming = None;
        self.frames_in_current = 0;
        if self.state.playing {
            self.open_current();
        }
    }

    /// Positions playback inside the current entry. A position at or past the
    /// entry's tagged duration, or one the file cannot supply, behaves like
    /// reaching the end of the track: playback continues with the next entry.
    fn seek(&mut self, position_ms: u64) {
        let Some(index) = self.state.current_index else {
            return;
        };
        let Some(entry) = self.state.queue.get(index) else {
            return;
        };
        if position_ms >= entry.duration_ms {
            let was_playing = self.state.playing;
            self.advance();
            self.state.playing = was_playing && self.state.current_index.is_some();
            return;
        }
        if self.current.is_none() && !self.open_current() {
            let was_playing = self.state.playing;
            self.advance();
            self.state.playing = was_playing && self.state.current_index.is_some();
            return;
        }
        let target = self.ms_to_frames(position_ms);
        let reached = self
            .current
            .as_mut()
            .is_some_and(|track| track.seek_to(target));
        if reached {
            self.frames_in_current = target;
        } else {
            let was_playing = self.state.playing;
            self.advance();
            self.state.playing = was_playing && self.state.current_index.is_some();
        }
    }

    fn render(&mut self, output: &mut [f32]) -> bool {
        // Pause and an empty queue both feed silence into the open stream.
        output.fill(0.0);
        if !self.state.playing || self.state.queue.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut cursor = 0;
        while cursor + self.channels <= output.len() && self.state.playing {
            if self.current.is_none() && !self.open_current() {
                // An entry that will not decode is skipped.
                changed = true;
                if !self.advance() {
                    break;
                }
                continue;
            }
            if self
                .current
                .as_ref()
                .is_some_and(PreparedTrack::drained)
            {
                // The current index moves only when the next entry's first
                // frame is written, so the boundary is sample-exact and the
                // stream is never closed, paused, or reopened in between.
                self.current = None;
                changed = true;
                if !self.advance() {
                    break;
                }
                continue;
            }

            let wrote = self
                .current
                .as_mut()
                .map_or(0, |track| track.write_frames(&mut output[cursor..]));
            self.frames_in_current = self
                .frames_in_current
                .saturating_add((wrote / self.channels.max(1)) as u64);
            cursor += wrote;
        }

        // The app volume is a per-sample multiplier on the mixed stream, on
        // top of whatever PipeWire and the system volume do with it. Zero
        // writes silence into the still-open stream, so unmuting is instant.
        if self.state.volume != 1.0 {
            for sample in output.iter_mut() {
                *sample *= self.state.volume;
            }
        }

        // Top up decode-ahead while the current track plays, so the next entry
        // is decoded before it is needed. Paused rendering returns above and
        // leaves both decoders untouched, which makes resume sample-exact.
        if self.state.playing {
            let ahead_frames = self.sample_rate as usize * DECODE_AHEAD_SECONDS;
            if let Some(track) = self.upcoming.as_mut() {
                track.fill(ahead_frames);
            }
        }
        changed
    }

    /// Saves the original (unshuffled) queue. Shuffle is a mode over the
    /// queue, not a rewrite, so the unshuffled order is the one that survives
    /// a restart; the shuffled order is reconstructed on load when needed.
    fn save_queue(&mut self) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM player_queue", [])?;
        for (position, entry) in self.original_queue.iter().enumerate() {
            transaction.execute(
                "INSERT INTO player_queue(position, path, title, artist, album, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    position as i64,
                    entry.path.to_string_lossy(),
                    entry.title,
                    entry.artist,
                    entry.album,
                    i64::try_from(entry.duration_ms).unwrap_or(i64::MAX),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Writes the current entry, position, volume, shuffle, and repeat into
    /// the single-row `player_state` table. The render callback calls this
    /// each time the periodic position event fires, so a hard exit loses at
    /// most a small chunk of the buffer; the command-side setters call it on
    /// every change so volume, shuffle, and repeat are durable at once.
    fn save_state(&mut self) -> Result<()> {
        let current_index = self
            .state
            .current_index
            .and_then(|index| i64::try_from(index).ok());
        let frames = i64::try_from(self.frames_in_current).unwrap_or(i64::MAX);
        let repeat = match self.state.repeat {
            RepeatMode::Off => 0_i64,
            RepeatMode::All => 1_i64,
            RepeatMode::One => 2_i64,
        };
        self.connection.execute(
            "INSERT INTO player_state(id, current_index, frames_in_current, volume, shuffle, repeat)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 current_index = excluded.current_index,
                 frames_in_current = excluded.frames_in_current,
                 volume = excluded.volume,
                 shuffle = excluded.shuffle,
                 repeat = excluded.repeat",
            params![
                current_index,
                frames,
                f64::from(self.state.volume),
                i64::from(self.state.shuffle),
                repeat,
            ],
        )?;
        Ok(())
    }

    /// Loads the queue from `player_queue` and the per-player settings from
    /// `player_state` into `self`. The queue comes back in declared position
    /// order with all its snapshot fields intact; settings that the previous
    /// session did not write keep their defaults. A launch always starts
    /// paused, so a silent restart can never surprise the listener with sound.
    fn load_state(&mut self) -> Result<()> {
        let queue: Vec<QueueEntry> = {
            let mut statement = self.connection.prepare(
                "SELECT path, title, artist, album, duration_ms FROM player_queue ORDER BY position",
            )?;
            statement
                .query_map([], |row| {
                    Ok(QueueEntry {
                        path: PathBuf::from(row.get::<_, String>(0)?),
                        title: row.get(1)?,
                        artist: row.get(2)?,
                        album: row.get(3)?,
                        duration_ms: row.get::<_, i64>(4)? as u64,
                        missing: false,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        self.original_queue = queue.clone();
        let mut queue = queue;

        let state_row: Option<(Option<i64>, i64, f64, i64, i64)> = self
            .connection
            .query_row(
                "SELECT current_index, frames_in_current, volume, shuffle, repeat
                 FROM player_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;

        if let Some((current_index, frames_in_current, volume, shuffle, repeat)) = state_row {
            self.state.current_index = current_index
                .and_then(|value| usize::try_from(value).ok())
                .filter(|index| *index < queue.len());
            // Frames are the source of truth; storing them avoids the
            // millisecond-round-trip loss a millisecond column would have.
            self.frames_in_current = frames_in_current.max(0) as u64;
            self.state.volume = volume as f32;
            self.state.shuffle = shuffle != 0;
            self.state.repeat = match repeat {
                1 => RepeatMode::All,
                2 => RepeatMode::One,
                _ => RepeatMode::Off,
            };
            if self.state.shuffle {
                // Shuffle was on when the previous session ended: re-anchor
                // the current entry and reshuffle the entries after it, the
                // same way turning shuffle on does.
                let split_at = self.state.current_index.map(|index| index + 1).unwrap_or(0);
                if split_at < queue.len() {
                    let mut tail = queue.split_off(split_at);
                    shuffle_slice(&mut tail, &mut self.shuffle_rng);
                    queue.extend(tail);
                }
            }
        }

        self.state.queue = queue;
        // A launch never opens with the stream playing: the listener would
        // hear whatever the saved position landed on without warning.
        self.state.playing = false;
        Ok(())
    }
}

type EventHandler = Arc<dyn Fn(PlayerSnapshot) + Send + Sync>;

/// Playback boundary shared by direct tests and Tauri commands.
pub struct Player {
    core: Arc<Mutex<Core>>,
    event_handler: Arc<Mutex<Option<EventHandler>>>,
    _output: Box<dyn RunningOutput>,
}

impl Player {
    pub fn open(database_path: impl AsRef<Path>, sink: impl OutputSink) -> Result<Self> {
        if let Some(parent) = database_path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| PlayerError(error.to_string()))?;
        }
        let connection = Connection::open(database_path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS player_queue (
                 position INTEGER PRIMARY KEY,
                 path TEXT NOT NULL,
                 title TEXT NOT NULL,
                 artist TEXT,
                 album TEXT NOT NULL,
                 duration_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS player_state (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 current_index INTEGER,
                 frames_in_current INTEGER NOT NULL,
                 volume REAL NOT NULL,
                 shuffle INTEGER NOT NULL,
                 repeat INTEGER NOT NULL
             );",
        )?;

        let sample_rate = sink.sample_rate();
        let channels = sink.channels().max(1);
        let core = Arc::new(Mutex::new(Core {
            state: PlayerSnapshot::default(),
            current: None,
            upcoming: None,
            original_queue: Vec::new(),
            frames_in_current: 0,
            previous_restart_threshold_frames: u64::from(sample_rate) * 3,
            shuffle_rng: ShuffleRng::new(),
            sample_rate,
            channels,
            connection,
        }));
        // Restore the queue, current entry, position, volume, shuffle, and
        // repeat from the database before the render callback opens its first
        // PreparedTrack. A launch never plays a frame, so a fresh Player
        // always opens paused at whatever position the previous session last
        // wrote. The loop back here drains the queue rows in declared position
        // order, so the listener's saved order comes back intact.
        core.lock()
            .unwrap_or_else(|error| error.into_inner())
            .load_state()?;
        let event_handler: Arc<Mutex<Option<EventHandler>>> = Arc::new(Mutex::new(None));
        let render_core = Arc::clone(&core);
        let render_events = Arc::clone(&event_handler);
        // The last position the window was told about by this callback, so
        // plain playback emits a position snapshot a few times per second
        // instead of only on state changes.
        let mut last_reported_position_ms = 0_u64;
        let output = Box::new(sink).start(Box::new(move |samples| {
            let snapshot = {
                let mut core = render_core
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let changed = core.render(samples);
                let snapshot = core.snapshot();
                let position_moved = snapshot
                    .position_ms
                    .abs_diff(last_reported_position_ms)
                    >= POSITION_EVENT_INTERVAL_MS;
                if changed || (snapshot.playing && position_moved) {
                    // The same trigger that announces a position event also
                    // persists it, so a hard exit loses at most one chunk of
                    // the buffer instead of the whole current entry.
                    let _ = core.save_state();
                    last_reported_position_ms = snapshot.position_ms;
                    Some(snapshot)
                } else {
                    None
                }
            };
            if let Some(snapshot) = snapshot {
                emit_to(&render_events, snapshot);
            }
        }))?;

        Ok(Self {
            core,
            event_handler,
            _output: output,
        })
    }

    pub fn set_event_handler(&self, handler: impl Fn(PlayerSnapshot) + Send + Sync + 'static) {
        *self
            .event_handler
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(handler));
    }

    /// Replaces the queue with the visible list beginning at `starting_path`
    /// and immediately starts that first entry.
    pub fn play_tracks(&self, tracks: Vec<Track>, starting_path: &Path) -> Result<PlayerSnapshot> {
        let entries = tracks.into_iter().map(QueueEntry::from).collect::<Vec<_>>();
        self.play_entries(entries, starting_path)
    }

    pub fn play_entries(
        &self,
        entries: Vec<QueueEntry>,
        starting_path: &Path,
    ) -> Result<PlayerSnapshot> {
        let start = entries
            .iter()
            .position(|entry| entry.path == starting_path)
            .ok_or_else(|| {
                PlayerError("the starting track is not in the visible list".to_owned())
            })?;
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            let mut queue = entries.into_iter().skip(start).collect::<Vec<_>>();
            core.original_queue = queue.clone();
            // The unshuffled queue is what must survive a restart; save it
            // before any shuffle reshapes the in-memory playing queue.
            core.save_queue()?;
            if core.state.shuffle && queue.len() > 1 {
                let mut tail = queue.split_off(1);
                shuffle_slice(&mut tail, &mut core.shuffle_rng);
                queue.extend(tail);
            }
            core.state.queue = queue;
            core.state.current_index = (!core.state.queue.is_empty()).then_some(0);
            core.state.playing = !core.state.queue.is_empty();
            core.current = None;
            core.upcoming = None;
            core.frames_in_current = 0;
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        Ok(snapshot)
    }

    pub fn play(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            if !core.state.queue.is_empty() {
                // A None current index means the queue has either never played
                // or has just ended. Both cases start the queue from the top.
                // When the current index is already set, the saved frames are
                // kept verbatim so a position restored from disk lands on the
                // exact spot they left off; open_current seeks the freshly
                // opened PreparedTrack to that frame before its first render.
                if core.state.current_index.is_none() {
                    core.state.current_index = Some(0);
                    core.current = None;
                    core.upcoming = None;
                    core.frames_in_current = 0;
                }
                core.state.playing = true;
            }
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn pause(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.playing = false;
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn toggle(&self) -> PlayerSnapshot {
        if self.snapshot().playing {
            self.pause()
        } else {
            self.play()
        }
    }

    pub fn next(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            let was_playing = core.state.playing;
            let advanced = core.advance();
            core.state.playing = was_playing && advanced;
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn previous(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.previous();
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Stops playback and rewinds to the start of the current entry. The queue
    /// and the current entry stay, so Play starts the same entry again.
    pub fn stop(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.playing = false;
            core.current = None;
            core.upcoming = None;
            core.frames_in_current = 0;
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Seeks relative to the current position. A negative offset rewinds; the
    /// result is clamped to the current entry.
    pub fn seek(&self, offset_ms: i64) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            let current = core.frames_to_ms(core.frames_in_current) as i64;
            let target = current.saturating_add(offset_ms).max(0) as u64;
            core.seek(target);
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Positions playback at `position_ms` inside the current entry. Positions
    /// past the entry's end move to the next entry, like a seek past the end.
    pub fn seek_to_ms(&self, position_ms: u64) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.seek(position_ms);
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        self.core
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot()
    }

    /// Toggles shuffle over the live queue. Turning shuffle on keeps the
    /// current entry in place and rearranges the entries after it. Turning it
    /// off restores the queue to its original order and keeps the same entry
    /// current.
    pub fn set_shuffle(&self, on: bool) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            if on != core.state.shuffle {
                if on {
                    let split_at = core
                        .state
                        .current_index
                        .map(|index| index + 1)
                        .unwrap_or(0);
                    if split_at < core.state.queue.len() {
                        let mut tail = core.state.queue.split_off(split_at);
                        shuffle_slice(&mut tail, &mut core.shuffle_rng);
                        core.state.queue.extend(tail);
                    }
                    // The reorder invalidates any decode-ahead prepared for
                    // the old order.
                    core.upcoming = None;
                } else {
                    let current_path = core
                        .state
                        .current_index
                        .and_then(|index| core.state.queue.get(index))
                        .map(|entry| entry.path.clone());
                    core.state.queue = core.original_queue.clone();
                    core.state.current_index = current_path.and_then(|path| {
                        core.state.queue
                            .iter()
                            .position(|entry| entry.path == path)
                    });
                    // The restored order invalidates any decode-ahead
                    // prepared for the shuffled order; the current entry
                    // keeps playing untouched.
                    core.upcoming = None;
                }
                core.state.shuffle = on;
            }
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn set_repeat(&self, mode: RepeatMode) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.repeat = mode;
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Sets the app volume, a linear multiplier clamped to 0.0..=1.0. It
    /// scales only the samples of Linkamp's own stream, never the system
    /// volume, and zero keeps the stream open on silence.
    pub fn set_volume(&self, volume: f32) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.volume = volume.clamp(0.0, 1.0);
            let snapshot = core.snapshot();
            let _ = core.save_state();
            snapshot
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Forces the queue and the per-player settings out to the database. Quit
    /// and hide-to-tray call this from the Tauri command bridge so the last
    /// position the listener heard is what the next session restores, even if
    /// no render callback has fired since the last change.
    pub fn flush(&self) {
        let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
        let _ = core.save_state();
        let _ = core.save_queue();
    }

    #[doc(hidden)]
    pub fn set_previous_restart_threshold_for_testing(&self, frames: u64) {
        let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
        core.previous_restart_threshold_frames = frames;
    }

    fn emit(&self, snapshot: PlayerSnapshot) {
        emit_to(&self.event_handler, snapshot);
    }
}

fn emit_to(handler: &Mutex<Option<EventHandler>>, snapshot: PlayerSnapshot) {
    let handler = handler
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if let Some(handler) = handler {
        handler(snapshot);
    }
}

/// Symphonia decoding of one file into interleaved f32 samples in the file's
/// own channel layout and sample rate. The mixer converts from there.
struct TrackSource {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
}

impl TrackSource {
    fn open(path: &Path) -> std::result::Result<Self, SymphoniaError> {
        let file = File::open(path).map_err(SymphoniaError::IoError)?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
            hint.with_extension(extension);
        }
        let probed = symphonia::default::get_probe().format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let format = probed.format;
        let track = format.default_track().ok_or(SymphoniaError::DecodeError(
            "file has no default audio track",
        ))?;
        let track_id = track.id;
        let time_base = track.codec_params.time_base;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;
        Ok(Self {
            path: path.to_owned(),
            format,
            decoder,
            track_id,
            time_base,
        })
    }

    /// Decodes one packet into `samples` as interleaved f32 and reports the
    /// channel count and sample rate. Returns None at the end of the stream.
    fn decode_packet_into(&mut self, samples: &mut Vec<f32>) -> Option<(usize, u32)> {
        loop {
            let packet = self.format.next_packet().ok()?;
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    let spec = *audio.spec();
                    let mut buffer = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
                    buffer.copy_interleaved_ref(audio);
                    if buffer.samples().is_empty() {
                        continue;
                    }
                    samples.clear();
                    samples.extend_from_slice(buffer.samples());
                    return Some((spec.channels.count(), spec.rate));
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(_) => return None,
            }
        }
    }
}

/// Converts one decoder's channels and sample rate to the single stream
/// format, so consecutive tracks in different formats still meet
/// sample-exactly at the boundary. Rate conversion is linear interpolation
/// between source frames; equal rates and channels are a plain copy.
struct Mixer {
    source_channels: usize,
    output_channels: usize,
    /// Source frames consumed per output frame.
    ratio: f64,
    /// Fractional position of the next output frame, relative to the front of
    /// `input`, always below one frame after normalization.
    position: f64,
    input: VecDeque<f32>,
}

impl Mixer {
    fn new(
        source_channels: usize,
        source_rate: u32,
        output_channels: usize,
        output_rate: u32,
    ) -> Self {
        Self {
            source_channels: source_channels.max(1),
            output_channels: output_channels.max(1),
            ratio: f64::from(source_rate) / f64::from(output_rate),
            position: 0.0,
            input: VecDeque::new(),
        }
    }

    fn push(&mut self, samples: &[f32]) {
        self.input.extend(samples.iter().copied());
    }

    /// Converts buffered input into interleaved output frames. Mid-stream a
    /// frame waits for its second interpolation tap instead of inventing a
    /// clamped value early; at end of stream the tail is flushed by clamping
    /// to the last sample, so a track yields exactly ceil(frames / ratio)
    /// output frames.
    fn produce(&mut self, output: &mut VecDeque<f32>, end_of_stream: bool) {
        if self.ratio == 1.0 && self.source_channels == self.output_channels {
            output.extend(self.input.drain(..));
            return;
        }
        loop {
            let available = self.input.len() / self.source_channels;
            let base = self.position.floor() as usize;
            if base >= available {
                return;
            }
            if base + 1 >= available && !end_of_stream {
                return;
            }
            let fraction = self.position - base as f64;
            let first = base * self.source_channels;
            let second = if base + 1 < available {
                first + self.source_channels
            } else {
                first
            };
            for channel in 0..self.output_channels {
                let a = self.channel_value(first, channel);
                let b = self.channel_value(second, channel);
                output.push_back((a + (b - a) * fraction) as f32);
            }
            self.position += self.ratio;
            let consumed = self.position.floor() as usize;
            if consumed > 0 {
                self.input
                    .drain(..(consumed * self.source_channels).min(self.input.len()));
                self.position -= consumed as f64;
            }
        }
    }

    /// Maps one source frame to an output channel: mono is duplicated, a
    /// multi-channel source is averaged into a mono output, and otherwise the
    /// shared channels are copied and extra output channels repeat the last
    /// source channel.
    fn channel_value(&self, frame_start: usize, channel: usize) -> f64 {
        if self.output_channels == 1 && self.source_channels > 1 {
            let sum: f64 = (0..self.source_channels)
                .map(|index| f64::from(self.input[frame_start + index]))
                .sum();
            sum / self.source_channels as f64
        } else {
            f64::from(self.input[frame_start + channel.min(self.source_channels - 1)])
        }
    }
}

/// One queue entry being decoded and converted to the stream format. Holds a
/// bounded span of converted audio, never the whole file. Used for the playing
/// track and for decode-ahead of the next queue entry.
struct PreparedTrack {
    queue_index: usize,
    source: TrackSource,
    mixer: Option<Mixer>,
    converted: VecDeque<f32>,
    scratch: Vec<f32>,
    finished: bool,
    /// Output frames handed out or discarded since the file was opened or
    /// last seeked. Anchors a seek target to the absolute stream position.
    frames_rendered: u64,
    output_rate: u32,
    output_channels: usize,
}

impl PreparedTrack {
    fn open(
        path: &Path,
        queue_index: usize,
        output_rate: u32,
        output_channels: usize,
    ) -> std::result::Result<Self, SymphoniaError> {
        Ok(Self {
            queue_index,
            source: TrackSource::open(path)?,
            mixer: None,
            converted: VecDeque::new(),
            scratch: Vec::new(),
            finished: false,
            frames_rendered: 0,
            output_rate,
            output_channels,
        })
    }

    /// True once the source is exhausted and every converted frame was drained.
    fn drained(&self) -> bool {
        self.finished && self.converted.is_empty()
    }

    /// Decodes until at least `frames` converted frames are buffered or the
    /// track ends, converting channels and rate as packets arrive.
    fn fill(&mut self, frames: usize) {
        let target = frames.saturating_mul(self.output_channels);
        while self.converted.len() < target && !self.finished {
            match self.source.decode_packet_into(&mut self.scratch) {
                Some((channels, rate)) => {
                    let mixer = self.mixer.get_or_insert_with(|| {
                        Mixer::new(channels, rate, self.output_channels, self.output_rate)
                    });
                    mixer.push(&self.scratch);
                    mixer.produce(&mut self.converted, false);
                }
                None => {
                    if let Some(mixer) = self.mixer.as_mut() {
                        mixer.produce(&mut self.converted, true);
                    }
                    self.finished = true;
                }
            }
        }
    }

    /// Writes converted interleaved samples into `output`, decoding on demand.
    /// Returns the number of samples written; zero means the track is drained.
    fn write_frames(&mut self, output: &mut [f32]) -> usize {
        if self.converted.is_empty() && !self.finished {
            self.fill(output.len() / self.output_channels);
        }
        let count = output.len().min(self.converted.len());
        for (target, sample) in output.iter_mut().zip(self.converted.drain(..count)) {
            *target = sample;
        }
        self.frames_rendered += (count / self.output_channels.max(1)) as u64;
        count
    }

    /// Positions the stream so that the next frame handed out is `target`,
    /// counted in output frames since the start of the file. The demuxer seeks
    /// first; only the remaining gap is decoded and discarded. Returns false
    /// when the file ends before the target.
    fn seek_to(&mut self, target: u64) -> bool {
        let Some(landed) = self.demuxer_seek(target) else {
            return false;
        };
        self.mixer = None;
        self.converted.clear();
        self.finished = false;
        self.frames_rendered = landed;
        let channels = self.output_channels.max(1);
        while self.frames_rendered < target {
            self.fill(64);
            if self.converted.is_empty() {
                return false;
            }
            let drop = ((target - self.frames_rendered) as usize)
                .min(self.converted.len() / channels);
            self.converted.drain(..drop * channels);
            self.frames_rendered += drop as u64;
        }
        true
    }

    /// Moves the demuxer near `target` and returns the absolute output-frame
    /// position it landed on, discarding buffered samples. When the container
    /// cannot seek, the file is reopened from the top instead. Returns None
    /// when even a reopen fails.
    fn demuxer_seek(&mut self, target: u64) -> Option<u64> {
        if let Some(time_base) = self.source.time_base {
            let seconds = target as f64 / f64::from(self.output_rate);
            let timestamp =
                (seconds * f64::from(time_base.denom) / f64::from(time_base.numer)).floor()
                    as u64;
            let sought = self.source.format.seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    track_id: self.source.track_id,
                    ts: timestamp,
                },
            );
            if let Ok(seeked) = sought {
                self.source.decoder.reset();
                let landed = seeked.actual_ts as f64 * f64::from(time_base.numer)
                    / f64::from(time_base.denom);
                return Some((landed * f64::from(self.output_rate)).round() as u64);
            }
        }
        // Reopen from the top when the container cannot seek or reports no
        // time base; the caller closes the remaining gap by discarding.
        let path = self.source.path.clone();
        match TrackSource::open(&path) {
            Ok(fresh) => {
                self.source = fresh;
                Some(0)
            }
            Err(_) => None,
        }
    }
}

/// Real application output. Construction does not fail when no device exists;
/// callers can fall back to `NoDeviceOutputSink` and keep the UI usable.
pub struct CpalOutputSink {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
}

impl CpalOutputSink {
    pub fn default_output() -> std::result::Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "no audio output device is available".to_owned())?;
        let config = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        Ok(Self { device, config })
    }
}

struct CpalRunningOutput(std::sync::mpsc::Sender<()>);
impl RunningOutput for CpalRunningOutput {}

impl Drop for CpalRunningOutput {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

impl OutputSink for CpalOutputSink {
    fn sample_rate(&self) -> u32 {
        self.config.sample_rate().0
    }

    fn channels(&self) -> usize {
        self.config.channels() as usize
    }

    fn start(self: Box<Self>, mut render: RenderCallback) -> Result<Box<dyn RunningOutput>> {
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let config: cpal::StreamConfig = self.config.clone().into();
            let on_error = |error| eprintln!("audio output stream: {error}");
            let stream = match self.config.sample_format() {
                cpal::SampleFormat::F32 => self.device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _| render(data),
                    on_error,
                    None,
                ),
                cpal::SampleFormat::I16 => self.device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        let mut samples = vec![0.0; data.len()];
                        render(&mut samples);
                        for (target, sample) in data.iter_mut().zip(samples) {
                            *target = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                    },
                    on_error,
                    None,
                ),
                cpal::SampleFormat::U16 => self.device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        let mut samples = vec![0.0; data.len()];
                        render(&mut samples);
                        for (target, sample) in data.iter_mut().zip(samples) {
                            *target =
                                ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                        }
                    },
                    on_error,
                    None,
                ),
                format => {
                    let _ = ready_sender
                        .send(Err(format!("unsupported output sample format: {format:?}")));
                    return;
                }
            };
            match stream {
                Ok(stream) => match stream.play() {
                    Ok(()) => {
                        let _ = ready_sender.send(Ok(()));
                        let _ = stop_receiver.recv();
                        drop(stream);
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error.to_string()));
                    }
                },
                Err(error) => {
                    let _ = ready_sender.send(Err(error.to_string()));
                }
            }
        });
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Box::new(CpalRunningOutput(stop_sender))),
            Ok(Err(error)) => {
                eprintln!("audio output unavailable: {error}");
                Ok(Box::new(NoDeviceRunningOutput))
            }
            Err(error) => Err(PlayerError(error.to_string())),
        }
    }
}

/// Keeps player commands operational when CPAL cannot find or open a device.
pub struct NoDeviceOutputSink {
    pub sample_rate: u32,
    pub channels: usize,
}

struct NoDeviceRunningOutput;
impl RunningOutput for NoDeviceRunningOutput {}

pub enum SystemOutputSink {
    Device(CpalOutputSink),
    NoDevice(NoDeviceOutputSink),
}

pub fn system_output_sink() -> SystemOutputSink {
    CpalOutputSink::default_output()
        .map(SystemOutputSink::Device)
        .unwrap_or_else(|error| {
            eprintln!("audio output unavailable: {error}");
            SystemOutputSink::NoDevice(NoDeviceOutputSink {
                sample_rate: 48_000,
                channels: 2,
            })
        })
}

impl OutputSink for SystemOutputSink {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Device(sink) => sink.sample_rate(),
            Self::NoDevice(sink) => sink.sample_rate(),
        }
    }
    fn channels(&self) -> usize {
        match self {
            Self::Device(sink) => sink.channels(),
            Self::NoDevice(sink) => sink.channels(),
        }
    }
    fn start(self: Box<Self>, render: RenderCallback) -> Result<Box<dyn RunningOutput>> {
        match *self {
            Self::Device(sink) => Box::new(sink).start(render),
            Self::NoDevice(sink) => Box::new(sink).start(render),
        }
    }
}

impl OutputSink for NoDeviceOutputSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn start(self: Box<Self>, _render: RenderCallback) -> Result<Box<dyn RunningOutput>> {
        Ok(Box::new(NoDeviceRunningOutput))
    }
}

#[derive(Clone)]
pub struct FakeOutputSink {
    inner: Arc<Mutex<FakeOutputInner>>,
    sample_rate: u32,
    channels: usize,
}

#[derive(Default)]
struct FakeOutputInner {
    render: Option<RenderCallback>,
    pulls: usize,
    starts: usize,
}

impl FakeOutputSink {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeOutputInner::default())),
            sample_rate,
            channels,
        }
    }

    /// Deterministically asks for `frames` more frames and returns exactly what
    /// the output device would have received.
    pub fn pull(&self, frames: usize) -> Vec<f32> {
        let mut samples = vec![0.0; frames * self.channels];
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(render) = inner.render.as_mut() {
            render(&mut samples);
            inner.pulls += 1;
        }
        samples
    }

    pub fn pulls(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pulls
    }

    /// How often the output stream was opened. Gapless playback opens it once
    /// for the player's lifetime and never reopens it at a track boundary.
    pub fn starts(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .starts
    }
}

struct FakeRunningOutput;
impl RunningOutput for FakeRunningOutput {}

impl OutputSink for FakeOutputSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn start(self: Box<Self>, render: RenderCallback) -> Result<Box<dyn RunningOutput>> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.starts += 1;
        inner.render = Some(render);
        Ok(Box::new(FakeRunningOutput))
    }
}

/// Fisher-Yates shuffle driven by a small deterministic RNG. The state is
/// reseeded on each Player open, so a given player produces a different order
/// each time shuffle is turned on, which is what the spec asks for.
struct ShuffleRng(u64);

impl ShuffleRng {
    fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Self(splitmix64(nanos ^ 0x9E3779B97F4A7C15))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = splitmix64(self.0);
        self.0
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}

fn shuffle_slice<T>(slice: &mut [T], rng: &mut ShuffleRng) {
    for index in (1..slice.len()).rev() {
        let chosen = (rng.next_u64() as usize) % (index + 1);
        slice.swap(index, chosen);
    }
}
