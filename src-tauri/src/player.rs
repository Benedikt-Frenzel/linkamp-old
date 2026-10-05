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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub queue: Vec<QueueEntry>,
    pub current_index: Option<usize>,
    pub playing: bool,
    /// Elapsed time inside the current entry. It advances only while samples
    /// reach the sink, so a pause freezes it, and it restarts with each track.
    pub position_ms: u64,
}

struct Core {
    state: PlayerSnapshot,
    decoder: Option<AudioDecoder>,
    sample_rate: u32,
    channels: usize,
    /// Output frames of the current entry that have reached the sink. Kept
    /// beside the state because it advances per frame, not per command.
    position_frames: u64,
    connection: Connection,
}

impl Core {
    fn snapshot(&self) -> PlayerSnapshot {
        let mut snapshot = self.state.clone();
        snapshot.position_ms = self.frames_to_ms(self.position_frames);
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
        match AudioDecoder::open(&entry.path, self.sample_rate) {
            Ok(decoder) => {
                self.decoder = Some(decoder);
                true
            }
            Err(error) => {
                eprintln!("could not decode {}: {error}", entry.path.display());
                self.decoder = None;
                false
            }
        }
    }

    /// Moves to the entry after the current one, stopping at the end of the
    /// queue. Returns whether an entry is current afterwards.
    fn advance(&mut self) -> bool {
        self.position_frames = 0;
        let next = self.state.current_index.map_or(0, |index| index + 1);
        if next >= self.state.queue.len() {
            self.state.playing = false;
            self.decoder = None;
            return false;
        }
        self.state.current_index = Some(next);
        self.decoder = None;
        self.open_current()
    }

    fn previous(&mut self) {
        self.position_frames = 0;
        let previous = self
            .state
            .current_index
            .map_or(0, |index| index.saturating_sub(1));
        if self.state.queue.is_empty() {
            self.state.current_index = None;
            self.state.playing = false;
        } else {
            self.state.current_index = Some(previous);
            self.decoder = None;
            if self.state.playing {
                self.open_current();
            }
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
        if self.decoder.is_none() && !self.open_current() {
            let was_playing = self.state.playing;
            self.advance();
            self.state.playing = was_playing && self.state.current_index.is_some();
            return;
        }
        let target = self.ms_to_frames(position_ms);
        let reached = self
            .decoder
            .as_mut()
            .is_some_and(|decoder| decoder.seek_to(target));
        if reached {
            self.position_frames = target;
        } else {
            let was_playing = self.state.playing;
            self.advance();
            self.state.playing = was_playing && self.state.current_index.is_some();
        }
    }

    fn render(&mut self, output: &mut [f32]) -> bool {
        output.fill(0.0);
        if !self.state.playing || self.state.queue.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut cursor = 0;
        while cursor + self.channels <= output.len() && self.state.playing {
            if self.decoder.is_none() && !self.open_current() {
                if !self.advance() {
                    changed = true;
                    break;
                }
                changed = true;
                continue;
            }

            let wrote_frame = self.decoder.as_mut().is_some_and(|decoder| {
                decoder.write_frame(&mut output[cursor..cursor + self.channels])
            });
            if wrote_frame {
                cursor += self.channels;
                self.position_frames = self.position_frames.saturating_add(1);
            } else {
                self.decoder = None;
                self.advance();
                changed = true;
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
            decoder: None,
            sample_rate,
            channels,
            position_frames: 0,
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
            core.state.queue = entries.into_iter().skip(start).collect();
            core.state.current_index = (!core.state.queue.is_empty()).then_some(0);
            core.state.playing = !core.state.queue.is_empty();
            core.decoder = None;
            core.position_frames = 0;
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
                if core.state.current_index.is_none() {
                    core.state.current_index = Some(0);
                    core.decoder = None;
                }
                if core.decoder.is_none() {
                    // Nothing is decoded: the current entry starts from the top.
                    core.position_frames = 0;
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
            core.advance();
            core.state.playing = was_playing && core.state.current_index.is_some();
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

    /// Stops playback and rewinds to the start of the current entry. The queue
    /// and the current entry stay, so Play starts the same entry again.
    pub fn stop(&self) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            core.state.playing = false;
            core.decoder = None;
            core.position_frames = 0;
            core.snapshot()
        };
        self.emit(snapshot.clone());
        snapshot
    }

    /// Seeks relative to the current position. A negative offset rewinds; the
    /// result is clamped to the current entry.
    pub fn seek(&self, offset_ms: i64) -> PlayerSnapshot {
        let snapshot = {
            let mut core = self.core.lock().unwrap_or_else(|error| error.into_inner());
            let current = core.frames_to_ms(core.position_frames) as i64;
            let target = current.saturating_add(offset_ms).max(0) as u64;
            core.seek(target);
            core.snapshot()
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

struct AudioDecoder {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    decoded: Vec<f32>,
    cursor: usize,
    source_channels: usize,
    source_rate: u32,
    output_rate: u32,
    rate_position: f64,
    /// Output frames consumed since the file was opened or last seeked.
    frames_rendered: u64,
}

impl AudioDecoder {
    fn open(path: &Path, output_rate: u32) -> std::result::Result<Self, SymphoniaError> {
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
            decoded: Vec::new(),
            cursor: 0,
            source_channels: 0,
            source_rate: output_rate,
            output_rate,
            rate_position: 0.0,
            frames_rendered: 0,
        })
    }

    fn decode_packet(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(_) => return false,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    self.source_channels = audio.spec().channels.count();
                    self.source_rate = audio.spec().rate;
                    let mut buffer =
                        SampleBuffer::<f32>::new(audio.capacity() as u64, *audio.spec());
                    buffer.copy_interleaved_ref(audio);
                    self.decoded.clear();
                    self.decoded.extend_from_slice(buffer.samples());
                    self.cursor = 0;
                    if !self.decoded.is_empty() {
                        return true;
                    }
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }

    /// Consumes one output frame of decoded audio, decoding packets as
    /// needed. Returns the index of the frame's first sample in `decoded`,
    /// or `None` at the end of the stream.
    fn next_frame(&mut self) -> Option<usize> {
        if self.cursor + self.source_channels.max(1) > self.decoded.len() && !self.decode_packet() {
            return None;
        }
        if self.source_channels == 0 {
            return None;
        }

        let source_frame = self.cursor / self.source_channels;
        let base = source_frame * self.source_channels;
        self.rate_position += self.source_rate as f64 / self.output_rate as f64;
        let advance = self.rate_position.floor() as usize;
        self.rate_position -= advance as f64;
        self.cursor = self.cursor.saturating_add(advance * self.source_channels);
        self.frames_rendered += 1;
        Some(base)
    }

    fn write_frame(&mut self, output: &mut [f32]) -> bool {
        let Some(base) = self.next_frame() else {
            return false;
        };
        for (channel, sample) in output.iter_mut().enumerate() {
            *sample = if self.source_channels == 1 {
                self.decoded[base]
            } else {
                self.decoded[base + channel.min(self.source_channels - 1)]
            };
        }
        true
    }

    /// Positions the stream so that the next frame handed out is `target`,
    /// counted in output frames since the start of the file. The demuxer seeks
    /// first; only the remaining gap is decoded and discarded. Returns `false`
    /// when the file ends before the target.
    fn seek_to(&mut self, target: u64) -> bool {
        if !self.demuxer_seek(target) {
            return false;
        }
        while self.frames_rendered < target {
            if self.next_frame().is_none() {
                return false;
            }
        }
        true
    }

    /// Moves the demuxer near `target`, discarding buffered samples. When the
    /// container cannot seek, the file is reopened from the top instead.
    fn demuxer_seek(&mut self, target: u64) -> bool {
        self.decoded.clear();
        self.cursor = 0;
        self.rate_position = 0.0;
        if let Some(time_base) = self.time_base {
            let seconds = target as f64 / f64::from(self.output_rate);
            let timestamp =
                (seconds * f64::from(time_base.denom) / f64::from(time_base.numer)).floor() as u64;
            let sought = self.format.seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    track_id: self.track_id,
                    ts: timestamp,
                },
            );
            if let Ok(seeked) = sought {
                self.decoder.reset();
                let landed = seeked.actual_ts as f64 * f64::from(time_base.numer)
                    / f64::from(time_base.denom);
                self.frames_rendered = (landed * f64::from(self.output_rate)).round() as u64;
                return true;
            }
        }
        match Self::open(&self.path, self.output_rate) {
            Ok(fresh) => {
                *self = fresh;
                true
            }
            Err(_) => false,
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
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .render = Some(render);
        Ok(Box::new(FakeRunningOutput))
    }
}
