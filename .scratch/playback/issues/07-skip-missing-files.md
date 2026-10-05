# 07: Skip missing files

**What to build:** A queued file that has disappeared from disk does not break the evening. When playback reaches a missing entry it skips to the next one instead of stopping or hanging. The row stays in Now Playing with the title, artist, album, and duration it was enqueued with, labeled Missing, so the skip is explained. The same holds for queues restored by a restart. The player still never writes tags and never deletes or moves files.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [ ] Playback reaching a queued file that is gone skips to the next entry and continues
- [ ] A missing entry keeps its title, artist, album, and duration in Now Playing
- [ ] A missing entry is labeled Missing in the queue view
- [ ] A restored queue after a restart skips missing entries the same way
- [ ] A missing first or last entry of a queue is handled the same as one in the middle
- [ ] Skipping a missing file does not count as playing it and does not touch anything on disk
- [ ] Tests delete a fixture file after enqueueing it, then assert the skip, the surviving row, and that playback continues
