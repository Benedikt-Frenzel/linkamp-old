# 01: Play a track and its list, with a live queue

**What to build:** The listener double-clicks a track (or presses Enter on the selected one) and hears it through the speakers. The queue becomes the visible list from that track down, whether that list is an album, an artist, genre, or year list, a search result, or Now Playing itself. Now Playing stops saying nothing is playing and lists the queue instead, with the current track highlighted. The transport play/pause button and Space reflect and drive the real play state. Reaching the end of a track advances to the next queue entry. Ctrl+Right and Ctrl+Left skip forward and back. A library rescan while music plays neither changes the queue nor stops the audio.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Double-clicking a track plays it as real audio, and Enter on the selected track does the same
- [ ] The queue becomes the visible list from that track down, from an album, artist, genre, or year list, from search results, and from Now Playing
- [ ] A second double-click replaces a live queue the same way
- [ ] Now Playing lists the queue with the current entry highlighted
- [ ] The transport play/pause button and Space reflect and drive the real play state
- [ ] Reaching the end of a track advances to the next queue entry
- [ ] Ctrl+Right skips forward and Ctrl+Left goes back, with Previous restarting the first entry at the queue head
- [ ] A library rescan while playing does not change the queue or stop playback
- [ ] Both MP3 and FLAC play
- [ ] A missing audio device leaves the app usable instead of crashing
- [ ] Tests drive the player through the interface the window uses, with a fake output sink and a temporary database, without a webview or an audio device
