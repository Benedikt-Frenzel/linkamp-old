# 06: Survive a restart

**What to build:** The listener restarts Linkamp and finds the evening where it was left. The queue with its track snapshots, the current entry and position, the volume, the shuffle setting, and the repeat setting persist in the app database, in tables the player owns beside the library's. The queue is written when it changes and the position periodically. On launch everything comes back paused at the saved position, so an app start never surprises the listener with sound.

**Blocked by:** 03: Seek, position, and volume, 04: Shuffle and repeat

**Status:** ready-for-agent

- [ ] The queue, current entry, position, volume, shuffle, and repeat persist across a restart
- [ ] The restored state is paused at the saved position, and Play resumes from there
- [ ] Restored queue rows keep their title, artist, album, and duration
- [ ] The queue is written when it changes, and the position often enough that a restart loses at most a little
- [ ] Player state lives in the existing app database in player-owned tables, with the library index untouched
- [ ] Persistence does not write anything outside the app data directory
- [ ] Tests close the player and reopen it on the same database, then assert the restored queue, position, volume, shuffle, and repeat
