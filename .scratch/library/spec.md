# Library

Status: ready-for-agent

## Problem Statement

The window is open, and the library is empty. Artists, Albums, Genres, and Years all say the library has not been scanned. The music is already on disk, mostly as `Artist/Album [Year]/`, and nothing about it should be copied, retagged, or deleted. The listener cannot see artists, albums, or covers, and cannot search, until Linkamp reads those folders in place.

## Solution

Linkamp watches folders in the background and builds a library from the MP3 and FLAC files it finds. The first folder is `~/Music` when that directory exists. More folders are added and removed under Watched Folders. The window opens immediately. As the scan commits, Artists, Albums, Genres, and Years fill in, a selected node shows its tracks on the right, and search narrows that list to title, artist, and album. An album is the directory that holds the audio. Its name and year come from the tags. Cover art comes from the embedded picture, or from a JPG in that directory. Closing the window, or leaving it hidden in the tray, does not stop the scan.

## User Stories

1. As a listener, I want the window to open while the library is still scanning, so that I am not staring at a blocked shell.
2. As a listener, I want `~/Music` to be watched on first launch when that directory exists, so that my collection shows up without a setup step.
3. As a listener, I want a missing `~/Music` to leave the library empty, so that Linkamp does not create a music folder for me.
4. As a listener, I want the scan to read files in place, so that my folders stay where they are.
5. As a listener, I want Linkamp to index MP3 and FLAC files, so that the collection I actually have appears.
6. As a listener, I want other file types to be ignored, so that artwork, playlists, and notes do not show up as tracks.
7. As a listener, I want album `.m3u` files to be ignored, so that they are not imported as playlists.
8. As a listener, I want files and directories whose names start with a dot to be skipped, so that hidden junk stays out of the tree.
9. As a listener, I want directory symlinks not to be followed, so that a loop cannot scan the same tree forever.
10. As a listener, I want each audio file to appear once, keyed by its canonical path, so that two watched folders do not list the same file twice.
11. As a listener, I want a file that cannot be read to be skipped, so that one bad file does not abort the scan.
12. As a listener, I want Artists, Albums, Genres, and Years to gain entries as folders are committed, so that I can browse before the whole collection is finished.
13. As a listener, I want an in-progress scan to be visible without taking over the window, so that I know the empty tree is temporary.
14. As a listener, I want a finished scan of a folder with no audio to say that no audio files were found, so that "has not been scanned" is not left up after a successful scan.
15. As a listener, I want to select Artists and see one entry per artist tag, so that I can browse by the people on the tracks.
16. As a listener, I want an artist entry to expand to the albums that contain at least one of that artist's tracks, so that a long artist is not a flat pile.
17. As a listener, I want selecting an artist to list every track with that artist tag, so that compilations still credit the track's artist.
18. As a listener, I want "Dance Monkey" to stay under Tones and I when it lives on a compilation, so that the album name does not replace the artist.
19. As a listener, I want Albums to list one entry per directory that directly contains audio, so that an album matches the folder the files share.
20. As a listener, I want an album's display name to come from the most common album tag in that directory, so that the folder name is not what I read.
21. As a listener, I want an album's year to come from the most common year tag in that directory, so that `[Year]` in the folder name is only a hint on disk.
22. As a listener, I want a directory with no album tag to show the directory name, so that the album still has a label.
23. As a listener, I want selecting an album to list every track in that directory, so that a compilation shows every artist on the disc.
24. As a listener, I want disc subfolders to stay separate albums, so that the scanner does not merge siblings that happen to share a parent.
25. As a listener, I want loose audio files in a watched folder to form one album for that directory, so that files with no album subfolder still appear.
26. As a listener, I want Genres to list one entry per genre tag, so that I can browse the tags that are actually present.
27. As a listener, I want selecting a genre to list every track with that genre tag, so that the right-hand list matches the tree.
28. As a listener, I want Years to list one entry per year, so that I can browse chronologically.
29. As a listener, I want a year stored as a date to count as its leading four-digit year, so that `2019-06-01` still sits under 2019.
30. As a listener, I want a value with no four-digit year to count as unknown, so that junk in the year tag does not invent a year.
31. As a listener, I want selecting a year to list every track with that year, so that the year is a filter and not only a label.
32. As a listener, I want a track with no artist tag to sit under Unknown in Artists, so that I can still find it.
33. As a listener, I want a track with no genre to sit under Unknown in Genres, so that it is not dropped from that view.
34. As a listener, I want a track with no year to sit under Unknown in Years, so that it is not dropped from that view.
35. As a listener, I want Unknown tracks to remain under their artist and their album, so that a missing genre or year does not hide them elsewhere.
36. As a listener, I want Unknown to sort last in each list, so that real names stay above the bucket.
37. As a listener, I want artists, albums, and genres to sort ascending and case-insensitive, so that the tree is stable.
38. As a listener, I want years to sort newest first, so that recent music is at the top.
39. As a listener, I want an audio-play episode that lives in its own directory to show up as an album under the series name in the artist tag, so that there is no separate audio-play section.
40. As a listener, I want German and other text from tags and file names to appear unchanged, so that the library is not translated.
41. As a listener, I want each track row to show title, artist, album, and duration, so that I can recognize a track without playing it.
42. As a listener, I want duration to come from the file's metadata, so that the scan does not decode the audio.
43. As a listener, I want tracks inside an album to sort by track number and then title, so that the disc order is preserved.
44. As a listener, I want a missing track number to sort after numbered tracks, by title, so that unnumbered extras sit at the end.
45. As a listener, I want an artist, genre, or year list to sort by album, then track number, then title, so that the same disc stays together.
46. As a listener, I want the track list to stay responsive with several thousand tracks, so that the whole collection can be shown.
47. As a listener, I want to type in the existing search field and see matches from title, artist, and album, so that I can jump to a song.
48. As a listener, I want search to be a case-insensitive substring, so that I do not have to match capitals or the whole tag.
49. As a listener, I want search to ignore genre, year, and file name, so that the match is only the three fields I was promised.
50. As a listener, I want a non-empty search to replace the right-hand list, so that the hits are what I see.
51. As a listener, I want the tree selection to stay put while I search, so that clearing the query restores that node's tracks.
52. As a listener, I want an empty result to say that no tracks match the query, so that a blank pane is explained.
53. As a listener, I want search to include tracks indexed so far while a scan is still running, so that I can find music before the scan finishes.
54. As a listener, I want selecting an album to show its cover, so that I can recognize the disc.
55. As a listener, I want the embedded picture to win over a JPG in the folder, so that the tag is the source of truth.
56. As a listener, I want the front picture to be chosen when the file embeds more than one, so that a back cover does not replace it.
57. As a listener, I want a folder JPG to be used when there is no embedded picture, preferring `cover.jpg`, then `folder.jpg`, then `front.jpg`, then any other `.jpg` or `.jpeg`, so that a sidecar cover still works.
58. As a listener, I want cover lookup to stay in the album directory, so that a parent folder's image is not stolen.
59. As a listener, I want an album with no picture to show no cover, so that Linkamp does not fetch one from the network.
60. As a listener, I want covers cached as small thumbnails, so that the window does not decode a large embed for every album.
61. As a listener, I want a changed picture to refresh the thumbnail, so that replacing artwork on disk shows up later.
62. As a listener, I want a Preferences command that opens Watched Folders, so that I can see which directories are indexed.
63. As a listener, I want to add a folder, so that music outside `~/Music` joins the library.
64. As a listener, I want adding a folder to scan it in the background, so that the window stays usable.
65. As a listener, I want adding a path that is already watched to do nothing, so that the same folder is not listed twice.
66. As a listener, I want adding a folder inside an existing watched folder to be refused, so that those tracks are not indexed twice.
67. As a listener, I want adding a parent of existing watched folders to replace those nested entries, so that one root covers them.
68. As a listener, I want to remove a watched folder, including `~/Music`, so that those tracks leave the library.
69. As a listener, I want removing a watched folder to leave the files on disk, so that the library is not a delete button.
70. As a listener, I want the labels Search, Preferences, and Watched Folders to stay English, so that the commands match the rest of the UI.
71. As a listener, I want a restart to show the library that was already indexed, so that I do not wait for a full reread of unchanged files.
72. As a listener, I want startup to re-read a file whose size or modification time changed, so that tag edits made while Linkamp was closed show up.
73. As a listener, I want startup to add audio files that appeared while Linkamp was closed, so that new rips join the tree.
74. As a listener, I want startup to drop a file that is no longer on disk from Artists, Albums, Genres, and Years, so that those views do not list ghosts.
75. As a listener, I want a watched folder that cannot be opened to keep its previous tracks, so that a temporary permissions failure does not wipe the library.
76. As a listener, I want a file change while Linkamp is running to update the same index, so that I do not have to restart to see a new album.
77. As a listener, I want a renamed file to leave the old entry and appear as the new one, so that the path remains the identity.
78. As a listener, I want the selected track to remain selected across a rescan when its path is still there, so that a background update does not throw away my place.
79. As a listener, I want Now Playing to keep saying that nothing is playing, so that this slice does not pretend something is queued.
80. As a listener, I want Playlists to keep saying that saved playlists will show up later, so that an empty playlist list is not a bug.
81. As a listener, I want Linkamp to never write tags, so that organizing the library cannot touch my files.
82. As a listener, I want Linkamp to never delete music files, so that removing a folder from the library is reversible on disk.
83. As a listener, I want the scan to keep running while the window is hidden in the tray, so that hiding Linkamp does not abandon a half-built library.

## Implementation Decisions

- The library index is one Rust module. The window and the tests call the same interface: list and change watched folders, list tree nodes and their children, list tracks for a node, search, read an album cover, and apply a scan or rescan. The window reaches that interface through the app command bridge. Tests call it directly.
- Rust owns the index. The UI sends those requests and renders what comes back. It does not parse audio.
- Tags, duration, and embedded pictures are read with lofty. Duration comes from metadata. This slice does not decode audio and does not open an output stream.
- Only `.mp3` and `.flac` are indexed, case-insensitive. Anything else is ignored, including `.m3u`.
- The index and watched-folder list live in the app database under the app data directory, `$XDG_DATA_HOME/linkamp/library.db`. Cover thumbnails live in the app cache, `$XDG_CACHE_HOME/linkamp/covers`. Music files are never copied there.
- A track's identity is its canonical path. A rescan updates that row. A path that is gone is deleted from the four library views. It is not kept as a missing row.
- An album's identity is the canonical directory that directly contains the audio files. Tracks in different directories are different albums even when the album tag matches. Disc subfolders stay separate. Files dropped straight into a watched folder share that folder as their album.
- The album display name is the most common album tag among its tracks. The album year is the most common four-digit year among its tracks. If no album tag exists, the display name is the directory name. Individual tracks still group by their own artist, genre, and year tags.
- Artists group by the track artist tag. Album artist is not a separate grouping. A featured-artist string is not split.
- A tag with no artist, no genre, or no four-digit year groups under Unknown. Unknown sorts last. Artists, albums, and genres sort ascending, case-insensitive, by code point. Years sort descending numerically.
- A year tag counts as the leading four-digit number when it has one, and as unknown otherwise.
- Track order inside an album is track number, then title. Tracks with no track number come last, by title. An artist, genre, or year list orders by album display name, then track number, then title. Search orders by artist, album, track number, then title.
- Search is a case-insensitive substring over title, artist, and album display name. A non-empty query replaces the right-hand track list and does not change the tree selection. An empty query shows the selected node's tracks.
- The first launch adds `~/Music` only when it is an existing directory. Later launches keep the saved watched-folder list.
- Adding a path that is already watched is a no-op. Adding a directory inside a watched folder is refused. Adding a directory that contains watched folders replaces those nested folders with the new parent. Removing a watched folder drops its tracks from the index and does not delete or modify anything in that folder.
- If a watched folder cannot be opened, that pass leaves its existing tracks in place. A file that fails to parse is skipped. The rest of the scan continues.
- Symlinked directories are not walked. Dotfiles and dot-directories are not indexed.
- On startup the index is readable immediately. A size-and-mtime pass then re-reads changed files, adds new audio files, and drops files that are gone. While the app is running, file notification invokes that same pass for the affected folder. Both paths share one update, so a test does not have to wait on the operating-system watcher.
- The scan never runs on the UI thread. Results are published as each folder is committed. The existing empty copy remains only until the first commit. A completed scan that found nothing says that no audio files were found.
- Cover choice, in order: the embedded front picture, otherwise the first embedded picture, otherwise `cover.jpg`, `folder.jpg`, `front.jpg`, otherwise any other `.jpg` or `.jpeg` in the album directory. Comparison of those names is case-insensitive. Parent directories are not searched. The cached thumbnail's long edge is 256 pixels. It is regenerated when the source picture changes. No cover means the album has no thumbnail.
- The existing search field is the only search box. Preferences is added to the Linkamp menu and opens Watched Folders, with English labels. That screen lists, adds, and removes watched folders. It does not grow into a general settings dialog.
- The track list is virtualized. A row shows title, artist, album, and duration. Selecting an album also shows that album's cover.
- Now Playing and Playlists stay the fixed tree entries they already are. This slice does not put tracks or playlists under them.
- The player does not write tags and does not delete music files. Those are invariants of the index, not UI policy.
- There is no audio-play detector. An episode directory becomes an album, and the artist tag is the series name, because that is how the ordinary rules treat it.

## Testing Decisions

- A good test drives the library index through the interface the window uses, then asserts the tree, the track list, search hits, cover choice, and how a rescan changes those results. It does not assert SQL, thumbnail file layout, or how file notification is registered.
- The only automated seam is that index. Build it on a temporary directory of real MP3 and FLAC files and a temporary database. Do not point tests at `~/Music`, and do not open the webview or an audio device.
- Fixtures are small valid files with tags lofty can read, including a compilation (one directory, several artist tags, one album tag), a track with no genre and no year, an audio-play episode directory whose artist tag is the series, an embedded front picture, a sidecar JPG and no embedded picture, and a file that is not audio.
- Cover the watched-folder rules: default `~/Music` when it exists, no invented folder when it does not, duplicate add, nested add refused, parent add absorbing children, remove dropping tracks while the files remain on disk.
- Cover rescan by changing the temporary directory and invoking the same pass startup uses: new file appears, edited tags replace the old grouping, deleted file leaves Artists, Albums, Genres, and Years, an unreadable folder does not wipe existing rows, and a second pass over unchanged files keeps the same identities.
- Assert that a scan does not write tags and does not delete or move the fixture files.
- There are no existing tests in the repo to copy. The window is wired to the same interface and is not a second automated seam.

## Out of Scope

- Playback, queue, seek, volume, shuffle, repeat, resume position, gapless output, and MPRIS. Transport controls stay as they are.
- The visualizer. The spectrum and the preset view stay idle.
- Creating, renaming, reordering, or deleting playlists. Enqueue, Add to Playlist, and multi-select. Double-click and Enter do not play.
- Drag-and-drop of audio, folders, or `.m3u` files onto the window.
- Importing album `.m3u` files as playlists, or exporting playlists.
- Stars, play count, and last played.
- Keeping a deleted file labeled Missing in a playlist or the queue. This slice drops it from the library views.
- A tag editor, ReplayGain, an equalizer, lyrics, scrobbling, and cover art fetched from the network.
- A separate audio-play section.
- i18n. UI copy stays English.
- Windows, and the Mac build.
- Preferences other than Watched Folders.

## Further Notes

- The collection this has to survive is about 27 GB, roughly 4380 MP3s and 116 FLACs. The scan has to commit in the background and the track list has to stay virtualized.
- A manual look on this machine, after the automated tests pass, is one Antilopen Gang album, one Bravo Hits disc with the track artist preserved, and one audio-play episode showing up as an album under the series name. That look uses the real folders. The tests do not.
- Hiding the window to the tray is already how closing works. The scan has to continue across that hide. Quit still exits.
