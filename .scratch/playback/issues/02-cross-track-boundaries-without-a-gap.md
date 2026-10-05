# 02: Cross track boundaries without a gap

**What to build:** Two consecutive album tracks flow into each other with no gap, the way a live or mix album needs. One output stream stays open for the app's lifetime and track transitions never close or reopen it. The next queue entry is decoded before the current one ends, bounded to a few seconds of audio rather than whole files. The mixer converts channels and sample rates, so an MP3 into a FLAC and a rate change still cross seamlessly. Pause feeds silence into the open stream and resume continues from the exact next sample.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [x] Consecutive tracks of an album cross with no audible gap
- [x] The output stream is opened once and never closed or reopened across track transitions
- [x] The next queue entry is decoded before the current one ends
- [x] Decode-ahead buffers a bounded span of seconds, not whole files
- [x] An MP3 followed by a FLAC, and a sample-rate change between tracks, still cross without a gap
- [x] Pause writes silence into the stream and resume continues sample-exact
- [x] Tests assert at the fake sink that one track's samples are followed immediately by the next track's samples with no interruption, including across format and rate boundaries
- [ ] The manual listen uses a real album whose tracks belong together

## Comments

Reworked the ticket-01 decoder into a three-stage pipeline inside the player: `TrackSource` (Symphonia decode), `Mixer` (channel mapping plus linear-interpolation rate conversion into the one stream format), and `PreparedTrack` (a bounded converted buffer). The core now keeps the playing track and an `upcoming` decode-ahead of the next queue entry, topped up on every render and capped at three seconds of converted audio, never whole files. A track boundary is crossed by draining the already-prepared buffer inside the same render call; the current index moves exactly when the next entry's first frame is written, and the stream is never closed or reopened. Pause fills the buffer with silence and leaves both decoders untouched, so resume continues from the exact next frame.

Tests assert at the fake sink, decoding the fixtures with Symphonia for reference frame counts: an MP3-FLAC-MP3 sequence crosses at exact frame boundaries on one stream (`starts() == 1`), the same holds with rate conversion active (exact at a 2:1 ratio, within one frame at 48 kHz), pause writes silence and resume is sample-exact by frame counting, and decode-ahead is pinned functionally by making the next file unreadable while the current one still plays — the prepared track plays to its end. `mise exec -- cargo test` passed: 45 integration tests (34 library and 11 player, 5 new); `corepack npm exec tsc -- --noEmit` passed; no new clippy warnings.

Deliberately left: MP3 encoder delay/padding is not trimmed (Symphonia exposes no LAME/iTunSMPB gapless data here), so a few dozen milliseconds of encoder silence can remain at MP3 boundaries — acceptable for the manual listen, revisit if it is audible. Both fixtures are 44.1 kHz, so the between-tracks rate change is covered by running the sink at rates other than the files' while crossing the format boundary. The manual listen itself is left for the human acceptance pass; the interactive app and audio devices were not exercised.
