# 04: Shuffle and repeat

**What to build:** The listener gets shuffle and repeat over the live queue. Shuffle rearranges only the entries after the current one and never interrupts what is playing; turning it off restores the queue's original order. Repeat One plays the current track again, Repeat All wraps from the last entry back to the first, and Repeat Off stops after the last entry while leaving the queue in place. Play after the queue has ended starts the queue from the top. Previous restarts the current track when a few seconds of it have played, and otherwise goes to the prior entry.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [x] Shuffle rearranges only the entries after the current one, and the current entry keeps playing
- [x] Turning shuffle off restores the queue's original order
- [x] Turning shuffle on again reshuffles the remaining entries
- [x] Repeat One replays the current entry when it ends
- [x] Repeat All wraps from the last entry to the first
- [x] Repeat Off stops after the last entry and leaves the queue in place
- [x] Play after the queue has ended starts the queue from the top
- [x] Previous restarts the current entry when a few seconds have played, and otherwise goes to the prior entry
- [x] Tests assert these as sequences of current entries across several end-of-track advances at the fake sink
- [x] Shuffle and repeat are visible and toggleable in the transport bar

## Comments

Added a `RepeatMode` enum (Off/All/One), shuffle and repeat fields on `PlayerSnapshot`, and a `set_shuffle`/`set_repeat` pair on the `Player`. The queue keeps an `original_queue` snapshot so turning shuffle off restores the order the listener loaded, with the same entry still current. A small splitmix64 RNG drives Fisher–Yates so each "on" reshuffles. `next()` honours Repeat One (replay the current entry) and Repeat All (wrap to the first); Repeat Off now leaves the queue in place with `current_index = None` and `playing = false` at the end, so a follow-up `play()` starts the queue from the top. The render loop counts output frames for the current entry; `previous()` restarts when the listener is more than a few seconds in, goes to the prior entry otherwise, and always restarts at the head. The transport bar gains a Shuffle toggle and a cycling Repeat button wired to two new Tauri commands; the snapshot now carries `shuffle` and `repeat` to the window through the existing `player-changed` event. `mise exec -- cargo test` passed: 50 integration tests (34 library and 16 player, 10 new for this ticket); `corepack npm exec tsc -- --noEmit` passed. The interactive app and audio devices were not exercised by hand. Persistence of shuffle, repeat, and position across a restart is left for ticket 06.
