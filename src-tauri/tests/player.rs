use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use linkamp_lib::library::{LibraryIndex, Track};
use linkamp_lib::player::{FakeOutputSink, NoDeviceOutputSink, Player, QueueEntry, RepeatMode};
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
