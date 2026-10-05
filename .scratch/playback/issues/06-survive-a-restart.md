# 06: Survive a restart

**What to build:** The listener restarts Linkamp and finds the evening where it was left. The queue with its track snapshots, the current entry and position, the volume, the shuffle setting, and the repeat setting persist in the app database, in tables the player owns beside the library's. The queue is written when it changes and the position periodically. On launch everything comes back paused at the saved position, so an app start never surprises the listener with sound.

**Blocked by:** 03: Seek, position, and volume, 04: Shuffle and repeat

**Status:** ready-for-agent

- [x] The queue, current entry, position, volume, shuffle, and repeat persist across a restart
- [x] The restored state is paused at the saved position, and Play resumes from there
- [x] Restored queue rows keep their title, artist, album, and duration
- [x] The queue is written when it changes, and the position often enough that a restart loses at most a little
- [x] Player state lives in the existing app database in player-owned tables, with the library index untouched
- [x] Persistence does not write anything outside the app data directory
- [x] Tests close the player and reopen it on the same database, then assert the restored queue, position, volume, shuffle, and repeat

## Comments

Added a single-row `player_state` table beside `player_queue` so the player keeps the queue's snapshots and the listener's settings together in the same SQLite file the library already owns. The queue table records the unshuffled order, the one that survives a restart; the in-memory shuffled order is reconstructed on load when shuffle was on. `player_state` holds `current_index`, `frames_in_current`, `volume`, `shuffle`, and `repeat`; `playing` is never stored because a launch is always paused. The render callback that fires the periodic position event now also calls `save_state`, so a hard exit loses at most one ~250 ms chunk of the buffer; the command-side setters call `save_state` on every change, and a new `Player::flush()` writes both tables so Quit and hide-to-tray are safe even between render callbacks. The Tauri command bridge calls `flush()` from `quit`, from the tray's Quit, and from `CloseRequested` (hide-to-tray). `Player::open` reads the player tables into memory before the render callback opens its first `PreparedTrack`, so a fresh launch is silent.

`Core::open_current` now seeks the freshly opened track to `frames_in_current` when that value is non-zero, which is how the saved position lands on the listener's exact frame after a restart. `Player::play` no longer resets `frames_in_current` when only the decoded track is missing; that was the one path that lost the restored position on the way from load to the sink. The whole shuffle model still rests on `original_queue`: `save_queue` writes the unshuffled order, `set_shuffle` keeps the current entry anchored, and `load_state` re-applies shuffle to the same tail when shuffle was on, so the un-shuffle path after a restart is just `set_shuffle(false)`.

Tests close the player and reopen it on the same database (no shared assertion), assert the restored queue, current entry, position, volume, shuffle, and repeat come back unchanged and paused; assert a custom title/artist/album/duration survives the round trip; assert the position is at least 250 ms after a pull past one periodic boundary without an explicit save; assert `flush()` catches a Quit between render callbacks; assert shuffle survives a restart with the same current entry still anchored; assert the player-owned tables (`player_queue`, `player_state`) are created without touching the library's tables; assert nothing leaks outside the database directory. Two-second WAV fixtures (the existing `silence.mp3` is 156 ms, which is shorter than the 272 ms the original test pulled and ended the queue mid-flight) back the round-trip tests so the saved position sits comfortably inside the audio.

Verification passed: `mise exec -- cargo test` ran 76 integration tests (34 library and 42 player, 8 new for this ticket) plus 0 unit and 0 doc tests; `corepack npm exec tsc -- --noEmit` passed; no new clippy warnings.

Deliberately left: the manual session-bus pass and the desktop acceptance (one Antilopen Gang album and one Bravo Hits disc play gapless with the right artist, a media key works, a restart resumes at the same place, hide-to-tray keeps the music in the tray) belong to the human acceptance pass; no session bus, audio device, or interactive window was exercised here.