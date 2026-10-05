use std::fs;
use std::path::{Path, PathBuf};

use linkamp_lib::library::{LibraryIndex, Track};
use linkamp_lib::player::{FakeOutputSink, NoDeviceOutputSink, Player, QueueEntry};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
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

/// Decodes a fixture the way the player does and reports its sample rate and
/// frame count, so a test can assert the exact frame at which the sink crosses
/// from one track's samples to the next track's.
fn decoded_shape(path: &Path) -> (u32, usize) {
    let file = fs::File::open(path).unwrap();
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(path.extension().unwrap().to_str().unwrap());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .unwrap();
    let mut format = probed.format;
    let track = format.default_track().unwrap();
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .unwrap();
    let mut rate = 0;
    let mut frames = 0;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                rate = audio.spec().rate;
                frames += audio.frames();
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    (rate, frames)
}

/// The mixer's output length for a track: exact when the rates match, and
/// ceil(frames * sink / source) with linear interpolation otherwise.
fn expected_output_frames(source_frames: usize, source_rate: u32, sink_rate: u32) -> usize {
    assert!(source_frames > 0, "fixtures must decode to some audio");
    if source_rate == sink_rate {
        return source_frames;
    }
    (source_frames as f64 * f64::from(sink_rate) / f64::from(source_rate)).ceil() as usize
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

#[test]
fn consecutive_tracks_cross_without_a_gap_on_one_open_stream() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let one = audio_file(&album, "one", "mp3");
    let two = audio_file(&album, "two", "flac");
    let three = audio_file(&album, "three", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![
                entry(one.clone(), "One"),
                entry(two.clone(), "Two"),
                entry(three.clone(), "Three"),
            ],
            &one,
        )
        .unwrap();

    let (rate, frames_one) = decoded_shape(&one);
    let (_, frames_two) = decoded_shape(&two);
    let (_, frames_three) = decoded_shape(&three);
    assert_eq!(rate, 44_100, "the fixtures match the sink rate");

    // Exactly the first track's frames arrive; the current entry has not moved.
    let first = sink.pull(frames_one);
    assert_eq!(player.snapshot().current_index, Some(0));
    assert!(first.iter().all(|sample| *sample == 0.0));

    // The very next frame already belongs to the second track: the MP3's last
    // sample is followed immediately by the FLAC's first sample.
    let _ = sink.pull(1);
    assert_eq!(player.snapshot().current_index, Some(1));

    let _ = sink.pull(frames_two - 1);
    assert_eq!(player.snapshot().current_index, Some(1));
    let _ = sink.pull(1);
    assert_eq!(player.snapshot().current_index, Some(2));

    let _ = sink.pull(frames_three);
    assert_eq!(player.snapshot().current_index, Some(2));
    let end = sink.pull(1);
    assert!(
        !player.snapshot().playing,
        "playback stops after the queue's last frame"
    );
    assert!(end.iter().all(|sample| *sample == 0.0));

    assert_eq!(
        sink.starts(),
        1,
        "the output stream opened once and was never closed or reopened"
    );
}

#[test]
fn a_format_change_during_rate_conversion_crosses_without_a_gap() {
    // Both fixtures are 44.1 kHz; the sink runs at 22.05 kHz, so every track
    // is rate-converted and the MP3-to-FLAC boundary lands mid-conversion.
    // The ratio is exactly 2, so the boundary frame is exact.
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let mp3 = audio_file(&album, "first", "mp3");
    let flac = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(22_050, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![entry(mp3.clone(), "MP3"), entry(flac.clone(), "FLAC")],
            &mp3,
        )
        .unwrap();

    let (rate, mp3_frames) = decoded_shape(&mp3);
    let (_, flac_frames) = decoded_shape(&flac);
    assert_eq!(rate, 44_100);
    let mp3_output = expected_output_frames(mp3_frames, 44_100, 22_050);
    let flac_output = expected_output_frames(flac_frames, 44_100, 22_050);

    let _ = sink.pull(mp3_output);
    assert_eq!(player.snapshot().current_index, Some(0));
    let _ = sink.pull(1);
    assert_eq!(
        player.snapshot().current_index,
        Some(1),
        "the frame after the resampled MP3's last frame is the FLAC's first"
    );
    let _ = sink.pull(flac_output - 1);
    assert_eq!(player.snapshot().current_index, Some(1));
    let _ = sink.pull(1);
    assert!(!player.snapshot().playing);
    assert_eq!(sink.starts(), 1);
}

#[test]
fn a_boundary_at_a_realistic_rate_change_stays_gapless() {
    // The daily-machine setup: 44.1 kHz files on a 48 kHz stream. The ratio
    // is not a round number, so the boundary is asserted with one frame of
    // tolerance around the interpolated length.
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let mp3 = audio_file(&album, "first", "mp3");
    let flac = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(48_000, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![entry(mp3.clone(), "MP3"), entry(flac.clone(), "FLAC")],
            &mp3,
        )
        .unwrap();

    let mp3_output = expected_output_frames(decoded_shape(&mp3).1, 44_100, 48_000);
    let flac_output = expected_output_frames(decoded_shape(&flac).1, 44_100, 48_000);

    let _ = sink.pull(mp3_output - 2);
    assert_eq!(player.snapshot().current_index, Some(0));
    let _ = sink.pull(4);
    assert_eq!(
        player.snapshot().current_index,
        Some(1),
        "the boundary lands within one frame of the interpolated MP3 length"
    );
    let _ = sink.pull(flac_output + 3);
    assert!(
        !player.snapshot().playing,
        "the FLAC runs its interpolated length and the queue ends"
    );
    assert_eq!(sink.starts(), 1);
}

#[test]
fn pause_writes_silence_into_the_open_stream_and_resume_is_sample_exact() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let mp3 = audio_file(&album, "first", "mp3");
    let flac = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![entry(mp3.clone(), "MP3"), entry(flac.clone(), "FLAC")],
            &mp3,
        )
        .unwrap();
    let (_, mp3_frames) = decoded_shape(&mp3);
    let heard = 1_000;
    let _ = sink.pull(heard);

    player.pause();
    let paused = sink.pull(2_000);
    assert!(
        paused.iter().all(|sample| *sample == 0.0),
        "pause feeds silence into the still-open stream"
    );
    assert_eq!(player.snapshot().current_index, Some(0));

    player.play();
    // The pause consumed nothing: exactly the remaining frames of the MP3
    // play out, and the frame after them belongs to the FLAC.
    let _ = sink.pull(mp3_frames - heard);
    assert_eq!(
        player.snapshot().current_index,
        Some(0),
        "resume continues from the exact frame the pause left behind"
    );
    let _ = sink.pull(1);
    assert_eq!(player.snapshot().current_index, Some(1));
    assert_eq!(sink.starts(), 1);
}

#[cfg(unix)]
#[test]
fn the_next_entry_is_decoded_before_the_current_one_ends() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let mp3 = audio_file(&album, "first", "mp3");
    let flac = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![entry(mp3.clone(), "MP3"), entry(flac.clone(), "FLAC")],
            &mp3,
        )
        .unwrap();
    let (_, mp3_frames) = decoded_shape(&mp3);
    let (_, flac_frames) = decoded_shape(&flac);
    let _ = sink.pull(100);

    // While the MP3 still has almost its whole length ahead of it, the FLAC
    // becomes unreadable. A player that opened it only at the boundary would
    // skip it; decode-ahead already holds its samples, so it plays in full.
    fs::set_permissions(&flac, fs::Permissions::from_mode(0o000)).unwrap();

    let _ = sink.pull(mp3_frames - 100);
    assert_eq!(player.snapshot().current_index, Some(0));
    let _ = sink.pull(1);
    assert_eq!(player.snapshot().current_index, Some(1));
    let _ = sink.pull(flac_frames - 1);
    assert_eq!(player.snapshot().current_index, Some(1));
    let _ = sink.pull(1);
    assert!(
        !player.snapshot().playing,
        "the prepared FLAC played to its end although its file was unreadable"
    );

    fs::set_permissions(&flac, fs::Permissions::from_mode(0o644)).unwrap();
}

