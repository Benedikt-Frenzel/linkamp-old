# 03: Seek, position, and volume

**What to build:** The listener sees where the current track is and can jump around in it. The transport bar shows elapsed time and duration, and a seek bar that can be dragged. Left and Right seek five seconds back and forward; a seek past the track's end lands on the next track; seeking while paused moves the resume point. Volume gets a slider and the Up and Down keys, as the app's own volume on top of PipeWire, never touching the system volume. Zero volume is silence while the stream stays alive. The window learns the position through events a few times per second, not per buffer.

**Blocked by:** 02: Cross track boundaries without a gap

**Status:** ready-for-agent

- [ ] The transport bar shows the elapsed time and the duration of the current track
- [ ] The seek bar shows the position and can be dragged to jump
- [ ] Left and Right seek five seconds back and forward
- [ ] A seek past the end of the track lands on the next track
- [ ] Seeking while paused moves the point playback resumes from
- [ ] A seek discards buffered decode-ahead audio and decodes from the new position
- [ ] The volume slider and Up and Down change the volume, as the app volume on top of PipeWire
- [ ] Changing Linkamp's volume does not change the system volume
- [ ] Zero volume writes silence while the stream stays alive
- [ ] Position events reach the window a few times per second
- [ ] Tests assert at the fake sink that half volume writes half-magnitude samples and that a seek moves the decode position
