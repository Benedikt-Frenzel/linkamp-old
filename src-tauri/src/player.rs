use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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
}

impl From<Track> for QueueEntry {
    fn from(track: Track) -> Self {
        Self {
            path: track.path,
            title: track.title,
            artist: track.artist,
            album: track.album,
            duration_ms: track.duration_ms,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub queue: Vec<QueueEntry>,
    pub current_index: Option<usize>,
    pub playing: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            current_index: None,
            playing: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }
}

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
        self.state.clone()
    }

    fn open_current(&mut self) -> bool {
        let Some(index) = self.state.current_index else {
            return false;
        };
        let Some(entry) = self.state.queue.get(index) else {
            return false;
        };
        match PreparedTrack::open(&entry.path, index, self.sample_rate, self.channels) {
            Ok(track) => {
                self.current = Some(track);
                self.frames_in_current = 0;
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
    /// becomes the current one without opening anything. Returns false at the
    /// end of the queue, where playback stops and the queue stays in place.
    fn advance(&mut self) -> bool {
        if self.state.repeat == RepeatMode::One && self.state.current_index.is_some() {
            // Replay the current entry from the top.
            self.current = None;
            self.upcoming = None;
            self.frames_in_current = 0;
            return self.open_current();
        }
        let next = self.state.current_index.map_or(0, |index| index + 1);
        if next >= self.state.queue.len() {
            if self.state.repeat == RepeatMode::All && !self.state.queue.is_empty() {
                // Wrap to the top of the queue.
                self.state.current_index = Some(0);
                self.current = None;
                self.upcoming = None;
                self.frames_in_current = 0;
                return self.open_current();
            }
            // Repeat Off at the end: stop with the queue in place. A None
            // current index is the stopped sentinel; Play starts from the top.
            self.state.playing = false;
            self.state.current_index = None;
            self.current = None;
            self.upcoming = None;
            self.frames_in_current = 0;
            return false;
        }
        self.state.current_index = Some(next);
        self.current = self
            .upcoming
            .take()
            .filter(|track| track.queue_index == next);
        self.frames_in_current = 0;
        self.prepare_upcoming();
        true
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

    fn save_queue(&mut self) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM player_queue", [])?;
        for (position, entry) in self.state.queue.iter().enumerate() {
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
        let event_handler: Arc<Mutex<Option<EventHandler>>> = Arc::new(Mutex::new(None));
        let render_core = Arc::clone(&core);
        let render_events = Arc::clone(&event_handler);
        let output = Box::new(sink).start(Box::new(move |samples| {
            let snapshot = {
                let mut core = render_core
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                core.render(samples).then(|| core.snapshot())
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
            core.save_queue()?;
            core.snapshot()
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
                if core.state.current_index.is_none() {
                    core.state.current_index = Some(0);
                    core.current = None;
                    core.upcoming = None;
                    core.frames_in_current = 0;
                }
                core.state.playing = true;
            }
            core.snapshot()
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn pause(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.playing = false;
            core.snapshot()
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
            core.snapshot()
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn previous(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.previous();
            core.snapshot()
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
            core.snapshot()
        };
        self.emit(snapshot.clone());
        snapshot
    }

    pub fn set_repeat(&self, mode: RepeatMode) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.repeat = mode;
            core.snapshot()
        };
        self.emit(snapshot.clone());
        snapshot
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
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
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
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;
        Ok(Self {
            format,
            decoder,
            track_id,
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
        count
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
