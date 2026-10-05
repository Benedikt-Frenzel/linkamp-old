# 03: Seek, position, and volume

**What to build:** The listener sees where the current track is and can jump around in it. The transport bar shows elapsed time and duration, and a seek bar that can be dragged. Left and Right seek five seconds back and forward; a seek past the track's end lands on the next track; seeking while paused moves the resume point. Volume gets a slider and the Up and Down keys, as the app's own volume on top of PipeWire, never touching the system volume. Zero volume is silence while the stream stays alive. The window learns the position through events a few times per second, not per buffer.

**Blocked by:** 02: Cross track boundaries without a gap

**Status:** ready-for-agent

- [x] The transport bar shows the elapsed time and the duration of the current track
- [x] The seek bar shows the position and can be dragged to jump
- [x] Left and Right seek five seconds back and forward
- [x] A seek past the end of the track lands on the next track
- [x] Seeking while paused moves the point playback resumes from
- [x] A seek discards buffered decode-ahead audio and decodes from the new position
- [x] The volume slider and Up and Down change the volume, as the app volume on top of PipeWire
- [x] Changing Linkamp's volume does not change the system volume
- [x] Zero volume writes silence while the stream stays alive
- [x] Position events reach the window a few times per second
- [x] Tests assert at the fake sink that half volume writes half-magnitude samples and that a seek moves the decode position

## Comments

The player already tracked position and accepted seeks (tickets 01 and 05); this ticket made the volume real and wired everything into the window. `PlayerSnapshot` gained `volume`, a linear 0.0–1.0 multiplier the render path applies per sample to the mixed output stream — on top of PipeWire, never the system volume, so one never loses the other. Zero volume writes silence into the still-open stream, which keeps unmuting instant; a fresh player starts at 0.8. `set_volume` clamps to the range and reports a snapshot like every other command. The render callback now also emits a snapshot whenever playback has advanced 250 ms since the last report, so the window sees the position a few times per second instead of only on state changes, and hears nothing from it while paused. MPRIS mirrors the same multiplier both ways: the polling loop pushes `Volume` changes to the bus, and `SetVolume` (the XFCE volume plugin) calls the player's `set_volume`, so bus and window never disagree. Negative MPRIS volumes clamp to zero inside the player, as the MPRIS rules require.

The transport bar replaced its placeholder seek strip with a real slider flanked by elapsed time and duration in the accent color. Drags preview locally and commit on pointer release (a window-level listener catches releases that land off the slider); clicks and keyboard changes seek at once. Left and Right seek ±5 s, Up and Down step the volume by 5 %, both through the new `player_seek` and `player_set_volume` commands, and the typing suppression now covers only text-entry fields, so a focused slider or button does not swallow the transport keys.

Tests: the fixtures are digital silence, so volume and seek assertions needed real non-zero samples — the suite now writes a deterministic 16-bit PCM WAV at test time (symphonia's default features already decode it; nothing production-facing changed, the library still indexes only MP3 and FLAC). New tests assert at the fake sink: full volume writes the decoded samples unchanged, half volume writes exactly half-magnitude samples (PCM 8192 decodes to exactly 0.25, so the comparisons are bit-exact), zero volume writes silence while the stream stays open and playing, and raising the volume again restores the samples immediately; a seek moves the decode position at the sink (quiet first second, loud remainder) on the same open stream; and 1.2 s of playback emits 3–6 monotonically advancing position events while a paused player emits none even though silence keeps flowing. `mise exec -- cargo test` passed: 68 integration tests (34 library, 34 player, 4 new); `corepack npm exec tsc -- --noEmit` passed; no new clippy warnings.

Deliberately left: persisting the volume across restarts belongs to ticket 06. The window-side behavior (drag feel, slider styling) and MPRIS volume against the real XFCE plugin await the manual acceptance pass; no session bus, audio device, or interactive window was exercised here.
