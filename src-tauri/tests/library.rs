use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use image::{ImageBuffer, ImageFormat, Rgb};
use linkamp_lib::library::LibraryIndex;
use lofty::config::WriteOptions;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::{Accessor, TagExt};
use lofty::tag::{ItemKey, Tag, TagType};
use tempfile::TempDir;

#[derive(Default)]
struct Tags<'a> {
    title: Option<&'a str>,
    artist: Option<&'a str>,
    album: Option<&'a str>,
    genre: Option<&'a str>,
    year: Option<&'a str>,
    track: Option<u32>,
}

fn fixture_path(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("silence.{extension}"))
}

fn audio_file(path: &Path, tags: Tags<'_>) {
    audio_file_with_pictures(path, tags, &[]);
}

fn audio_file_with_pictures(path: &Path, tags: Tags<'_>, pictures: &[(PictureType, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let extension = path
        .extension()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    fs::copy(fixture_path(&extension), path).unwrap();

    let tag_type = match extension.as_str() {
        "mp3" => TagType::Id3v2,
        "flac" => TagType::VorbisComments,
        _ => unreachable!(),
    };
    let mut tag = Tag::new(tag_type);
    if let Some(value) = tags.title {
        tag.set_title(value.to_owned());
    }
    if let Some(value) = tags.artist {
        tag.set_artist(value.to_owned());
    }
    if let Some(value) = tags.album {
        tag.set_album(value.to_owned());
    }
    if let Some(value) = tags.genre {
        tag.set_genre(value.to_owned());
    }
    if let Some(value) = tags.year {
        tag.insert_text(ItemKey::RecordingDate, value.to_owned());
    }
    if let Some(value) = tags.track {
        tag.set_track(value);
    }
    for (picture_type, bytes) in pictures {
        let picture = Picture::unchecked(bytes.to_vec())
            .pic_type(*picture_type)
            .build();
        tag.push_picture(picture);
    }
    if !tag.is_empty() {
        tag.save_to_path(path, WriteOptions::new()).unwrap();
    }
}

fn solid_png(red: u8, green: u8, blue: u8, size: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(size, size, |_, _| Rgb([red, green, blue]));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn solid_jpeg(red: u8, green: u8, blue: u8, size: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(size, size, |_, _| Rgb([red, green, blue]));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
        .unwrap();
    bytes
}

fn first_album_id(index: &LibraryIndex) -> String {
    index.snapshot().unwrap().albums[0].id.clone()
}

fn first_cover(index: &LibraryIndex) -> PathBuf {
    let album_id = first_album_id(index);
    index
        .album_cover(&album_id)
        .unwrap()
        .expect("album has a cover")
        .path
}

fn index_for(temp: &TempDir, music: &Path) -> LibraryIndex {
    LibraryIndex::open(
        temp.path().join("library.db"),
        temp.path().join("covers"),
        Some(music),
    )
    .unwrap()
}

#[test]
fn indexes_real_mp3_and_flac_files_through_the_public_index() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Album/one.mp3"),
        Tags {
            title: Some("One"),
            artist: Some("An Artist"),
            album: Some("Both Formats"),
            genre: Some("Rock"),
            year: Some("2024"),
            track: Some(1),
        },
    );
    audio_file(
        &music.join("Album/two.FLAC"),
        Tags {
            title: Some("Two"),
            artist: Some("An Artist"),
            album: Some("Both Formats"),
            genre: Some("Rock"),
            year: Some("2024-02-03"),
            track: Some(2),
        },
    );

    let index = index_for(&temp, &music);
    let result = index.scan().unwrap();
    let snapshot = index.snapshot().unwrap();

    assert_eq!(result.track_count, 2);
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.albums[0].name, "Both Formats");
    assert_eq!(snapshot.albums[0].year, Some(2024));

    let tracks = index.tracks_for_album(&snapshot.albums[0].id).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["One", "Two"]
    );
    assert!(tracks.iter().all(|track| track.duration_ms > 0));
    assert_eq!(tracks[0].artist.as_deref(), Some("An Artist"));
    assert_eq!(tracks[0].album, "Both Formats");
    assert_eq!(tracks[0].genre.as_deref(), Some("Rock"));
    assert_eq!(tracks[0].year, Some(2024));
    assert_eq!(tracks[0].track_number, Some(1));
    assert_eq!(
        tracks[0].path,
        music.join("Album/one.mp3").canonicalize().unwrap()
    );
}

#[test]
fn ignores_non_audio_hidden_entries_and_symlinked_directories_and_deduplicates_paths() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Visible");
    let original = album.join("song.mp3");
    audio_file(
        &original,
        Tags {
            title: Some("Visible song"),
            ..Tags::default()
        },
    );
    fs::write(album.join("cover.jpg"), b"not audio").unwrap();
    fs::write(album.join("list.m3u"), b"song.mp3\n").unwrap();
    fs::write(album.join("renamed.mp3"), b"not an MP3").unwrap();
    audio_file(
        &album.join(".hidden.flac"),
        Tags {
            title: Some("Hidden file"),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join(".hidden/track.mp3"),
        Tags {
            title: Some("Hidden directory"),
            ..Tags::default()
        },
    );

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&original, album.join("duplicate.MP3")).unwrap();
        let outside = temp.path().join("outside");
        audio_file(
            &outside.join("secret.mp3"),
            Tags {
                title: Some("Outside symlink"),
                ..Tags::default()
            },
        );
        std::os::unix::fs::symlink(&outside, music.join("linked-directory")).unwrap();
    }

    let index = index_for(&temp, &music);
    let result = index.scan().unwrap();
    let albums = index.snapshot().unwrap().albums;
    let tracks = index.tracks_for_album(&albums[0].id).unwrap();

    assert_eq!(result.track_count, 1);
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Visible song");
}

#[test]
fn containing_directories_are_album_identity_for_discs_and_loose_files() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("loose.mp3"),
        Tags {
            title: Some("Loose"),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("Release/Disc 1/one.mp3"),
        Tags {
            title: Some("Disc one"),
            album: Some("Release"),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("Release/Disc 2/two.flac"),
        Tags {
            title: Some("Disc two"),
            album: Some("Release"),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let albums = index.snapshot().unwrap().albums;
    let ids = albums
        .iter()
        .map(|album| PathBuf::from(&album.id))
        .collect::<Vec<_>>();

    assert_eq!(albums.len(), 3);
    assert!(ids.contains(&music.canonicalize().unwrap()));
    assert!(ids.contains(&music.join("Release/Disc 1").canonicalize().unwrap()));
    assert!(ids.contains(&music.join("Release/Disc 2").canonicalize().unwrap()));
    assert_eq!(
        albums
            .iter()
            .filter(|album| album.name == "Release")
            .count(),
        2
    );
    assert_eq!(
        albums
            .iter()
            .find(|album| album.id == music.canonicalize().unwrap().to_string_lossy())
            .unwrap()
            .name,
        "Music"
    );
}

#[test]
fn derives_album_metadata_and_orders_tracks_without_changing_tag_text() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("folder-name");
    for (file, title, album_tag, year, track) in [
        ("z.mp3", "Zweiter", "Über den Wolken", "2020-01-02", Some(1)),
        ("a.flac", "Äpfel", "Über den Wolken", "2020", Some(1)),
        ("b.mp3", "Bonus", "Other", "2021", None),
        ("c.mp3", "Anhang", "Über den Wolken", "2020-12", None),
    ] {
        audio_file(
            &album.join(file),
            Tags {
                title: Some(title),
                artist: Some("Die Ärzte"),
                album: Some(album_tag),
                genre: Some("Deutschrock"),
                year: Some(year),
                track,
            },
        );
    }
    audio_file(
        &music.join("Ohne Albumtag/plain.flac"),
        Tags {
            title: Some("Plain"),
            year: Some("n/a"),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let albums = index.snapshot().unwrap().albums;
    let tagged_album = albums
        .iter()
        .find(|entry| entry.name == "Über den Wolken")
        .unwrap();
    let fallback_album = albums
        .iter()
        .find(|entry| entry.name == "Ohne Albumtag")
        .unwrap();

    assert_eq!(tagged_album.year, Some(2020));
    assert_eq!(fallback_album.year, None);
    let tracks = index.tracks_for_album(&tagged_album.id).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["Zweiter", "Äpfel", "Anhang", "Bonus"]
    );
    assert!(
        tracks
            .iter()
            .all(|track| track.artist.as_deref() == Some("Die Ärzte"))
    );
    assert!(tracks.iter().all(|track| track.album == "Über den Wolken"));
}

#[test]
fn artists_group_by_track_artist_without_splitting_or_replacing() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Bravo Hits/dance-monkey.mp3"),
        Tags {
            title: Some("Dance Monkey"),
            artist: Some("Tones and I"),
            album: Some("Bravo Hits"),
            genre: Some("Pop"),
            year: Some("2019"),
            track: Some(3),
        },
    );
    audio_file(
        &music.join("Bravo Hits/collab.flac"),
        Tags {
            title: Some("Collab"),
            artist: Some("Alpha feat. Beta"),
            album: Some("Bravo Hits"),
            genre: Some("Pop"),
            year: Some("2019"),
            track: Some(4),
        },
    );
    audio_file(
        &music.join("The Kids Are Coming/title-track.mp3"),
        Tags {
            title: Some("The Kids Are Coming"),
            artist: Some("Tones and I"),
            album: Some("The Kids Are Coming"),
            genre: Some("Pop"),
            year: Some("2019"),
            track: Some(1),
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let snapshot = index.snapshot().unwrap();

    // One entry per artist tag: no featured-artist split, no album grouping.
    assert_eq!(
        snapshot
            .artists
            .iter()
            .map(|artist| artist.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Alpha feat. Beta"), Some("Tones and I")]
    );

    // The artist expands to every album containing at least one of its tracks.
    let tones = &snapshot.artists[1];
    assert_eq!(tones.track_count, 2);
    assert_eq!(
        tones
            .albums
            .iter()
            .map(|album| album.name.as_str())
            .collect::<Vec<_>>(),
        ["Bravo Hits", "The Kids Are Coming"]
    );

    // The compilation track stays under its own artist, in album order.
    let tracks = index.tracks_for_artist(Some("Tones and I")).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["Dance Monkey", "The Kids Are Coming"]
    );
    assert!(
        tracks
            .iter()
            .all(|track| track.artist.as_deref() == Some("Tones and I"))
    );
    assert_eq!(tracks[0].album, "Bravo Hits");

    let collab = index.tracks_for_artist(Some("Alpha feat. Beta")).unwrap();
    assert_eq!(collab.len(), 1);
    assert_eq!(collab[0].title, "Collab");
}

#[test]
fn missing_artist_genre_and_year_group_under_unknown_and_unknown_sorts_last() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Known/one.mp3"),
        Tags {
            title: Some("Zebra Song"),
            artist: Some("zebra"),
            album: Some("Known"),
            genre: Some("Rock"),
            year: Some("2001"),
            track: Some(1),
        },
    );
    audio_file(
        &music.join("Known/two.flac"),
        Tags {
            title: Some("Apple Song"),
            artist: Some("Apple"),
            album: Some("Known"),
            genre: Some("indie"),
            year: Some("1999"),
            track: Some(2),
        },
    );
    audio_file(
        &music.join("Bare/junk-year.mp3"),
        Tags {
            title: Some("Junk Year"),
            year: Some("n/a"),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("Bare/no-tags.flac"),
        Tags {
            title: Some("No Tags"),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let snapshot = index.snapshot().unwrap();

    // Ascending, case-insensitive, with Unknown last.
    assert_eq!(
        snapshot
            .artists
            .iter()
            .map(|artist| artist.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Apple"), Some("zebra"), None]
    );
    assert_eq!(
        snapshot
            .genres
            .iter()
            .map(|genre| genre.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("indie"), Some("Rock"), None]
    );
    // Newest first, Unknown last; "n/a" and a missing year share the bucket.
    assert_eq!(
        snapshot
            .years
            .iter()
            .map(|year| (year.year, year.track_count))
            .collect::<Vec<_>>(),
        [(Some(2001), 1), (Some(1999), 1), (None, 2)]
    );

    let unknown_artists = index.tracks_for_artist(None).unwrap();
    assert_eq!(
        unknown_artists
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["Junk Year", "No Tags"]
    );
    assert!(unknown_artists.iter().all(|track| track.album == "Bare"));
    assert_eq!(index.tracks_for_genre(None).unwrap().len(), 2);
    assert_eq!(index.tracks_for_year(None).unwrap().len(), 2);

    // Unknown tracks still appear under their album.
    let bare = snapshot
        .albums
        .iter()
        .find(|album| album.name == "Bare")
        .unwrap();
    let album_tracks = index.tracks_for_album(&bare.id).unwrap();
    assert_eq!(
        album_tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["Junk Year", "No Tags"]
    );
}

#[test]
fn genre_and_year_lists_hold_matching_tracks_in_album_order() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    for (file, title, track) in [
        ("second.mp3", "Second", Some(2)),
        ("first.mp3", "First", Some(1)),
        ("extra.mp3", "Extra", None),
    ] {
        audio_file(
            &music.join("B Side").join(file),
            Tags {
                title: Some(title),
                artist: Some("Shared Artist"),
                album: Some("B Side"),
                genre: Some("Rock"),
                year: Some("2020-05-06"),
                track,
            },
        );
    }
    audio_file(
        &music.join("A Side/opening.flac"),
        Tags {
            title: Some("Opening"),
            artist: Some("Other Artist"),
            album: Some("A Side"),
            genre: Some("Rock"),
            year: Some("2020"),
            track: Some(1),
        },
    );
    audio_file(
        &music.join("A Side/jazz.flac"),
        Tags {
            title: Some("Jazz Piece"),
            artist: Some("Other Artist"),
            album: Some("A Side"),
            genre: Some("Jazz"),
            year: Some("1965"),
            track: Some(2),
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let snapshot = index.snapshot().unwrap();

    assert_eq!(
        snapshot
            .genres
            .iter()
            .map(|genre| (genre.name.as_deref(), genre.track_count))
            .collect::<Vec<_>>(),
        [(Some("Jazz"), 1), (Some("Rock"), 4)]
    );
    // A date counts as its leading four-digit year.
    assert_eq!(
        snapshot
            .years
            .iter()
            .map(|year| (year.year, year.track_count))
            .collect::<Vec<_>>(),
        [(Some(2020), 4), (Some(1965), 1)]
    );

    // Album display name, then track number, then title; unnumbered last.
    let expected = ["Opening", "First", "Second", "Extra"];
    let rock = index.tracks_for_genre(Some("Rock")).unwrap();
    assert_eq!(
        rock.iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    let twenty_twenty = index.tracks_for_year(Some(2020)).unwrap();
    assert_eq!(
        twenty_twenty
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        index.tracks_for_genre(Some("Jazz")).unwrap()[0].title,
        "Jazz Piece"
    );
    assert_eq!(
        index.tracks_for_year(Some(1965)).unwrap()[0].title,
        "Jazz Piece"
    );
}

#[test]
fn episode_directory_is_an_album_under_its_series_artist() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Die drei Fragezeichen/Folge 1/episode.mp3"),
        Tags {
            title: Some("Der Super-Papagei"),
            artist: Some("Die drei Fragezeichen"),
            genre: Some("Hörspiel"),
            year: Some("1979"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let snapshot = index.snapshot().unwrap();

    // The episode directory is an ordinary album; there is no extra section.
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.albums[0].name, "Folge 1");
    assert_eq!(snapshot.artists.len(), 1);
    let series = &snapshot.artists[0];
    assert_eq!(series.name.as_deref(), Some("Die drei Fragezeichen"));
    assert_eq!(
        series
            .albums
            .iter()
            .map(|album| album.id.as_str())
            .collect::<Vec<_>>(),
        [snapshot.albums[0].id.as_str()]
    );

    let tracks = index
        .tracks_for_artist(Some("Die drei Fragezeichen"))
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Der Super-Papagei");
    assert_eq!(tracks[0].album, "Folge 1");
}

#[test]
fn default_music_folder_is_considered_only_on_the_first_open() {
    let temp = TempDir::new().unwrap();
    let existing_music = temp.path().join("existing/Music");
    fs::create_dir_all(&existing_music).unwrap();
    let database = temp.path().join("existing.db");

    {
        let index =
            LibraryIndex::open(&database, temp.path().join("covers"), Some(&existing_music))
                .unwrap();
        assert_eq!(
            index.watched_folders().unwrap(),
            [existing_music.canonicalize().unwrap()]
        );
        assert!(index.remove_watched_folder(&existing_music).unwrap());
    }
    let reopened =
        LibraryIndex::open(&database, temp.path().join("covers"), Some(&existing_music)).unwrap();
    assert!(reopened.watched_folders().unwrap().is_empty());

    let missing_music = temp.path().join("missing/Music");
    let missing_database = temp.path().join("missing.db");
    {
        let index = LibraryIndex::open(
            &missing_database,
            temp.path().join("covers"),
            Some(&missing_music),
        )
        .unwrap();
        assert!(index.watched_folders().unwrap().is_empty());
        assert!(!missing_music.exists());
    }
    fs::create_dir_all(&missing_music).unwrap();
    let reopened = LibraryIndex::open(
        &missing_database,
        temp.path().join("covers"),
        Some(&missing_music),
    )
    .unwrap();
    assert!(reopened.watched_folders().unwrap().is_empty());
}

#[test]
fn adding_a_duplicate_canonical_folder_is_a_no_op() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    fs::create_dir_all(&music).unwrap();
    let index = LibraryIndex::open(
        temp.path().join("library.db"),
        temp.path().join("covers"),
        None,
    )
    .unwrap();

    assert!(index.add_watched_folder(&music).unwrap());
    assert!(!index.add_watched_folder(&music.join(".")).unwrap());
    assert_eq!(
        index.watched_folders().unwrap(),
        [music.canonicalize().unwrap()]
    );
}

#[test]
fn adding_a_nested_folder_is_refused_as_already_covered() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let nested = music.join("Artist/Album");
    fs::create_dir_all(&nested).unwrap();
    let index = LibraryIndex::open(
        temp.path().join("library.db"),
        temp.path().join("covers"),
        None,
    )
    .unwrap();

    assert!(index.add_watched_folder(&music).unwrap());
    let error = index.add_watched_folder(&nested).unwrap_err();

    assert!(error.to_string().contains("already covered"));
    assert_eq!(
        index.watched_folders().unwrap(),
        [music.canonicalize().unwrap()]
    );
}

#[test]
fn adding_a_parent_replaces_nested_watched_folders() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let first = music.join("First");
    let second = music.join("Second");
    audio_file(
        &first.join("one.mp3"),
        Tags {
            title: Some("One"),
            ..Tags::default()
        },
    );
    audio_file(
        &second.join("two.flac"),
        Tags {
            title: Some("Two"),
            ..Tags::default()
        },
    );
    let index = LibraryIndex::open(
        temp.path().join("library.db"),
        temp.path().join("covers"),
        None,
    )
    .unwrap();

    assert!(index.add_watched_folder(&first).unwrap());
    assert!(index.add_watched_folder(&second).unwrap());
    assert_eq!(index.scan().unwrap().track_count, 2);

    assert!(index.add_watched_folder(&music).unwrap());
    assert_eq!(
        index.watched_folders().unwrap(),
        [music.canonicalize().unwrap()]
    );
    // Replacing the nested rows drops their indexed tracks through the
    // foreign key. Scanning the parent indexes those same files once.
    assert!(index.snapshot().unwrap().albums.is_empty());
    assert_eq!(index.scan().unwrap().track_count, 2);
}

#[test]
fn removing_a_watched_folder_drops_tracks_but_leaves_file_bytes() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let path = music.join("Album/song.flac");
    audio_file(
        &path,
        Tags {
            title: Some("Keep Me"),
            artist: Some("Still Here"),
            album: Some("On Disk"),
            genre: Some("Rock"),
            year: Some("2024"),
            ..Tags::default()
        },
    );
    let bytes_before = fs::read(&path).unwrap();
    let index = index_for(&temp, &music);
    assert_eq!(index.scan().unwrap().track_count, 1);

    assert!(index.remove_watched_folder(&music).unwrap());

    let snapshot = index.snapshot().unwrap();
    assert!(snapshot.albums.is_empty());
    assert!(snapshot.artists.is_empty());
    assert!(snapshot.genres.is_empty());
    assert!(snapshot.years.is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes_before);
}

#[test]
fn scan_reads_fixtures_in_place_without_modifying_them() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let path = music.join("Album/song.flac");
    audio_file(
        &path,
        Tags {
            title: Some("Unchanged"),
            album: Some("Read Only"),
            ..Tags::default()
        },
    );
    let bytes_before = fs::read(&path).unwrap();
    let modified_before = fs::metadata(&path).unwrap().modified().unwrap();

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    assert_eq!(fs::read(&path).unwrap(), bytes_before);
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        modified_before
    );
    assert!(path.exists());
}

#[test]
fn rescan_keeps_file_whose_size_and_mtime_were_restored_despite_new_bytes() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let path = music.join("Album/song.mp3");
    audio_file(
        &path,
        Tags {
            title: Some("A Fairly Long Original Title"),
            artist: Some("Original Artist"),
            album: Some("Original Album"),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    // Rewrite the file with a different tag, then pad it back to the stored
    // size and restore the stored modification time. A rescan must trust
    // that fingerprint and skip the tag read, so the old title remains.
    let metadata = fs::metadata(&path).unwrap();
    let original_len = metadata.len();
    let original_modified = metadata.modified().unwrap();
    audio_file(
        &path,
        Tags {
            title: Some("X"),
            artist: Some("Different Artist"),
            album: Some("Different Album"),
            ..Tags::default()
        },
    );
    let new_len = fs::metadata(&path).unwrap().len();
    assert!(new_len <= original_len, "replacement file must not grow");
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&vec![0u8; (original_len - new_len) as usize])
        .unwrap();
    drop(file);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_modified)
        .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        original_modified,
        "filesystem cannot restore modification times exactly"
    );

    index.scan().unwrap();

    let albums = index.snapshot().unwrap().albums;
    let tracks = index.tracks_for_album(&albums[0].id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "A Fairly Long Original Title");
    assert_eq!(tracks[0].artist.as_deref(), Some("Original Artist"));
}

#[test]
fn rescan_rereads_file_when_mtime_and_tags_change_and_regroups_it() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let path = music.join("Release/song.flac");
    audio_file(
        &path,
        Tags {
            title: Some("Old Title"),
            artist: Some("Old Artist"),
            album: Some("Old Album"),
            genre: Some("Old Genre"),
            year: Some("2001"),
            track: Some(1),
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    // Retag the file. The write changes the modification time, so the next
    // pass re-reads it and the track moves to its new groupings.
    audio_file(
        &path,
        Tags {
            title: Some("New Title"),
            artist: Some("New Artist"),
            album: Some("New Album"),
            genre: Some("New Genre"),
            year: Some("2002"),
            track: Some(1),
        },
    );
    index.scan().unwrap();

    let snapshot = index.snapshot().unwrap();
    assert_eq!(
        snapshot
            .artists
            .iter()
            .map(|artist| artist.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("New Artist")]
    );
    assert_eq!(
        snapshot
            .genres
            .iter()
            .map(|genre| genre.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("New Genre")]
    );
    assert_eq!(
        snapshot
            .years
            .iter()
            .map(|year| year.year)
            .collect::<Vec<_>>(),
        [Some(2002)]
    );
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.albums[0].name, "New Album");
    assert_eq!(snapshot.albums[0].year, Some(2002));

    let tracks = index.tracks_for_album(&snapshot.albums[0].id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "New Title");
    assert!(index.search("Old Title").unwrap().is_empty());
}

#[test]
fn rescan_adds_an_audio_file_that_appeared() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Album/one.mp3"),
        Tags {
            title: Some("One"),
            album: Some("Album"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    assert_eq!(index.scan().unwrap().track_count, 1);

    audio_file(
        &music.join("Album/two.flac"),
        Tags {
            title: Some("Two"),
            album: Some("Album"),
            track: Some(2),
            ..Tags::default()
        },
    );
    assert_eq!(index.scan().unwrap().track_count, 2);

    let albums = index.snapshot().unwrap().albums;
    let tracks = index.tracks_for_album(&albums[0].id).unwrap();
    let titles: Vec<&str> = tracks.iter().map(|track| track.title.as_str()).collect();
    assert_eq!(titles, ["One", "Two"]);
}

#[test]
fn rescan_drops_a_deleted_file_from_every_view_without_a_missing_row() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let doomed = music.join("Doomed/song.mp3");
    audio_file(
        &doomed,
        Tags {
            title: Some("Doomed"),
            artist: Some("Leaving Artist"),
            album: Some("Doomed"),
            genre: Some("Leaving Genre"),
            year: Some("1999"),
            track: Some(1),
        },
    );
    audio_file(
        &music.join("Staying/song.flac"),
        Tags {
            title: Some("Staying"),
            artist: Some("Staying Artist"),
            album: Some("Staying"),
            genre: Some("Staying Genre"),
            year: Some("2024"),
            track: Some(1),
        },
    );

    let index = index_for(&temp, &music);
    assert_eq!(index.scan().unwrap().track_count, 2);
    let doomed_album = index
        .snapshot()
        .unwrap()
        .albums
        .into_iter()
        .find(|album| album.name == "Doomed")
        .unwrap();

    fs::remove_file(&doomed).unwrap();
    assert_eq!(index.scan().unwrap().track_count, 1);

    let snapshot = index.snapshot().unwrap();
    assert_eq!(
        snapshot
            .albums
            .iter()
            .map(|album| album.name.as_str())
            .collect::<Vec<_>>(),
        ["Staying"]
    );
    assert_eq!(
        snapshot
            .artists
            .iter()
            .map(|artist| artist.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Staying Artist")]
    );
    assert_eq!(
        snapshot
            .genres
            .iter()
            .map(|genre| genre.name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Staying Genre")]
    );
    assert_eq!(
        snapshot
            .years
            .iter()
            .map(|year| year.year)
            .collect::<Vec<_>>(),
        [Some(2024)]
    );
    // The deleted file is not kept as a missing row anywhere.
    assert!(index.search("Doomed").unwrap().is_empty());
    assert!(index.tracks_for_album(&doomed_album.id).is_err());
}

#[test]
fn rescan_treats_a_rename_as_a_removed_path_plus_a_new_path() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let old_path = music.join("Album/old-name.mp3");
    audio_file(
        &old_path,
        Tags {
            title: Some("Renamed Song"),
            artist: Some("An Artist"),
            album: Some("Album"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    let old_canonical = old_path.canonicalize().unwrap();
    let new_path = music.join("Album/new-name.mp3");
    fs::rename(&old_path, &new_path).unwrap();
    index.scan().unwrap();

    let albums = index.snapshot().unwrap().albums;
    let tracks = index.tracks_for_album(&albums[0].id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Renamed Song");
    assert_eq!(tracks[0].path, new_path.canonicalize().unwrap());
    assert!(
        index
            .search("Renamed Song")
            .unwrap()
            .iter()
            .all(|track| track.path != old_canonical)
    );
}

#[test]
fn rescan_keeps_tracks_of_a_watched_folder_that_cannot_be_opened() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Album/song.mp3"),
        Tags {
            title: Some("Surviving Song"),
            artist: Some("Surviving Artist"),
            album: Some("Surviving Album"),
            genre: Some("Surviving Genre"),
            year: Some("2024"),
            track: Some(1),
        },
    );

    let index = index_for(&temp, &music);
    assert_eq!(index.scan().unwrap().track_count, 1);

    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&music, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&music).is_ok() {
        // Elevated test processes bypass permission checks; there is nothing
        // to assert. Restore permissions so the temp directory can be deleted.
        fs::set_permissions(&music, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    // The failed walk must not wipe the folder's existing tracks.
    assert_eq!(index.scan().unwrap().track_count, 1);
    let snapshot = index.snapshot().unwrap();
    let albums: Vec<String> = snapshot
        .albums
        .iter()
        .map(|album| album.name.clone())
        .collect();
    let artists: Vec<Option<String>> = snapshot
        .artists
        .iter()
        .map(|artist| artist.name.clone())
        .collect();
    let genres: Vec<Option<String>> = snapshot
        .genres
        .iter()
        .map(|genre| genre.name.clone())
        .collect();
    let years: Vec<Option<u32>> = snapshot.years.iter().map(|year| year.year).collect();

    // Restore permissions before any assertion and before the temp
    // directory is deleted.
    fs::set_permissions(&music, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(albums, ["Surviving Album"]);
    assert_eq!(artists, [Some("Surviving Artist".to_owned())]);
    assert_eq!(genres, [Some("Surviving Genre".to_owned())]);
    assert_eq!(years, [Some(2024)]);
}

#[test]
fn reopened_index_shows_the_saved_library_before_and_after_a_rescan() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Album/song.mp3"),
        Tags {
            title: Some("Persisted"),
            artist: Some("An Artist"),
            album: Some("Persisted Album"),
            genre: Some("Rock"),
            year: Some("2024"),
            track: Some(1),
        },
    );
    let database = temp.path().join("library.db");

    let album_id = {
        let index = LibraryIndex::open(&database, temp.path().join("covers"), Some(&music))
            .unwrap();
        index.scan().unwrap();
        index.snapshot().unwrap().albums[0].id.clone()
    };

    // A restart reads the saved library at once, before any scan runs.
    let reopened = LibraryIndex::open(&database, temp.path().join("covers"), Some(&music))
        .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.albums[0].name, "Persisted Album");
    assert_eq!(snapshot.artists.len(), 1);
    assert_eq!(snapshot.genres.len(), 1);
    assert_eq!(snapshot.years.len(), 1);

    // A pass over the unchanged files keeps the same identities.
    reopened.scan().unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.albums[0].id, album_id);
    let tracks = reopened.tracks_for_album(&album_id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Persisted");
}

#[test]
fn search_matches_title_artist_and_album_case_insensitively() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("alpha/track.mp3"),
        Tags {
            title: Some("Sentinel"),
            artist: Some("Indigo Pulse"),
            album: Some("Disc A"),
            track: Some(1),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("bravo/track.mp3"),
        Tags {
            title: Some("Sondrel"),
            artist: Some("Bravo Echo"),
            album: Some("Disc B"),
            track: Some(1),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("charlie/track.mp3"),
        Tags {
            title: Some("Standback"),
            artist: Some("Charlie Foxtrot"),
            album: Some("Favourite"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    let titles = |hits: &[linkamp_lib::library::Track]| -> Vec<String> {
        hits.iter().map(|track| track.title.clone()).collect()
    };

    // Title hit, case-insensitive.
    assert_eq!(titles(&index.search("sentinel").unwrap()), ["Sentinel"]);
    assert_eq!(titles(&index.search("ENTINEL").unwrap()), ["Sentinel"]);
    // Substring of title.
    assert_eq!(titles(&index.search("ntin").unwrap()), ["Sentinel"]);

    // Artist hit.
    assert_eq!(titles(&index.search("bravo").unwrap()), ["Sondrel"]);
    assert_eq!(titles(&index.search("BraVo").unwrap()), ["Sondrel"]);

    // Album display name hit (the most common album tag, not the directory).
    assert_eq!(titles(&index.search("favourite").unwrap()), ["Standback"]);
    assert_eq!(titles(&index.search("OURIT").unwrap()), ["Standback"]);
}

#[test]
fn search_ignores_genre_year_and_file_name() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    // The track's title, artist, and album deliberately avoid "pop", "2019",
    // and "secret", so only genre, year, and file name could match.
    audio_file(
        &music.join("plain/track.mp3"),
        Tags {
            title: Some("Plain Track"),
            artist: Some("Plain Artist"),
            album: Some("Plain Album"),
            genre: Some("Pop"),
            year: Some("2019"),
            track: Some(1),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("untitled/secret.mp3"),
        Tags {
            title: Some("Untitled"),
            artist: Some("Anonymous"),
            album: Some("Unknown"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    assert!(index.search("pop").unwrap().is_empty());
    assert!(index.search("2019").unwrap().is_empty());
    assert!(index.search("secret").unwrap().is_empty());
}

#[test]
fn search_orders_by_artist_album_track_number_then_title() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("a/track.mp3"),
        Tags {
            title: Some("Sentinel"),
            artist: Some("Alpha"),
            album: Some("Disc A"),
            track: Some(1),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("b/track.mp3"),
        Tags {
            title: Some("Sondrel"),
            artist: Some("Alpha"),
            album: Some("Disc A"),
            track: Some(2),
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("c/track.mp3"),
        Tags {
            title: Some("Standback"),
            artist: Some("Alpha"),
            album: Some("Disc A"),
            track: None,
            ..Tags::default()
        },
    );
    audio_file(
        &music.join("d/track.mp3"),
        Tags {
            title: Some("Stardust"),
            artist: Some("Beta"),
            album: Some("Disc A"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    let titles: Vec<String> = index
        .search("S")
        .unwrap()
        .iter()
        .map(|track| track.title.clone())
        .collect();

    // Alpha before Beta; within Alpha: numbered ascending, then unnumbered.
    assert_eq!(titles, ["Sentinel", "Sondrel", "Standback", "Stardust"]);
}

#[test]
fn search_returns_no_results_for_empty_or_whitespace_query() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("track.mp3"),
        Tags {
            title: Some("Sentinel"),
            artist: Some("Indigo Pulse"),
            album: Some("Disc A"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    // Empty or whitespace-only queries carry no search restriction, so the
    // caller shows the selected node's tracks instead of search hits.
    assert!(index.search("").unwrap().is_empty());
    assert!(index.search(" ").unwrap().is_empty());
    assert!(index.search("   ").unwrap().is_empty());
    assert!(index.search("\t\n").unwrap().is_empty());
}

/// Builds a library with just the given picture as the album's only source
/// and returns the bytes of the resulting thumbnail.
fn thumbnail_for(picture_bytes: &[u8], as_picture: PictureType, extension: &str) -> Vec<u8> {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Album");
    audio_file_with_pictures(
        &album.join(format!("song.{extension}")),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
        &[(as_picture, picture_bytes)],
    );
    let index = index_for(&temp, &music);
    index.scan().unwrap();
    fs::read(first_cover(&index)).unwrap()
}

/// Builds a library whose only cover source is a sidecar JPG.
fn thumbnail_for_sidecar(jpeg_bytes: &[u8], name: &str) -> Vec<u8> {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Album");
    audio_file(
        &album.join("song.mp3"),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
    );
    fs::write(album.join(name), jpeg_bytes).unwrap();
    let index = index_for(&temp, &music);
    index.scan().unwrap();
    fs::read(first_cover(&index)).unwrap()
}

#[test]
fn embedded_front_picture_beats_sidecar_and_non_front_embedded() {
    let front_png = solid_png(255, 0, 0, 100);
    let back_png = solid_png(0, 255, 0, 100);
    let sidecar_jpg = solid_jpeg(0, 0, 255, 100);

    // Baseline: the embedded front picture on its own.
    let baseline = thumbnail_for(&front_png, PictureType::CoverFront, "mp3");

    // Album with an embedded front picture, an embedded non-front picture,
    // and a sidecar cover.jpg. The thumbnail must still come from the front.
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Album");
    audio_file_with_pictures(
        &album.join("front.mp3"),
        Tags {
            title: Some("Front"),
            track: Some(1),
            ..Tags::default()
        },
        &[(PictureType::CoverFront, &front_png)],
    );
    audio_file_with_pictures(
        &album.join("back.flac"),
        Tags {
            title: Some("Back"),
            track: Some(2),
            ..Tags::default()
        },
        &[(PictureType::CoverBack, &back_png)],
    );
    fs::write(album.join("cover.jpg"), &sidecar_jpg).unwrap();

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let combined = fs::read(first_cover(&index)).unwrap();

    assert_eq!(combined, baseline);

    // Sanity check: the back and sidecar thumbnails are visibly different
    // from the front, so the equality above is a real signal.
    let back_only = thumbnail_for(&back_png, PictureType::CoverBack, "mp3");
    let sidecar_only = thumbnail_for_sidecar(&sidecar_jpg, "cover.jpg");
    assert_ne!(combined, back_only);
    assert_ne!(combined, sidecar_only);
}

#[test]
fn sidecar_preference_is_cover_folder_front_then_others() {
    let cover_jpg = solid_jpeg(255, 0, 0, 100);
    let folder_jpg = solid_jpeg(0, 255, 0, 100);
    let front_jpg = solid_jpeg(0, 0, 255, 100);
    let other_jpg = solid_jpeg(255, 255, 0, 100);
    let other_upper = solid_jpeg(0, 255, 255, 100);

    let baseline_cover = thumbnail_for_sidecar(&cover_jpg, "cover.jpg");
    let baseline_folder = thumbnail_for_sidecar(&folder_jpg, "folder.jpg");
    let baseline_front = thumbnail_for_sidecar(&front_jpg, "front.jpg");
    let baseline_other = thumbnail_for_sidecar(&other_jpg, "other.jpg");

    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Album");
    audio_file(
        &album.join("song.mp3"),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
    );
    fs::write(album.join("cover.jpg"), &cover_jpg).unwrap();
    fs::write(album.join("folder.jpg"), &folder_jpg).unwrap();
    fs::write(album.join("front.jpg"), &front_jpg).unwrap();
    fs::write(album.join("other.jpg"), &other_jpg).unwrap();
    fs::write(album.join("EXTRA.JPEG"), &other_upper).unwrap();

    let index = index_for(&temp, &music);
    index.scan().unwrap();
    let combined = fs::read(first_cover(&index)).unwrap();

    assert_eq!(combined, baseline_cover);
    assert_ne!(combined, baseline_folder);
    assert_ne!(combined, baseline_front);
    assert_ne!(combined, baseline_other);
}

#[test]
fn cover_lookup_does_not_walk_to_parent_directories() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let parent = music.join("Parent");
    let child = parent.join("Child");
    fs::create_dir_all(&child).unwrap();
    audio_file(
        &child.join("song.mp3"),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
    );
    fs::write(parent.join("cover.jpg"), b"parent cover bytes").unwrap();
    fs::write(child.join("folder.jpg"), b"not a real jpeg").unwrap();

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    // The child's folder.jpg is not a valid JPEG, so decoding fails: the
    // album has no cover, even though the parent has a usable one.
    let album_id = index
        .snapshot()
        .unwrap()
        .albums
        .iter()
        .find(|album| album.id.contains("Child"))
        .unwrap()
        .id
        .clone();
    assert!(index.album_cover(&album_id).unwrap().is_none());

    // Likewise, the parent album still has no cover: it has no audio files.
    let parent_canonical = parent
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let parent_album = index
        .snapshot()
        .unwrap()
        .albums
        .iter()
        .find(|album| album.id == parent_canonical)
        .map(|album| album.id.clone());
    assert!(
        parent_album.is_none(),
        "parent directory should not be an album"
    );
}

#[test]
fn album_without_picture_has_no_cover() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    audio_file(
        &music.join("Album/song.mp3"),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    assert!(
        index
            .album_cover(&first_album_id(&index))
            .unwrap()
            .is_none()
    );
}

#[test]
fn replacing_the_source_picture_refreshes_the_thumbnail() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let album = music.join("Album");
    audio_file(
        &album.join("song.mp3"),
        Tags {
            title: Some("Song"),
            track: Some(1),
            ..Tags::default()
        },
    );

    let first_jpg = solid_jpeg(255, 0, 0, 100);
    let second_jpg = solid_jpeg(0, 0, 255, 100);
    fs::write(album.join("cover.jpg"), &first_jpg).unwrap();

    let index = index_for(&temp, &music);
    index.scan().unwrap();

    let album_id = first_album_id(&index);
    let cover_before = index.album_cover(&album_id).unwrap().expect("first cover");
    let bytes_before = fs::read(&cover_before.path).unwrap();

    // Replace the sidecar. The next cover lookup must regenerate the
    // thumbnail instead of returning the cached one.
    fs::write(album.join("cover.jpg"), &second_jpg).unwrap();

    let cover_after = index.album_cover(&album_id).unwrap().expect("second cover");
    let bytes_after = fs::read(&cover_after.path).unwrap();

    assert_eq!(cover_before.path, cover_after.path);
    assert_ne!(cover_before.source_hash, cover_after.source_hash);
    assert_ne!(bytes_before, bytes_after);

    // A second lookup with the same source should hit the cache and keep
    // the regenerated thumbnail in place.
    let cover_again = index.album_cover(&album_id).unwrap().expect("third cover");
    let bytes_again = fs::read(&cover_again.path).unwrap();
    assert_eq!(bytes_again, bytes_after);
}

#[test]
fn thumbnail_long_edge_is_256_pixels() {
    let cases = [
        ("wide source", solid_jpeg(255, 0, 0, 1024)),
        ("tall source", solid_jpeg(255, 0, 0, 256)),
        ("small source", solid_jpeg(255, 0, 0, 50)),
        ("sidecar wide", solid_jpeg(0, 255, 0, 1024)),
        ("sidecar tall", solid_jpeg(0, 255, 0, 256)),
    ];
    for (label, picture) in cases {
        let temp = TempDir::new().unwrap();
        let music = temp.path().join("Music");
        let album = music.join("Album");
        audio_file_with_pictures(
            &album.join("song.mp3"),
            Tags {
                title: Some("Song"),
                track: Some(1),
                ..Tags::default()
            },
            &[(PictureType::CoverFront, &picture)],
        );
        fs::write(album.join("cover.jpg"), &picture).unwrap();

        let index = index_for(&temp, &music);
        index.scan().unwrap();
        let cover_path = first_cover(&index);
        assert!(
            cover_path.starts_with(temp.path().join("covers")),
            "{label}: thumbnail escaped the cache: {}",
            cover_path.display()
        );
        let cover = fs::read(&cover_path).unwrap();
        let decoded = image::ImageReader::new(std::io::Cursor::new(&cover))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        let (width, height) = (decoded.width(), decoded.height());
        let long_edge = width.max(height);
        assert_eq!(long_edge, 256, "{label}: long edge {width}x{height}");
        // The shorter edge is non-zero and respects the source aspect ratio.
        assert!(width.min(height) > 0, "{label}: short edge zero");
    }
}

#[test]
fn rescan_keeps_going_when_a_subdirectory_cannot_be_opened() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    let open = music.join("Open");
    let locked = music.join("Locked");
    audio_file(
        &open.join("keep.mp3"),
        Tags {
            title: Some("Keep"),
            ..Tags::default()
        },
    );
    audio_file(
        &open.join("gone.mp3"),
        Tags {
            title: Some("Gone"),
            ..Tags::default()
        },
    );
    audio_file(
        &locked.join("hidden.mp3"),
        Tags {
            title: Some("Hidden"),
            ..Tags::default()
        },
    );

    let index = index_for(&temp, &music);
    assert_eq!(index.scan().unwrap().track_count, 3);

    fs::remove_file(open.join("gone.mp3")).unwrap();
    audio_file(
        &open.join("new.mp3"),
        Tags {
            title: Some("New"),
            ..Tags::default()
        },
    );

    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let locked_unreadable = fs::read_dir(&locked).is_err();
    let scan = index.scan();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    scan.unwrap();
    if !locked_unreadable {
        return;
    }

    let titles = index
        .search("e")
        .unwrap()
        .into_iter()
        .map(|track| track.title)
        .collect::<Vec<_>>();
    // "e" matches Keep, Hidden, and New, and must not match Gone.
    assert!(titles.contains(&"Keep".to_owned()));
    assert!(titles.contains(&"Hidden".to_owned()));
    assert!(titles.contains(&"New".to_owned()));
    assert!(!titles.iter().any(|title| title == "Gone"));
}

#[test]
fn overlapping_scans_do_not_drop_files_that_are_still_on_disk() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    for number in 0..12 {
        audio_file(
            &music.join(format!("Album/song-{number}.mp3")),
            Tags {
                title: Some("Held"),
                track: Some(number),
                ..Tags::default()
            },
        );
    }
    let index = Arc::new(index_for(&temp, &music));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let index = Arc::clone(&index);
        handles.push(thread::spawn(move || index.scan().unwrap()));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(index.snapshot().unwrap().albums[0].track_count, 12);
}

#[test]
fn reopened_index_starts_complete_after_the_default_folder_was_considered() {
    let temp = TempDir::new().unwrap();
    let music = temp.path().join("Music");
    fs::create_dir_all(&music).unwrap();
    let database = temp.path().join("library.db");
    let cache = temp.path().join("covers");
    LibraryIndex::open(&database, &cache, Some(&music))
        .unwrap()
        .scan()
        .unwrap();
    let reopened = LibraryIndex::open(&database, &cache, Some(&music)).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().scan_status,
        linkamp_lib::library::ScanStatus::Complete
    );
}
