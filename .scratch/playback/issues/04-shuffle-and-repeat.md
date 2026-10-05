# 04: Shuffle and repeat

**What to build:** The listener gets shuffle and repeat over the live queue. Shuffle rearranges only the entries after the current one and never interrupts what is playing; turning it off restores the queue's original order. Repeat One plays the current track again, Repeat All wraps from the last entry back to the first, and Repeat Off stops after the last entry while leaving the queue in place. Play after the queue has ended starts the queue from the top. Previous restarts the current track when a few seconds of it have played, and otherwise goes to the prior entry.

**Blocked by:** 01: Play a track and its list, with a live queue

**Status:** ready-for-agent

- [ ] Shuffle rearranges only the entries after the current one, and the current entry keeps playing
- [ ] Turning shuffle off restores the queue's original order
- [ ] Turning shuffle on again reshuffles the remaining entries
- [ ] Repeat One replays the current entry when it ends
- [ ] Repeat All wraps from the last entry to the first
- [ ] Repeat Off stops after the last entry and leaves the queue in place
- [ ] Play after the queue has ended starts the queue from the top
- [ ] Previous restarts the current entry when a few seconds have played, and otherwise goes to the prior entry
- [ ] Tests assert these as sequences of current entries across several end-of-track advances at the fake sink
- [ ] Shuffle and repeat are visible and toggleable in the transport bar
