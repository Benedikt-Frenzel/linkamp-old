# 01: Play a track and its list, with a live queue

**What to build:** The listener double-clicks a track (or presses Enter on the selected one) and hears it through the speakers. The queue becomes the visible list from that track down, whether that list is an album, an artist, genre, or year list, a search result, or Now Playing itself. Now Playing stops saying nothing is playing and lists the queue instead, with the current track highlighted. The transport play/pause button and Space reflect and drive the real play state. Reaching the end of a track advances to the next queue entry. Ctrl+Right and Ctrl+Left skip forward and back. A library rescan while music plays neither changes the queue nor stops the audio.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [x] Double-clicking a track plays it as real audio, and Enter on the selected track does the same
- [x] The queue becomes the visible list from that track down, from an album, artist, genre, or year list, from search results, and from Now Playing
- [x] A second double-click replaces a live queue the same way
- [x] Now Playing lists the queue with the current entry highlighted
- [x] The transport play/pause button and Space reflect and drive the real play state
- [x] Reaching the end of a track advances to the next queue entry
- [x] Ctrl+Right skips forward and Ctrl+Left goes back, with Previous restarting the first entry at the queue head
- [x] A library rescan while playing does not change the queue or stop playback
- [x] Both MP3 and FLAC play
- [x] A missing audio device leaves the app usable instead of crashing
- [x] Tests drive the player through the interface the window uses, with a fake output sink and a temporary database, without a webview or an audio device

## Comments

Implemented the Symphonia/CPAL player, queue and transport command/event bridge, fake pull-driven sink, and live Now Playing UI. Audio setup falls back safely when no device is available, and the live queue stays independent of library rescans. `mise exec -- cargo test` passed: 40 integration tests (34 library and 6 player); `corepack npm exec tsc -- --noEmit` passed. The interactive app and audio devices were not exercised by hand.
