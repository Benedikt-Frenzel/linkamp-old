# 05: Media keys, MPRIS, and tray transport

**What to build:** Linkamp behaves like the desktop's other players. The keyboard's media keys control playback, and the XFCE volume plugin shows and controls Linkamp, both through MPRIS. MPRIS reports the current title, artist, album, and length, reports the position, and accepts seeks; Raise shows the window and Quit quits. The tray menu's Play, Pause, Next, and Previous work while the window is hidden, calling the player directly in Rust instead of sending events through the webview. Hiding the window to the tray keeps the music playing; Quit is what stops it.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [ ] The keyboard's media keys play, pause, and skip
- [ ] The XFCE volume plugin shows Linkamp and its controls work
- [ ] MPRIS reports title, artist, album, and length of the current track
- [ ] MPRIS reports the position and accepts seeks
- [ ] MPRIS Raise shows the window and Quit quits the app
- [ ] The tray menu's Play, Pause, Next, and Previous drive the player directly in Rust and work while the window is hidden
- [ ] Hiding the window to the tray does not stop playback, and Quit stops it
- [ ] Playback state changes from MPRIS and the tray reach the window like local ones
- [ ] Verified by hand on the session bus; no automated MPRIS or device tests
