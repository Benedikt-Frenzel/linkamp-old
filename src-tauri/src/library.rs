use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use image::{GenericImageView, ImageReader};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

const DEFAULT_FOLDER_SETTING: &str = "default_music_folder_considered";
const THUMBNAIL_LONG_EDGE: u32 = 256;
const THUMBNAIL_QUALITY: u8 = 85;

const SIDECAR_NAMES: &[&str] = &["cover", "folder", "front"];
const SIDECAR_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

#[derive(Debug)]
pub struct LibraryError(String);

impl fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LibraryError {}

impl From<rusqlite::Error> for LibraryError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<std::io::Error> for LibraryError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanStatus {
    NotScanned,
    Scanning,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    pub year: Option<u32>,
    pub track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: String,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub duration_ms: u64,
}

/// A browsing group keyed by a tag value. `None` is the Unknown bucket,
/// which always sorts last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistEntry {
    pub name: Option<String>,
    pub albums: Vec<Album>,
    pub track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreEntry {
    pub name: Option<String>,
    pub track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearEntry {
    pub year: Option<u32>,
    pub track_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub scan_status: ScanStatus,
    pub albums: Vec<Album>,
    pub artists: Vec<ArtistEntry>,
    pub genres: Vec<GenreEntry>,
    pub years: Vec<YearEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumCover {
    pub path: PathBuf,
    pub source_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub track_count: usize,
    pub album_count: usize,
}

/// The persistent library boundary used by both Tauri commands and tests.
///
/// `default_music_folder` is injected so tests never inspect the process user's
/// real Music directory. It is considered once for each new database.
/// `cache_directory` holds the cached cover thumbnails; the app passes the
/// Tauri app cache directory, while tests use a temporary directory.
pub struct LibraryIndex {
    connection: Mutex<Connection>,
    status: Mutex<ScanStatus>,
    /// One pass at a time. Two walks must not commit against different listings.
    scan_gate: Mutex<()>,
    cache_directory: PathBuf,
}

impl LibraryIndex {
    pub fn open(
        database_path: impl AsRef<Path>,
        cache_directory: impl AsRef<Path>,
        default_music_folder: Option<&Path>,
    ) -> Result<Self> {
        let database_path = database_path.as_ref();
        let cache_directory = cache_directory.as_ref().to_path_buf();
        fs::create_dir_all(&cache_directory)?;
        if let Some(parent) = database_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut connection = Connection::open(database_path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS settings (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS watched_folders (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL UNIQUE
             );
             CREATE TABLE IF NOT EXISTS tracks (
                 path TEXT PRIMARY KEY,
                 watched_folder_id INTEGER NOT NULL REFERENCES watched_folders(id) ON DELETE CASCADE,
                 album_directory TEXT NOT NULL,
                 title TEXT NOT NULL,
                 artist TEXT,
                 album_tag TEXT,
                 genre TEXT,
                 year INTEGER,
                 track_number INTEGER,
                 duration_ms INTEGER NOT NULL,
                 file_size INTEGER NOT NULL DEFAULT -1,
                 modified_at INTEGER NOT NULL DEFAULT -1
             );
             CREATE INDEX IF NOT EXISTS tracks_by_album ON tracks(album_directory);
             CREATE TABLE IF NOT EXISTS cover_cache (
                 album_id TEXT PRIMARY KEY,
                 source_hash TEXT NOT NULL
             );",
        )?;
        ensure_rescan_columns(&connection)?;

        let transaction = connection.transaction()?;
        let already_considered = transaction
            .query_row(
                "SELECT 1 FROM settings WHERE key = ?1",
                [DEFAULT_FOLDER_SETTING],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        if !already_considered {
            if let Some(folder) = default_music_folder.filter(|folder| folder.is_dir()) {
                if let Ok(canonical) = folder.canonicalize() {
                    transaction.execute(
                        "INSERT OR IGNORE INTO watched_folders(path) VALUES (?1)",
                        [path_text(&canonical)],
                    )?;
                }
            }
            transaction.execute(
                "INSERT INTO settings(key, value) VALUES (?1, '1')",
                [DEFAULT_FOLDER_SETTING],
            )?;
        }
        transaction.commit()?;

        // A database that already considered its default folder has been
        // scanned before. A brand-new one has not.
        let initial_status = if already_considered {
            ScanStatus::Complete
        } else {
            ScanStatus::NotScanned
        };

        Ok(Self {
            connection: Mutex::new(connection),
            status: Mutex::new(initial_status),
            scan_gate: Mutex::new(()),
            cache_directory,
        })
    }

    pub fn watched_folders(&self) -> Result<Vec<PathBuf>> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT path FROM watched_folders ORDER BY path COLLATE NOCASE")?;
        let paths = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(paths.into_iter().map(PathBuf::from).collect())
    }

    /// Adds an existing directory using its canonical path.
    ///
    /// An exact duplicate is a no-op. A directory below a watched root is
    /// rejected because that root already covers it. When the new directory
    /// contains watched roots, those rows (and their indexed tracks) are
    /// replaced atomically by the new parent.
    pub fn add_watched_folder(&self, folder: &Path) -> Result<bool> {
        let canonical = folder
            .canonicalize()
            .map_err(|error| LibraryError(format!("could not add folder: {error}")))?;
        if !canonical.is_dir() {
            return Err(LibraryError("the selected path is not a folder".to_owned()));
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let watched = {
            let mut statement = transaction.prepare("SELECT path FROM watched_folders")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        };

        if watched.iter().any(|saved| saved == &canonical) {
            return Ok(false);
        }
        if watched.iter().any(|saved| canonical.starts_with(saved)) {
            return Err(LibraryError(
                "This folder is already covered by a watched folder.".to_owned(),
            ));
        }

        for nested in watched.iter().filter(|saved| saved.starts_with(&canonical)) {
            transaction.execute(
                "DELETE FROM watched_folders WHERE path = ?1",
                [path_text(nested)],
            )?;
        }
        transaction.execute(
            "INSERT INTO watched_folders(path) VALUES (?1)",
            [path_text(&canonical)],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Removes a saved watched folder without touching anything in that folder.
    /// This keeps first-launch initialization distinguishable from an empty list.
    pub fn remove_watched_folder(&self, folder: &Path) -> Result<bool> {
        let canonical = folder
            .canonicalize()
            .unwrap_or_else(|_| folder.to_path_buf());
        let changed = self.connection()?.execute(
            "DELETE FROM watched_folders WHERE path = ?1",
            [path_text(&canonical)],
        )?;
        Ok(changed > 0)
    }

    pub fn scan(&self) -> Result<ScanSummary> {
        self.scan_notifying(&|| {})
    }

    /// `on_progress` runs after scanning begins and after each watched folder commits.
    /// Callers that arrive while a pass holds `scan_gate` run their own pass when it releases.
    pub fn scan_notifying(&self, on_progress: &dyn Fn()) -> Result<ScanSummary> {
        let _gate = self
            .scan_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *self.status()? = ScanStatus::Scanning;
        on_progress();
        let result = self.scan_watched_folders(on_progress);
        if result.is_ok() {
            *self.status()? = ScanStatus::Complete;
        }
        result
    }

    pub fn snapshot(&self) -> Result<LibrarySnapshot> {
        let albums = self.albums()?;
        let rows = self.all_tracks(&albums)?;
        Ok(LibrarySnapshot {
            scan_status: *self.status()?,
            artists: artist_entries(&rows, &albums),
            genres: genre_entries(&rows),
            years: year_entries(&rows),
            albums,
        })
    }

    /// Tracks with this exact artist tag. `None` selects the Unknown bucket.
    pub fn tracks_for_artist(&self, artist: Option<&str>) -> Result<Vec<Track>> {
        self.tracks_for_group(|track| track.artist.as_deref() == artist)
    }

    pub fn tracks_for_genre(&self, genre: Option<&str>) -> Result<Vec<Track>> {
        self.tracks_for_group(|track| track.genre.as_deref() == genre)
    }

    pub fn tracks_for_year(&self, year: Option<u32>) -> Result<Vec<Track>> {
        self.tracks_for_group(|track| track.year == year)
    }

    /// Case-insensitive substring over the title, the artist tag, and the
    /// album display name. Genre, year, and file name are not searched. An
    /// empty or whitespace-only query returns no tracks: the caller is
    /// expected to show the current selection's tracks instead.
    pub fn search(&self, query: &str) -> Result<Vec<Track>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let needle = trimmed.to_lowercase();
        let albums = self.albums()?;
        let mut tracks = self
            .all_tracks(&albums)?
            .into_iter()
            .filter(|row| {
                let track = &row.track;
                track.title.to_lowercase().contains(&needle)
                    || track
                        .artist
                        .as_deref()
                        .is_some_and(|artist| artist.to_lowercase().contains(&needle))
                    || track.album.to_lowercase().contains(&needle)
            })
            .map(|row| row.track)
            .collect::<Vec<_>>();
        tracks.sort_by(search_track_order);
        Ok(tracks)
    }

    fn tracks_for_group(&self, matches: impl Fn(&Track) -> bool) -> Result<Vec<Track>> {
        let albums = self.albums()?;
        let mut tracks = self
            .all_tracks(&albums)?
            .into_iter()
            .filter(|row| matches(&row.track))
            .map(|row| row.track)
            .collect::<Vec<_>>();
        tracks.sort_by(group_track_order);
        Ok(tracks)
    }

    fn all_tracks(&self, albums: &[Album]) -> Result<Vec<LibraryRow>> {
        let album_names: HashMap<&str, &str> = albums
            .iter()
            .map(|album| (album.id.as_str(), album.name.as_str()))
            .collect();
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT path, album_directory, title, artist, genre, year, track_number, duration_ms
             FROM tracks
             ORDER BY path",
        )?;
        let rows = statement
            .query_map([], |row| {
                let album_id = row.get::<_, String>(1)?;
                let album = album_names
                    .get(album_id.as_str())
                    .map(|name| (*name).to_owned())
                    .unwrap_or_else(|| directory_name(&album_id));
                Ok(LibraryRow {
                    track: Track {
                        path: PathBuf::from(row.get::<_, String>(0)?),
                        title: row.get(2)?,
                        artist: row.get(3)?,
                        album,
                        genre: row.get(4)?,
                        year: row.get(5)?,
                        track_number: row.get(6)?,
                        duration_ms: row.get::<_, i64>(7)? as u64,
                    },
                    album_id,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn tracks_for_album(&self, album_id: &str) -> Result<Vec<Track>> {
        let album_name = self
            .albums()?
            .into_iter()
            .find(|album| album.id == album_id)
            .map(|album| album.name)
            .ok_or_else(|| LibraryError(format!("album does not exist: {album_id}")))?;

        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT path, title, artist, genre, year, track_number, duration_ms
             FROM tracks
             WHERE album_directory = ?1
             ORDER BY track_number IS NULL, track_number, title COLLATE NOCASE, path",
        )?;
        let tracks = statement
            .query_map([album_id], |row| {
                Ok(Track {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    album: album_name.clone(),
                    genre: row.get(3)?,
                    year: row.get(4)?,
                    track_number: row.get(5)?,
                    duration_ms: row.get::<_, i64>(6)? as u64,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn album_cover(&self, album_id: &str) -> Result<Option<AlbumCover>> {
        let tracks = self.tracks_for_album(album_id)?;
        // Album ids are absolute paths. Joining one would escape the cache
        // and write the thumbnail beside the music. Hash the id instead.
        let thumbnail = self
            .cache_directory
            .join(format!("{}.jpg", sha256_hex(album_id.as_bytes())));
        if tracks.is_empty() {
            self.forget_cover(album_id, &thumbnail)?;
            return Ok(None);
        }
        let album_directory = PathBuf::from(album_id);
        let audio_paths: Vec<PathBuf> = tracks.iter().map(|track| track.path.clone()).collect();

        let candidates = cover_candidates(&album_directory, &audio_paths);
        if candidates.is_empty() {
            self.forget_cover(album_id, &thumbnail)?;
            return Ok(None);
        }

        for picture_bytes in &candidates {
            let hash = sha256_hex(picture_bytes);
            let stored_hash = self
                .connection()?
                .query_row(
                    "SELECT source_hash FROM cover_cache WHERE album_id = ?1",
                    [album_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if stored_hash.as_deref() == Some(hash.as_str()) && thumbnail.is_file() {
                return Ok(Some(AlbumCover {
                    path: thumbnail,
                    source_hash: hash,
                }));
            }
            if let Some(image) = decode_picture(picture_bytes) {
                let thumbnail_image = resize_thumbnail(&image);
                let mut writer = fs::File::create(&thumbnail)?;
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut writer,
                    THUMBNAIL_QUALITY,
                );
                encoder
                    .encode_image(&thumbnail_image)
                    .map_err(|error| LibraryError(format!("cover image encode: {error}")))?;
                drop(encoder);
                drop(thumbnail_image);

                self.connection()?.execute(
                    "INSERT INTO cover_cache(album_id, source_hash) VALUES (?1, ?2)
                     ON CONFLICT(album_id) DO UPDATE SET source_hash = excluded.source_hash",
                    params![album_id, hash],
                )?;
                return Ok(Some(AlbumCover {
                    path: thumbnail,
                    source_hash: hash,
                }));
            }
        }

        self.forget_cover(album_id, &thumbnail)?;
        Ok(None)
    }

    fn forget_cover(&self, album_id: &str, thumbnail: &Path) -> Result<()> {
        let _ = fs::remove_file(thumbnail);
        self.connection()?.execute(
            "DELETE FROM cover_cache WHERE album_id = ?1",
            [album_id],
        )?;
        Ok(())
    }

    /// The size-and-mtime pass shared by startup, folder adds, and file
    /// notification. A file whose size and modification time still match the
    /// stored fingerprint keeps its row and is not re-read. Rows whose path
    /// was not seen on disk are deleted, but only for a folder whose walk
    /// succeeded.
    fn scan_watched_folders(&self, on_progress: &dyn Fn()) -> Result<ScanSummary> {
        let watched_folders = self.watched_folder_rows()?;
        let mut seen_paths = HashSet::new();

        for (folder_id, root) in watched_folders {
            // Only a failure to open the watched root keeps every previous track.
            // A nested directory that cannot be read is skipped, and paths under
            // it are not deleted, because this walk never saw them.
            let Ok(walk) = audio_paths(&root) else {
                continue;
            };

            let stored = self.folder_fingerprints(folder_id)?;
            let mut seen_in_folder = HashSet::new();
            let mut changed = Vec::new();
            for path in walk.files {
                if !seen_paths.insert(path.clone()) {
                    continue;
                }
                let path_key = path_text(&path);
                seen_in_folder.insert(path_key.clone());
                let Some((file_size, modified_at)) = file_fingerprint(&path) else {
                    continue;
                };
                // Size and modification time stand in for the file's bytes:
                // an unchanged pair skips the lofty read entirely.
                if stored.get(&path_key) == Some(&(file_size, modified_at)) {
                    continue;
                }
                if let Ok(track) = read_track(&path) {
                    changed.push((track, file_size, modified_at));
                }
            }

            let mut connection = self.connection()?;
            let transaction = connection.transaction()?;
            let still_watched = transaction
                .query_row(
                    "SELECT 1 FROM watched_folders WHERE id = ?1",
                    [folder_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            // A folder can be removed while a scan is reading it. In that
            // case, discard the stale scan result instead of trying to insert
            // tracks with a deleted foreign key.
            if !still_watched {
                continue;
            }
            for (track, file_size, modified_at) in changed {
                transaction.execute(
                    "INSERT INTO tracks (
                         path, watched_folder_id, album_directory, title, artist,
                         album_tag, genre, year, track_number, duration_ms,
                         file_size, modified_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(path) DO UPDATE SET
                         watched_folder_id = excluded.watched_folder_id,
                         album_directory = excluded.album_directory,
                         title = excluded.title,
                         artist = excluded.artist,
                         album_tag = excluded.album_tag,
                         genre = excluded.genre,
                         year = excluded.year,
                         track_number = excluded.track_number,
                         duration_ms = excluded.duration_ms,
                         file_size = excluded.file_size,
                         modified_at = excluded.modified_at",
                    params![
                        path_text(&track.path),
                        folder_id,
                        path_text(&track.album_directory),
                        track.title,
                        track.artist,
                        track.album_tag,
                        track.genre,
                        track.year,
                        track.track_number,
                        i64::try_from(track.duration_ms).unwrap_or(i64::MAX),
                        file_size,
                        modified_at,
                    ],
                )?;
            }
            // A path that is gone leaves the index; it is not kept as a
            // missing row. A rename looks like a delete plus an insert.
            let existing = {
                let mut statement = transaction
                    .prepare("SELECT path FROM tracks WHERE watched_folder_id = ?1")?;
                statement
                    .query_map([folder_id], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for path in existing {
                if seen_in_folder.contains(&path) {
                    continue;
                }
                let kept_by_skipped_directory = walk.skipped_directories.iter().any(|directory| {
                    Path::new(&path).starts_with(directory)
                });
                if kept_by_skipped_directory {
                    continue;
                }
                transaction.execute("DELETE FROM tracks WHERE path = ?1", [path])?;
            }
            transaction.commit()?;
            drop(connection);
            on_progress();
        }

        let albums = self.albums()?;
        let track_count = albums.iter().map(|album| album.track_count).sum();
        Ok(ScanSummary {
            track_count,
            album_count: albums.len(),
        })
    }

    /// Stored size and modification time for every track of one folder.
    fn folder_fingerprints(&self, folder_id: i64) -> Result<HashMap<String, (i64, i64)>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT path, file_size, modified_at FROM tracks WHERE watched_folder_id = ?1",
        )?;
        let rows = statement
            .query_map([folder_id], |row| {
                Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?)))
            })?
            .collect::<std::result::Result<HashMap<_, _>, _>>()?;
        Ok(rows)
    }

    fn watched_folder_rows(&self) -> Result<Vec<(i64, PathBuf)>> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT id, path FROM watched_folders ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get(0)?, PathBuf::from(row.get::<_, String>(1)?)))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn albums(&self) -> Result<Vec<Album>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT album_directory, album_tag, year FROM tracks ORDER BY path")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<u32>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        drop(connection);

        let mut grouped: HashMap<String, AlbumAccumulator> = HashMap::new();
        for (directory, album_tag, year) in rows {
            let album = grouped.entry(directory).or_default();
            album.track_count += 1;
            if let Some(name) = album_tag.filter(|name| !name.is_empty()) {
                *album.names.entry(name).or_default() += 1;
            }
            if let Some(year) = year {
                *album.years.entry(year).or_default() += 1;
            }
        }

        let mut albums = grouped
            .into_iter()
            .map(|(id, values)| Album {
                name: most_common(values.names).unwrap_or_else(|| directory_name(&id)),
                year: most_common(values.years),
                track_count: values.track_count,
                id,
            })
            .collect::<Vec<_>>();
        albums.sort_by(album_order);
        Ok(albums)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| LibraryError("library database lock was poisoned".to_owned()))
    }

    fn status(&self) -> Result<MutexGuard<'_, ScanStatus>> {
        self.status
            .lock()
            .map_err(|_| LibraryError("library scan status lock was poisoned".to_owned()))
    }
}

struct LibraryRow {
    track: Track,
    album_id: String,
}

fn album_order(left: &Album, right: &Album) -> Ordering {
    left.name
        .to_lowercase()
        .cmp(&right.name.to_lowercase())
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.id.cmp(&right.id))
}

/// Ascending, case-insensitive; the Unknown bucket (`None`) sorts last.
fn name_order(left: &Option<String>, right: &Option<String>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left
            .to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right)),
    }
}

/// Artist, album, track number (unnumbered last), then title. Unknown artist
/// (`None`) sorts last, matching the convention in `name_order`.
fn search_track_order(left: &Track, right: &Track) -> Ordering {
    match (&left.artist, &right.artist) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left_artist), Some(right_artist)) => left_artist
            .to_lowercase()
            .cmp(&right_artist.to_lowercase())
            .then_with(|| left_artist.cmp(right_artist)),
    }
    .then_with(|| left.album.to_lowercase().cmp(&right.album.to_lowercase()))
    .then_with(|| left.album.cmp(&right.album))
    .then_with(|| match (left.track_number, right.track_number) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    })
    .then_with(|| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.title.cmp(&right.title))
    })
    .then_with(|| left.path.cmp(&right.path))
}

/// Album display name, then track number (unnumbered last), then title.
fn group_track_order(left: &Track, right: &Track) -> Ordering {
    left.album
        .to_lowercase()
        .cmp(&right.album.to_lowercase())
        .then_with(|| left.album.cmp(&right.album))
        .then_with(|| match (left.track_number, right.track_number) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.title.cmp(&right.title))
        })
        .then_with(|| left.path.cmp(&right.path))
}

fn artist_entries(rows: &[LibraryRow], albums: &[Album]) -> Vec<ArtistEntry> {
    let mut grouped: HashMap<Option<String>, (usize, HashSet<String>)> = HashMap::new();
    for row in rows {
        let (track_count, album_ids) = grouped.entry(row.track.artist.clone()).or_default();
        *track_count += 1;
        album_ids.insert(row.album_id.clone());
    }
    let mut entries = grouped
        .into_iter()
        .map(|(name, (track_count, album_ids))| ArtistEntry {
            name,
            albums: albums
                .iter()
                .filter(|album| album_ids.contains(&album.id))
                .cloned()
                .collect(),
            track_count,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| name_order(&left.name, &right.name));
    entries
}

fn genre_entries(rows: &[LibraryRow]) -> Vec<GenreEntry> {
    let mut grouped: HashMap<Option<String>, usize> = HashMap::new();
    for row in rows {
        *grouped.entry(row.track.genre.clone()).or_default() += 1;
    }
    let mut entries = grouped
        .into_iter()
        .map(|(name, track_count)| GenreEntry { name, track_count })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| name_order(&left.name, &right.name));
    entries
}

fn year_entries(rows: &[LibraryRow]) -> Vec<YearEntry> {
    let mut grouped: HashMap<Option<u32>, usize> = HashMap::new();
    for row in rows {
        *grouped.entry(row.track.year).or_default() += 1;
    }
    let mut entries = grouped
        .into_iter()
        .map(|(year, track_count)| YearEntry { year, track_count })
        .collect::<Vec<_>>();
    // Newest first; the Unknown bucket (`None`) sorts last.
    entries.sort_by(|left, right| match (left.year, right.year) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => right.cmp(&left),
    });
    entries
}

#[derive(Default)]
struct AlbumAccumulator {
    names: HashMap<String, usize>,
    years: HashMap<u32, usize>,
    track_count: usize,
}

struct IndexedTrack {
    path: PathBuf,
    album_directory: PathBuf,
    title: String,
    artist: Option<String>,
    album_tag: Option<String>,
    genre: Option<String>,
    year: Option<u32>,
    track_number: Option<u32>,
    duration_ms: u64,
}

fn read_track(path: &Path) -> std::result::Result<IndexedTrack, lofty::error::FileParseError> {
    let tagged_file = Probe::open(path)?.guess_file_type()?.read()?;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let title = tag
        .and_then(|tag| tag.title())
        .filter(|title| !title.is_empty())
        .map(|title| title.into_owned())
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    let artist = tag
        .and_then(|tag| tag.artist())
        .filter(|value| !value.is_empty())
        .map(|value| value.into_owned());
    let album_tag = tag
        .and_then(|tag| tag.album())
        .filter(|value| !value.is_empty())
        .map(|value| value.into_owned());
    let genre = tag
        .and_then(|tag| tag.genre())
        .filter(|value| !value.is_empty())
        .map(|value| value.into_owned());
    let year = tag
        .and_then(|tag| {
            tag.get_string(ItemKey::RecordingDate)
                .or_else(|| tag.get_string(ItemKey::Year))
        })
        .and_then(leading_year);
    let track_number = tag.and_then(|tag| tag.track());
    let duration_ms = tagged_file
        .properties()
        .duration()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);

    Ok(IndexedTrack {
        path: path.to_path_buf(),
        album_directory: path.parent().unwrap_or(path).to_path_buf(),
        title,
        artist,
        album_tag,
        genre,
        year,
        track_number,
        duration_ms,
    })
}

fn leading_year(value: &str) -> Option<u32> {
    let leading = value.as_bytes().get(..4)?;
    leading
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| value[..4].parse().ok())
        .flatten()
}

struct FolderWalk {
    files: Vec<PathBuf>,
    skipped_directories: Vec<PathBuf>,
}

fn audio_paths(root: &Path) -> std::io::Result<FolderWalk> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut skipped_directories = Vec::new();
    let mut opening_root = true;

    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if opening_root => return Err(error),
            Err(_) => {
                opening_root = false;
                let skipped = directory.canonicalize().unwrap_or(directory);
                skipped_directories.push(skipped);
                continue;
            }
        };
        opening_root = false;
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if file_type.is_symlink() && fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir())
            {
                continue;
            }
            if !is_audio_path(&entry.path()) {
                continue;
            }
            let Ok(canonical) = entry.path().canonicalize() else {
                continue;
            };
            if canonical.is_file() {
                files.push(canonical);
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(FolderWalk {
        files,
        skipped_directories,
    })
}

/// Adds the rescan fingerprint columns to a database created by an earlier
/// version. The -1 default can never match a real file, so rows from such a
/// database are re-read once on the next scan and then skipped normally.
fn ensure_rescan_columns(connection: &Connection) -> Result<()> {
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(tracks)")?;
        statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for column in ["file_size", "modified_at"] {
        if !columns.iter().any(|existing| existing == column) {
            connection.execute_batch(&format!(
                "ALTER TABLE tracks ADD COLUMN {column} INTEGER NOT NULL DEFAULT -1"
            ))?;
        }
    }
    Ok(())
}

/// The fingerprint a scan stores per file: size and modification time.
/// Modification time is kept in nanoseconds since the Unix epoch so a
/// restored timestamp compares equal on filesystems with that precision.
fn file_fingerprint(path: &Path) -> Option<(i64, i64)> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let nanos = i64::try_from(modified.as_secs())
        .ok()?
        .saturating_mul(1_000_000_000)
        .saturating_add(i64::from(modified.subsec_nanos()));
    Some((i64::try_from(metadata.len()).unwrap_or(i64::MAX), nanos))
}

fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("mp3") || extension.eq_ignore_ascii_case("flac")
        })
}

fn most_common<T>(values: HashMap<T, usize>) -> Option<T>
where
    T: Ord,
{
    values
        .into_iter()
        .max_by(|(left_value, left_count), (right_value, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_value.cmp(left_value))
        })
        .map(|(value, _)| value)
}

fn directory_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn cover_candidates(album_directory: &Path, audio_paths: &[PathBuf]) -> Vec<Vec<u8>> {
    let mut sorted_audio = audio_paths.to_vec();
    sorted_audio.sort();
    sorted_audio.dedup();

    let mut fronts: Vec<Vec<u8>> = Vec::new();
    let mut first_pictures: Vec<Vec<u8>> = Vec::new();

    for audio in &sorted_audio {
        let probe = match Probe::open(audio) {
            Ok(probe) => probe,
            Err(_) => continue,
        };
        let probe = match probe.guess_file_type() {
            Ok(probe) => probe,
            Err(_) => continue,
        };
        let tagged_file = match probe.read() {
            Ok(file) => file,
            Err(_) => continue,
        };
        let Some(tag) = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
        else {
            continue;
        };
        let pictures = tag.pictures();
        if pictures.is_empty() {
            continue;
        }
        let has_front = pictures
            .iter()
            .any(|picture| picture.pic_type() == PictureType::CoverFront);
        if has_front {
            for picture in pictures {
                if picture.pic_type() == PictureType::CoverFront {
                    fronts.push(picture.data().to_vec());
                }
            }
        } else {
            first_pictures.push(pictures[0].data().to_vec());
        }
    }

    let mut candidates = fronts;
    candidates.extend(first_pictures);
    candidates.extend(sidecar_cover(album_directory));
    candidates
}

fn sidecar_cover(album_directory: &Path) -> Vec<Vec<u8>> {
    let Ok(entries) = fs::read_dir(album_directory) else {
        return Vec::new();
    };
    let mut candidates: Vec<(Vec<u8>, u8, String)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Some((stem, extension)) = split_file_name(name_str) else {
            continue;
        };
        if !SIDECAR_EXTENSIONS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
        {
            continue;
        }
        let priority = SIDECAR_NAMES
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(&stem))
            .map(|position| position as u8)
            .unwrap_or(SIDECAR_NAMES.len() as u8);
        if priority as usize > SIDECAR_NAMES.len() {
            continue;
        }
        if let Ok(bytes) = fs::read(entry.path()) {
            candidates.push((bytes, priority, entry.path().to_string_lossy().into_owned()));
        }
    }
    candidates.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.2.cmp(&right.2)));
    candidates.into_iter().map(|(bytes, _, _)| bytes).collect()
}

fn split_file_name(name: &str) -> Option<(String, String)> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() {
        return None;
    }
    Some((stem.to_owned(), extension.to_owned()))
}

fn decode_picture(bytes: &[u8]) -> Option<image::DynamicImage> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.decode().ok())
}

fn resize_thumbnail(image: &image::DynamicImage) -> image::RgbImage {
    let (width, height) = image.dimensions();
    let (target_width, target_height) = if width >= height && width > 0 {
        (
            THUMBNAIL_LONG_EDGE,
            ((height as u64 * THUMBNAIL_LONG_EDGE as u64) / width as u64).max(1) as u32,
        )
    } else if height > 0 {
        (
            ((width as u64 * THUMBNAIL_LONG_EDGE as u64) / height as u64).max(1) as u32,
            THUMBNAIL_LONG_EDGE,
        )
    } else {
        (1, 1)
    };
    image
        .resize_exact(
            target_width,
            target_height,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
