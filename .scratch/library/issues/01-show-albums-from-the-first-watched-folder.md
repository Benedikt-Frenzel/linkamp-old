# 01: Show albums from the first watched folder

**What to build:** The listener opens Linkamp and the window is usable at once. On first launch, `~/Music` is watched when that directory exists, and it is not created when it does not. MP3 and FLAC files in that folder show up as albums: one album per directory that directly contains the audio, named from the most common album tag, or from the directory name when the tag is missing, with the year taken from the most common four-digit year tag. Choosing an album lists every track in that directory, with title, artist, album, and duration, in disc order. The scan reads files in place. It does not write tags and does not delete or move music. Other file types, album `.m3u` files, dotfiles, and symlinked directories stay out of the library. A file that cannot be read is skipped. Now Playing still says nothing is playing, and Playlists still says saved playlists will show up later.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] The window opens while a scan is still running, and a quiet scanning state is visible until the first albums are committed
- [ ] First launch watches `~/Music` only when that directory already exists, and never creates it
- [ ] A missing `~/Music` leaves the library empty
- [ ] MP3 and FLAC are indexed, matching extensions case-insensitively; other files, `.m3u`, and names starting with a dot are ignored
- [ ] Symlinked directories are not walked, and each audio file appears once by its canonical path
- [ ] A file that cannot be parsed is skipped and the rest of the scan continues
- [ ] An album is the directory that directly contains the audio files, so disc subfolders stay separate albums and loose files in the watched folder form one album
- [ ] The album name is the most common album tag, falling back to the directory name; the album year is the most common four-digit year tag
- [ ] Selecting an album shows every track in that directory, with title, artist, album, and duration from metadata, ordered by track number then title, with unnumbered tracks last
- [ ] Tag text, including German, is shown unchanged
- [ ] The track list stays usable with a large number of rows
- [ ] A finished scan that found no audio says that no audio files were found, instead of claiming the library has not been scanned
- [ ] Artists, Genres, and Years are not grouped yet, and they no longer claim the library was never scanned once a scan has finished
- [ ] Now Playing and Playlists keep their current empty copy
- [ ] The search field remains, and it does not filter the track list yet
- [ ] Hiding the window does not stop the scan
- [ ] The scan never writes tags and never deletes or moves music files
- [ ] The same behavior is covered by tests that call the library index with a temporary folder of real audio files and a temporary database, without opening the window or a real `~/Music`

## Comments

Implemented on the library index and wired into the window. `mise exec -- cargo test` in `src-tauri` passed: 6 integration tests. The window was not exercised by hand.
