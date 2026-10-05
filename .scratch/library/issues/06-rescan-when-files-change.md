# 06: Rescan when files change

**What to build:** The listener restarts Linkamp and the library is already there, without a full reread of unchanged files. A startup pass re-reads files whose size or modification time changed, adds audio files that appeared, and drops files that are no longer on disk from the library views. The same pass runs when files change while Linkamp is open, including while the window is hidden in the tray. A watched folder that cannot be opened keeps the tracks it already had. A renamed file leaves its old entry and appears under the new path. The selected track stays selected when its path survives the rescan.

**Blocked by:** 01: Show albums from the first watched folder

**Status:** ready-for-agent

- [x] A restart shows the indexed library immediately, then updates changed, new, and missing files from size and modification time
- [x] Unchanged files keep the same identity and are not fully reread
- [x] A file that disappeared leaves Artists, Albums, Genres, and Years; it is not kept as a missing row
- [x] A new audio file is added, and edited tags move the track to its new artist, album, genre, and year groupings
- [x] A rename is a removed old path plus an added new path
- [x] A watched folder that cannot be opened does not wipe its existing tracks
- [x] The same update runs for changes that happen while Linkamp is open, including while the window is hidden
- [x] A selected track whose path still exists stays selected across the update
- [x] The pass uses the saved watched-folder list, so folders added later are covered without a second mechanism
- [x] Tests call that pass directly against a temporary folder and a temporary database, without waiting on the operating-system watcher

## Comments

Implemented. `mise exec -- cargo test --test library` passed: 31 integration tests. The window was not exercised by hand.
