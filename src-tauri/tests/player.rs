use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use linkamp_lib::library::{LibraryIndex, Track};
use linkamp_lib::player::{FakeOutputSink, NoDeviceOutputSink, Player, QueueEntry, RepeatMode};
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
        missing: false,
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
fn a_missing_middle_entry_keeps_its_snapshot_and_is_skipped() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let missing = audio_file(&album, "missing", "flac");
    let last = audio_file(&album, "last", "mp3");
    let last_contents = fs::read(&last).unwrap();
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    let mut missing_entry = entry(missing.clone(), "Remembered title");
    missing_entry.artist = Some("Remembered artist".to_owned());
    missing_entry.album = "Remembered album".to_owned();
    missing_entry.duration_ms = 123_456;
    player
        .play_entries(
            vec![
                entry(first.clone(), "First"),
                missing_entry,
                entry(last.clone(), "Last"),
            ],
            &first,
        )
        .unwrap();

    fs::remove_file(&missing).unwrap();
    let queued = player.snapshot();
    assert!(queued.queue[1].missing);
    assert_eq!(queued.queue[1].title, "Remembered title");
    assert_eq!(queued.queue[1].artist.as_deref(), Some("Remembered artist"));
    assert_eq!(queued.queue[1].album, "Remembered album");
    assert_eq!(queued.queue[1].duration_ms, 123_456);

    for _ in 0..30 {
        sink.pull(500);
        if player.snapshot().current_index == Some(2) {
            break;
        }
    }
    let after_skip = player.snapshot();
    assert_eq!(after_skip.current_index, Some(2));
    assert!(
        after_skip.playing,
        "playback continues after the missing row"
    );
    assert_eq!(fs::read(&last).unwrap(), last_contents);
    assert!(first.is_file());
    assert!(!missing.exists());
}

#[test]
fn missing_entries_at_the_queue_edges_are_skipped() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let missing_first = audio_file(&album, "missing-first", "mp3");
    let playable = audio_file(&album, "playable", "flac");
    let missing_last = audio_file(&album, "missing-last", "mp3");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![
                entry(missing_first.clone(), "Missing first"),
                entry(playable.clone(), "Playable"),
                entry(missing_last.clone(), "Missing last"),
            ],
            &missing_first,
        )
        .unwrap();
    fs::remove_file(&missing_first).unwrap();
    fs::remove_file(&missing_last).unwrap();

    sink.pull(1);
    let after_first = player.snapshot();
    assert_eq!(after_first.current_index, Some(1));
    assert!(after_first.playing);
    assert!(after_first.queue[0].missing);
    assert!(after_first.queue[2].missing);

    for _ in 0..30 {
        sink.pull(500);
        if !player.snapshot().playing {
            break;
        }
    }
    let after_last = player.snapshot();
    assert!(!after_last.playing, "a missing final row ends the queue");
    assert_eq!(
        after_last.current_index,
        None,
        "the queue stops without the missing row ever becoming the played track"
    );
    assert!(after_last.queue.len() == 3, "the queue stays in place");
    assert!(playable.is_file());
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
    let end = sink.pull(1);
    let stopped = player.snapshot();
    assert!(
        !stopped.playing,
        "playback stops after the queue's last frame"
    );
    assert_eq!(
        stopped.current_index,
        None,
        "the stopped queue carries no current entry; Play starts from the top"
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

fn paths(entries: &[QueueEntry]) -> Vec<PathBuf> {
    entries.iter().map(|entry| entry.path.clone()).collect()
}

fn paths_set(entries: &[QueueEntry]) -> HashSet<PathBuf> {
    entries.iter().map(|entry| entry.path.clone()).collect()
}

fn four_track_queue(temp: &TempDir) -> (Vec<PathBuf>, Player, FakeOutputSink) {
    let album = temp.path().join("Album");
    let a = audio_file(&album, "a", "mp3");
    let b = audio_file(&album, "b", "flac");
    let c = audio_file(&album, "c", "mp3");
    let d = audio_file(&album, "d", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(
            vec![entry(a.clone(), "A"), entry(b.clone(), "B"), entry(c.clone(), "C"), entry(d.clone(), "D")],
            &a,
        )
        .unwrap();
    (vec![a, b, c, d], player, sink)
}

#[test]
fn shuffle_rearranges_only_entries_after_the_current() {
    let temp = TempDir::new().unwrap();
    let (paths_in_order, player, _sink) = four_track_queue(&temp);
    assert_eq!(player.snapshot().queue.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), paths_in_order);

    let shuffled = player.set_shuffle(true).queue;
    assert_eq!(shuffled[0].path, paths_in_order[0], "the current entry keeps playing");
    assert_eq!(
        paths_set(&shuffled[1..]),
        paths_set(&paths_in_order[1..].iter().map(|p| QueueEntry {
            path: p.clone(),
            title: String::new(),
            artist: None,
            album: String::new(),
            duration_ms: 0,
            missing: false,
        }).collect::<Vec<_>>()),
        "the remaining entries are a permutation of the original tail"
    );
}

#[test]
fn turning_shuffle_off_restores_the_original_order() {
    let temp = TempDir::new().unwrap();
    let (paths_in_order, player, _sink) = four_track_queue(&temp);
    player.set_shuffle(true);

    let restored = player.set_shuffle(false).queue;
    assert_eq!(
        paths(&restored),
        paths_in_order,
        "turning shuffle off returns the queue to its original order"
    );
    assert_eq!(player.snapshot().current_index, Some(0));
    assert!(!player.snapshot().shuffle);
}

#[test]
fn turning_shuffle_on_again_reshuffles_the_remaining_entries() {
    let temp = TempDir::new().unwrap();
    let (paths_in_order, player, _sink) = four_track_queue(&temp);
    let tail: HashSet<PathBuf> = paths_in_order[1..].iter().cloned().collect();

    let first = paths(&player.set_shuffle(true).queue);
    assert_eq!(first[0], paths_in_order[0]);
    assert_eq!(first[1..].iter().cloned().collect::<HashSet<_>>(), tail);

    player.set_shuffle(false);
    let second = paths(&player.set_shuffle(true).queue);
    assert_eq!(second[0], paths_in_order[0], "the current entry is still anchored");
    assert_eq!(
        second[1..].iter().cloned().collect::<HashSet<_>>(),
        tail,
        "the remaining entries are again a permutation of the original tail"
    );
}

#[test]
fn repeat_one_replays_the_current_entry_when_it_ends() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    player.set_repeat(RepeatMode::One);

    let mut reached_repeat = false;
    for _ in 0..40 {
        sink.pull(1_000);
        let snapshot = player.snapshot();
        if snapshot.current_index == Some(1) {
            // The first track should replay under Repeat One instead of advancing.
            reached_repeat = true;
            break;
        }
    }
    assert!(
        !reached_repeat,
        "Repeat One must keep the current entry instead of advancing to the next"
    );
    let after = player.snapshot();
    assert_eq!(after.current_index, Some(0));
    assert!(after.playing);
}

#[test]
fn repeat_all_wraps_from_the_last_entry_to_the_first() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    player.set_repeat(RepeatMode::All);
    // Walk to the last entry.
    for _ in 0..40 {
        sink.pull(1_000);
        if player.snapshot().current_index == Some(1) {
            break;
        }
    }
    assert_eq!(player.snapshot().current_index, Some(1));

    // The last entry ending must wrap to the first.
    let mut wrapped = false;
    for _ in 0..60 {
        sink.pull(1_000);
        let snapshot = player.snapshot();
        if snapshot.current_index == Some(0) {
            wrapped = true;
            break;
        }
    }
    let after = player.snapshot();
    assert!(wrapped, "Repeat All must wrap the last entry back to the first");
    assert_eq!(after.current_index, Some(0));
    assert!(after.playing);
}

#[test]
fn repeat_off_stops_after_the_last_entry_and_leaves_the_queue_in_place() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    player.set_repeat(RepeatMode::Off);

    let mut ended = false;
    for _ in 0..80 {
        sink.pull(1_000);
        if !player.snapshot().playing {
            ended = true;
            break;
        }
    }
    let after = player.snapshot();
    assert!(ended, "Repeat Off must stop the queue once it reaches the end");
    assert!(!after.playing);
    assert_eq!(after.current_index, None);
    assert_eq!(paths(&after.queue), vec![first, second], "the queue is left in place");
}

#[test]
fn play_after_the_queue_has_ended_starts_the_queue_from_the_top() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();

    // Walk until the queue ends under the default Repeat Off.
    for _ in 0..80 {
        sink.pull(1_000);
        if !player.snapshot().playing {
            break;
        }
    }
    assert!(!player.snapshot().playing);

    let resumed = player.play();
    assert_eq!(resumed.current_index, Some(0));
    assert!(resumed.playing);
    assert_eq!(paths(&resumed.queue), vec![first, second]);
}

#[test]
fn previous_restarts_the_current_entry_after_a_few_seconds_have_played() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    // Bring the restart threshold well within the short fixture so the test
    // can cross it without playing for real seconds.
    player.set_previous_restart_threshold_for_testing(10);
    sink.pull(200);

    let after = player.previous();
    assert_eq!(
        after.current_index,
        Some(0),
        "Previous past the threshold restarts the current entry rather than skipping back"
    );
    assert!(after.playing);
}

#[test]
fn previous_goes_to_the_prior_entry_when_within_the_threshold() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    for _ in 0..40 {
        sink.pull(1_000);
        if player.snapshot().current_index == Some(1) {
            break;
        }
    }
    assert_eq!(player.snapshot().current_index, Some(1));

    let after = player.previous();
    assert_eq!(after.current_index, Some(0));
    assert!(after.playing);
}

#[test]
fn previous_at_the_head_restarts_the_first_entry() {
    let temp = TempDir::new().unwrap();
    let album = temp.path().join("Album");
    let first = audio_file(&album, "first", "mp3");
    let second = audio_file(&album, "second", "flac");
    let sink = FakeOutputSink::new(44_100, 2);
    let player = Player::open(temp.path().join("library.db"), sink.clone()).unwrap();
    player
        .play_entries(vec![entry(first.clone(), "First"), entry(second.clone(), "Second")], &first)
        .unwrap();
    assert_eq!(player.snapshot().current_index, Some(0));

    let after = player.previous();
    assert_eq!(after.current_index, Some(0));
    assert!(after.playing);
}
/// Elapsed milliseconds the player reports after `frames` frames at 44.1 kHz,
/// the same arithmetic the player uses.
fn elapsed_ms(frames: u64) -> u64 {
    frames * 1_000 / 44_100
}

/// A queue entry whose tagged duration is long enough to seek within.
fn long_entry(path: PathBuf, title: &str) -> QueueEntry {
    QueueEntry {
        duration_ms: 5_000,
        ..entry(path, title)
    }
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
