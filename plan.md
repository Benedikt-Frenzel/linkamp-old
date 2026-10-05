# Linkamp

A desktop player for daily use, shaped like the Winamp 5 media library: one window, local files, search, albums, playlists the app owns, and a preset visualizer. This document lives at `plan.md` in the repo. The repo is empty and `main` has no commits yet.

The first version is daily on this machine: Debian 13, XFCE, X11, PipeWire, a 2560×1600 screen at scale factor 2 (about 1280×800 logical). The same tree then builds a Mac app. Windows is out of scope.

## Language

Repository language is English. That covers the UI, menus, tray, `.desktop` file, README, code comments, commit messages, and this plan. The first version has no i18n. German shows up only when it comes from tags and filenames in the music library.

## What the first version does

One window. Transport bar on top, tree on the left, tracks on the right. A dark UI of our own: no Winamp bitmaps and no Winamp fonts. The layout follows the Winamp 5 media library: dark gray, light type, one accent for the time and the play state. The UI follows the XFCE scale factor.

The library watches folders in place and copies nothing. The first folder is `~/Music`. More folders are added in Preferences. That collection is 27 GB today, about 4380 MP3s and 116 FLACs, almost all laid out as `Artist/Album [Year]/`. Tags are usable: "Dance Monkey" stays with Tones and I, and the album is "Bravo Hits 107".

The tree has **Artists**, **Albums**, **Genres**, **Years**, and **Playlists**, plus a fixed **Now Playing** entry. Search filters title, artist, and album, and shows the hits on the right. An album is a directory. Its display name and year come from the tags. Artists, genres, and years group by tags. Tracks with no genre or no year sit under **Unknown** and still appear under Artists and Albums. Audio plays stay albums under the series name.

Double-clicking a track plays it and replaces the queue with the visible list from that track down. That applies to an album, a saved playlist, and search results. The saved playlist does not change. **Enqueue** and **Add to Playlist** are in the context menu. Multi-select is standard.

Playlists belong to the app and live in its database. The 129 album `.m3u` files sitting next to the audio are not imported. Dropping an `.m3u` from the file manager onto the window plays it as the queue. Dropped audio files or a dropped folder play the same way and are not added as a watched folder. A folder joins the library only from Preferences.

The player reads tags and never writes them. It never deletes music files. Rating (5 stars), play count, and last played live in the app. A play counts when half the track has been heard, or two minutes, whichever comes first. Cover art comes from the embedded picture, otherwise from a JPG in the album folder.

Transport: play/pause, seek, previous/next, volume, shuffle of the current queue, repeat **Off** / **All** / **One**. Volume is the app volume on top of PipeWire. A sample of the library has no ReplayGain tags, so there is no normalization. Closing the window hides it to the tray, playback continues, and media keys keep working. **Quit** is a separate command in the menu and the tray, next to **Show**, **Play**, **Pause**, **Next**, and **Previous**. **Always on Top** is a toggle, off by default. One instance: a second launch focuses the existing window. Queue, position, volume, window geometry, shuffle, repeat, and the last visualizer preset survive a restart. A missing file is labeled **Missing**.

Keys, while the search field is not focused:

- Space: play/pause
- Ctrl+Left / Ctrl+Right: previous / next track
- Left / Right: seek ±5 seconds
- Up / Down: volume
- V: toggle the preset view
- F: presets fullscreen, when that view is on
- Ctrl+F: search
- Enter on a track: same as double-click
- Delete in a playlist: remove the entry from the playlist, leave the file

Visible commands stay English on a German keyboard layout: **Search**, **Preferences**, **Watched Folders**.

Media keys go through MPRIS, so the XFCE volume plugin sees them too.

## Visualizer

A narrow spectrum stays in the transport bar. The large view is about six bundled abstract audio-reactive presets, switchable. V lays that view over the track list. The library is the normal state. F opens the same view fullscreen. No `.milk` files, no lyrics, no cover slideshow. The equalizer is not in the first version.

## Out of scope

Internet radio, video, a tag editor, an equalizer, MilkDrop compatibility, scrobbling, cover art fetched from the network, accounts, a store, auto-update, Windows, a separate audio-play section, importing album `.m3u` files as playlists, and exporting playlists. The Mac build is the step after daily use on Linux. It is not part of the first acceptance test.

## Architecture

Rust and Tauri. The toolchain comes from mise and is pinned in `mise.toml`. The UI is Vite, TypeScript, and React, with our own CSS and a virtualized track list. No component kit.

Rust owns the index, playback, and OS integration. The UI sends commands and receives position, play state, and frequency bands as events.

- Tags and embedded covers: `lofty`
- MP3 and FLAC decode: `symphonia`
- Output: `cpal`. One stream stays open. The next track is decoded before the current one ends, so album tracks do not gap. Pause and seek happen inside that stream.
- Frequency bands come from the same samples just written to the device.
- Index and state: `rusqlite`, at `$XDG_DATA_HOME/linkamp/library.db`
- Cover thumbnails: `$XDG_CACHE_HOME/linkamp/covers`
- File changes: `notify`, plus a size and mtime pass on startup
- MPRIS: `mpris-server`
- Tray and single instance: Tauri tray and single-instance
- A `.desktop` file and an icon so XFCE can launch Linkamp

A directory is an album. The scan runs in the background and does not block the window from opening. A missing file leaves the Artists, Albums, Genres, and Years views. It stays in a playlist and in the queue, marked missing, and is skipped during playback.

## Order

1. **Window.** Tauri shell, dark layout, English UI copy, tray, Quit distinct from close, Always on Top, single instance, remembered geometry, English `.desktop` file. No audio yet.
2. **Library.** Scan `~/Music`, tree, covers, search, extra folders, rescan.
3. **Playback.** Gapless stream, queue rules, seek, volume, shuffle, repeat, resume, MPRIS, and media keys.
4. **Playlists and drag-and-drop.** Create, rename, delete, reorder, enqueue. Files dropped from outside play without joining the library.
5. **Visualizer.** Spectrum, six presets, V and fullscreen, bands from the live stream.
6. **Daily use.** Stars, play count, missing files, folder preferences.

Acceptance on this machine: one Antilopen Gang album and one Bravo Hits disc play gapless with the right artist, an audio-play episode shows up as an album, closing the window keeps the music in the tray, a media key works, and a restart resumes at the same place. The Mac build comes after that: the same UI, CoreAudio output, a menu-bar icon, and Now Playing integration.

## Risks

- Gapless playback plus the visualizer on one open PipeWire stream is the hard part of audio. Acceptance depends on it.
- The XFCE tray needs an ordinary status icon. If Tauri only speaks a different protocol there, the icon is the first fix before the player is daily.
- The webview stays resident while the tray is running. That is the cost of the presets.
- macOS signing and notarization belong to the Mac step, not to Linux acceptance.
