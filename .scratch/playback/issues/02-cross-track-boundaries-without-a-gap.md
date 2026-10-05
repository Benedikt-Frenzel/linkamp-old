# 02: Cross track boundaries without a gap

**What to build:** Two consecutive album tracks flow into each other with no gap, the way a live or mix album needs. One output stream stays open for the app's lifetime and track transitions never close or reopen it. The next queue entry is decoded before the current one ends, bounded to a few seconds of audio rather than whole files. The mixer converts channels and sample rates, so an MP3 into a FLAC and a rate change still cross seamlessly. Pause feeds silence into the open stream and resume continues from the exact next sample.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [ ] Consecutive tracks of an album cross with no audible gap
- [ ] The output stream is opened once and never closed or reopened across track transitions
- [ ] The next queue entry is decoded before the current one ends
- [ ] Decode-ahead buffers a bounded span of seconds, not whole files
- [ ] An MP3 followed by a FLAC, and a sample-rate change between tracks, still cross without a gap
- [ ] Pause writes silence into the stream and resume continues sample-exact
- [ ] Tests assert at the fake sink that one track's samples are followed immediately by the next track's samples with no interruption, including across format and rate boundaries
- [ ] The manual listen uses a real album whose tracks belong together
