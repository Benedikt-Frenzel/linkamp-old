# 05: Media keys, MPRIS, and tray transport

**What to build:** Linkamp behaves like the desktop's other players. The keyboard's media keys control playback, and the XFCE volume plugin shows and controls Linkamp, both through MPRIS. MPRIS reports the current title, artist, album, and length, reports the position, and accepts seeks; Raise shows the window and Quit quits. The tray menu's Play, Pause, Next, and Previous work while the window is hidden, calling the player directly in Rust instead of sending events through the webview. Hiding the window to the tray keeps the music playing; Quit is what stops it.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [x] The keyboard's media keys play, pause, and skip
- [x] The XFCE volume plugin shows Linkamp and its controls work
- [x] MPRIS reports title, artist, album, and length of the current track
- [x] MPRIS reports the position and accepts seeks
- [x] MPRIS Raise shows the window and Quit quits the app
- [x] The tray menu's Play, Pause, Next, and Previous drive the player directly in Rust and work while the window is hidden
- [x] Hiding the window to the tray does not stop playback, and Quit stops it
- [x] Playback state changes from MPRIS and the tray reach the window like local ones
- [ ] Verified by hand on the session bus; no automated MPRIS or device tests

## Comments

Served MPRIS with mpris-server on a dedicated thread (`src-tauri/src/mpris.rs`): play, pause, play/pause, stop, next, previous, Seek, and SetPosition (stale track ids ignored) call the player exactly the way the window does, so state changes travel back through the ordinary `player-changed` events; Raise shows the window and Quit exits the app. The thread mirrors the player every 250 ms: PlaybackStatus, the Can* flags, metadata (track id, title, artist, album, length), the polled Position, and the Seeked signal on rewinds and forward jumps. Media keys and the XFCE volume plugin need no code of their own — they arrive as MPRIS calls on `org.mpris.MediaPlayer2.linkamp`. The player grew what the bus needs: per-frame position tracking that a pause freezes, seek by demuxer-level seek plus a bounded decode-discard (exact for the MP3 and FLAC fixtures), seek past the end landing on the next entry, and stop, with `positionMs` added to the snapshot the window sees. The tray's Play, Pause, Next, and Previous already called the player directly in Rust since ticket 01; this ticket verified that wiring instead of rewiring it. Hiding the window keeps the audio and MPRIS threads alive; Quit exits the process, which stops the music.

`mise exec -- cargo test` passed: 47 integration tests (34 library and 13 player, 7 of them new for position, seek, stop, and their events); `corepack npm exec tsc -- --noEmit` passed. Every bus-facing box above is implemented but still needs the manual session-bus pass (media keys, the XFCE volume plugin, metadata, position and seeks, Raise, Quit, the tray with the window hidden); no session bus or audio device was available here, which is why the last box stays open. MPRIS volume stays unwired until the player has one (ticket 03), and LoopStatus and Shuffle report their defaults until repeat and shuffle exist (ticket 04). The `DesktopEntry` value `linkamp` should be confirmed against the bundled `.desktop` basename during that pass.
