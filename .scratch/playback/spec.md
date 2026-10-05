# Playback

Status: ready-for-agent

## Problem Statement

The library is on screen, but Linkamp is silent. The transport bar's Play and Pause buttons flip a label, nothing comes out of the speakers, and Now Playing still says nothing is playing. The listener can find an album but cannot hear it. There is no queue, no seek, no volume, no repeat, and no media-key support, so the app is a browser, not a player.

## Solution

Rust gains a player that owns the queue, the transport, the decoding, and one audio output stream. Double-clicking a track (or pressing Enter on it) plays it and replaces the queue with the visible list from that track down. Consecutive album tracks cross without a gap. The listener gets play/pause, previous/next, seek, and volume in the transport bar and from the keyboard, shuffle over the remaining queue, and repeat Off/All/One. Now Playing shows the queue with the current track highlighted. MPRIS makes the XFCE media keys and the volume plugin work. Hiding the window to the tray does not stop the music; Quit does. A restart restores the queue, position, volume, shuffle, and repeat, paused at the saved place.

## User Stories

1. As a listener, I want to double-click a track and hear it, so that playing music is one action.
2. As a listener, I want Enter on the selected track to do the same as double-click, so that I can play without the mouse.
3. As a listener, I want double-clicking a track to replace the queue with the visible list from that track down, so that the rest of what I see is what plays next.
4. As a listener, I want double-clicking a track in an album to queue the rest of that album, so that a disc plays in order.
5. As a listener, I want double-clicking a track in an artist, genre, or year list to queue the rest of that list, so that any view I browse becomes the evening's music.
6. As a listener, I want double-clicking a search hit to queue the rest of the hits, so that a query becomes a queue.
7. As a listener, I want Now Playing to show the queue, so that I can see what is lined up.
8. As a listener, I want the current track highlighted in Now Playing, so that I can see where I am.
9. As a listener, I want double-clicking a track inside Now Playing to re-queue from there, so that I can jump around my own queue.
10. As a listener, I want a second track of an album to start the moment the first ends, with no gap, so that live and mix albums survive.
11. As a listener, I want the same gapless behavior when the two tracks differ in format, one MP3 and one FLAC, so that a mixed collection still flows.
12. As a listener, I want the same gapless behavior when the two tracks differ in sample rate, so that a hi-res FLAC after an MP3 still flows.
13. As a listener, I want Play to start and Pause to hold the current position, so that a pause is not a stop.
14. As a listener, I want the transport bar's play/pause button to reflect the real play state, so that the button tells the truth.
15. As a listener, I want Space to toggle play and pause, so that I can control playback without looking.
16. As a listener, I want Ctrl+Right to skip to the next track, so that skipping an unwanted song is instant.
17. As a listener, I want Ctrl+Left to go back, so that I can replay what just passed.
18. As a listener, I want Previous to restart the current track when it is a few seconds in, and to go to the prior entry otherwise, so that one press means "that one again" the way I expect.
19. As a listener, I want the seek bar to show the position in the current track, so that I can see how far in I am.
20. As a listener, I want to drag the seek bar to jump, so that skipping a boring intro is one gesture.
21. As a listener, I want Left and Right to seek five seconds back and forward, so that nudging is possible from the keyboard.
22. As a listener, I want to seek while paused and resume from the new place, so that seeking is not tied to playing.
23. As a listener, I want a seek past the end of a track to land on the next track, so that the seek bar never dead-ends.
24. As a listener, I want Up and Down to raise and lower the volume, so that I can react to a loud track quickly.
25. As a listener, I want a volume slider in the transport bar, so that I can set a level directly.
26. As a listener, I want the volume to be Linkamp's own on top of the system volume, so that changing one does not lose the other.
27. As a listener, I want zero volume to mean silence while the stream stays alive, so that unmuting is immediate.
28. As a listener, I want Shuffle to rearrange the tracks after the current one and leave the current one playing, so that shuffling never interrupts.
29. As a listener, I want turning Shuffle off to restore the queue's original order, so that a shuffle is not destructive.
30. As a listener, I want Repeat One to play the current track again, so that a song on repeat stays on.
31. As a listener, I want Repeat All to wrap from the last track back to the first, so that the queue never runs dry.
32. As a listener, I want Repeat Off to stop at the end of the queue, so that playback ends when the music ends.
33. As a listener, I want Play after the queue has ended to start the queue from the top, so that Play always means music.
34. As a listener, I want the transport bar to show the elapsed time and the duration of the current track, so that I know where I am without the seek bar.
35. As a listener, I want the queue to survive a library rescan, so that a background scan does not cut the music short.
36. As a listener, I want a queued file that has disappeared from disk to be skipped during playback, so that one deleted file does not stop the album.
37. As a listener, I want a queued file that is gone to be labeled Missing in Now Playing, so that the skip is explained.
38. As a listener, I want queue rows to keep their title, artist, album, and duration even when the file is gone, so that the Now Playing list never goes blank.
39. As a listener, I want closing the window to keep the music playing from the tray, so that hiding Linkamp is not pausing it.
40. As a listener, I want the tray menu's Play, Pause, Next, and Previous to work while the window is hidden, so that the tray is a real remote.
41. As a listener, I want Quit to be what stops the music, so that quitting is unambiguous.
42. As a listener, I want my keyboard's media keys to control playback, so that Linkamp behaves like every other player.
43. As a listener, I want the XFCE volume plugin to show and control Linkamp, so that the desktop's own controls work.
44. As a listener, I want MPRIS to report the current title, artist, and album, so that desktop shells can show what plays.
45. As a listener, I want MPRIS to report the position and accept seeks, so that outside tools can scrub.
46. As a listener, I want MPRIS Raise to show the window, so that a desktop shell's button finds it.
47. As a listener, I want a restart to restore the queue at the saved position, paused, so that Linkamp comes back where I left it without surprising me with sound.
48. As a listener, I want the volume to survive a restart, so that I set it once.
49. As a listener, I want Shuffle and Repeat to survive a restart, so that a mode is not reset behind my back.
50. As a listener, I want playback to continue while a rescan runs, so that scanning is never audible.
51. As a listener, I want the transport keys to be ignored while the search field is focused, so that typing Space searches instead of pausing.
52. As a listener, I want Linkamp to stay usable when no audio device is present, so that a missing device is not a crash.
53. As a listener, I want both MP3 and FLAC to play, so that my whole library is listenable.
54. As a listener, I want pause and resume to be sample-exact, so that a long pause picks up exactly where it left.
55. As a listener, I want the player to never write tags and never delete files, so that listening cannot damage the library.

## Implementation Decisions

- One new Rust module, the player, beside the library index, with the same shape: the window reaches it through the app command bridge, and tests call it directly. It is the only new seam in this slice.
- The player is constructed with a database path and an output sink. The app passes the real audio stream; tests pass a fake sink that records written samples and can be advanced deterministically.
- One output stream stays open for the app's lifetime. Pause feeds silence into it. Seek discards buffered audio and decodes from the new position. The next queue entry is decoded before the current one ends, so track transitions never close or reopen the stream.
- Decoding is symphonia for MP3 and FLAC. A small mixer converts each decoder's channels and sample rate to the single stream format and applies the app volume as a per-sample multiplier. Decode-ahead buffers a bounded span of seconds, never whole files.
- The queue is a list of track snapshots: path, title, artist, album, duration, captured when the queue is built. It does not join the library index at playback time, which is why rescans do not disturb it and why a gone file still has a row. A queue entry whose path no longer exists is labeled Missing and skipped when reached.
- Queue replacement takes a list and a starting entry: the queue becomes that list from the entry down. The same operation serves albums, artist, genre, and year lists, search results, and Now Playing itself.
- End-of-track advances to the next queue entry. Repeat One replays the current entry, Repeat All wraps, Repeat Off stops after the last entry and leaves the queue in place. Play from the stopped state starts the queue from the top.
- Shuffle is a mode over the queue, not a rewrite: the original order is kept, the current entry stays put, and the remaining entries play in a shuffled order that is reshuffled each time the mode turns on. Turning it off returns to the original order.
- Previous restarts the current entry when more than a few seconds of it have played, and otherwise moves to the prior entry. At the head of the queue it restarts the first entry.
- Player state persists in the existing app database at the app data location, in tables the player owns: the queue with its snapshots, the current entry and position, volume, shuffle, and repeat. The queue is written when it changes; the position is written periodically. On launch the state is restored paused at the saved position.
- The player publishes play-state, position, and queue events to the window on the same event bridge the library uses. Position events fire a few times per second, not per buffer.
- The tray menu's Play, Pause, Next, and Previous items call the player directly in Rust. They no longer travel through the webview.
- MPRIS is served by mpris-server: play, pause, toggle, next, previous, stop, seek, position, Raise, and Quit, with title, artist, album, and length as metadata. Media keys arrive through MPRIS.
- The transport bar grows real controls: play/pause, previous, next, a seek bar with elapsed time and duration, a volume slider, and shuffle and repeat toggles. Now Playing becomes the queue view with the current entry highlighted and Missing labels. Keyboard handling follows the plan: Space, Ctrl+Left/Right, Left/Right, Up/Down, Enter, all suppressed while the search field is focused.
- The library's silence fixtures and tagged-file builders are reused; no new fixture machinery.

## Testing Decisions

- A good test drives the player through the interface the window uses and asserts what a listener would notice: queue contents and order, which entry is current, the samples the sink received, timing of transitions, and what a reopen restores. It does not assert thread layout, buffer counts, or SQL.
- The only automated seam is the player with the fake sink: a temporary database, the existing MP3 and FLAC fixtures, no webview, no audio device, no MPRIS bus, and no real `~/Music`.
- The fake sink makes time deterministic: a test advances playback by pulling a number of samples, so end-of-track, decode-ahead, and seek behavior are asserted without sleeping.
- Gaplessness is asserted at the sink: reaching the end of one track's samples is followed immediately by the next track's samples, with no pause or close of the stream in between, including across an MP3-to-FLAC and a rate-change boundary.
- Volume is asserted on the samples themselves: half volume writes half-magnitude samples, zero volume writes silence, and the stream stays open.
- Pause writes silence and resume continues from the exact sample that would have come next. Seek while paused moves the resume point.
- Repeat and shuffle are asserted as sequences of current entries across several end-of-track advances, including toggling shuffle off again.
- Queue replacement is asserted from each kind of source list, including a second double-click replacing a live queue.
- A queue entry whose file is deleted after enqueue is skipped, keeps its row and label, and playback continues.
- Persistence is asserted by closing the player and reopening it on the same database: queue, current entry, position, volume, shuffle, and repeat come back, paused.
- Prior art is the library test suite: same direct-call style, same temporary directories and database, same real-file fixtures.
- MPRIS, the real output stream, media keys, and tray behavior are verified by hand in the manual acceptance pass, not by automated tests.

## Out of Scope

- Playlists: creating, renaming, deleting, and reordering them, Enqueue, Add to Playlist, and multi-select in the track list. The context menu stays as it is.
- Drag-and-drop of audio files, folders, or `.m3u` files onto the window.
- The visualizer: the spectrum in the transport bar, the preset view, fullscreen presets, and frequency bands. V and F keep their current shell behavior.
- Stars, play count, and last played. A play still counts for nothing in this slice.
- ReplayGain and any normalization, the equalizer, and MilkDrop compatibility.
- Importing or exporting playlists, and `.m3u` handling of any kind.
- The Mac build, CoreAudio, and Now Playing integration there.
- i18n. UI copy stays English.
- Windows.
- Writing tags or touching files on disk, which are invariants of the whole app.

## Further Notes

- Manual acceptance on this machine, after the automated tests pass: one Antilopen Gang album and one Bravo Hits disc play gapless with the right artist, closing the window keeps the music in the tray, a media key works, and a restart resumes at the same place.
- Gapless playback on one open stream is the known hard part of this slice. The fake-sink seam exists so its rules can be pinned down before the real device is trusted.
- The volume is app-level on top of PipeWire; it must not touch the system volume.
- The collection is about 27 GB and 4480 files. Decode-ahead bounded to a few seconds keeps memory flat no matter how long the queue is.
