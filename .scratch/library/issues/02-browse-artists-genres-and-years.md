# 02: Browse artists, genres, and years

**What to build:** After albums are already on screen, the listener can browse the same library by artist, genre, and year. Artists lists one entry per artist tag and expands to the albums that contain that artist. Selecting an artist lists every track with that tag, so a compilation track stays with its own artist. Genres and years are flat lists. Selecting one lists the matching tracks. A missing artist, genre, or year sits under Unknown, and Unknown sorts last. Years sort newest first. Artists, albums, and genres sort ascending, case-insensitive. An audio-play episode that already lives in its own directory shows up as an album under the series name carried by the artist tag. There is no separate audio-play section.

**Blocked by:** 01: Show albums from the first watched folder

**Status:** ready-for-agent

- [ ] Artists lists one entry per track artist tag and expands to the albums that contain at least one of that artist's tracks
- [ ] Selecting an artist lists every track with that artist tag, ordered by album, track number, then title
- [ ] A compilation track stays under its track artist, not under the album name
- [ ] Artist strings are not split on featured-artist wording, and album artist is not a separate grouping
- [ ] Genres lists one entry per genre tag, and selecting it lists the matching tracks
- [ ] Years lists one entry per year, newest first, and selecting it lists the matching tracks
- [ ] A year tag counts as its leading four-digit year, and a value with no four-digit year counts as unknown
- [ ] Missing artist, genre, or year appears under Unknown, Unknown sorts last, and those tracks still appear under their album
- [ ] Artists, genres, and album names sort ascending and case-insensitive
- [ ] An episode directory whose artist tag is the series name appears as an album under that artist, with no extra section
- [ ] Tests cover this through the library index, using temporary files and a temporary database

## Comments

Implemented. `mise exec -- cargo test` in `src-tauri` passed: 10 integration tests. The window was not exercised by hand.

## Comments

Implemented on the library index and wired into the window. `mise exec -- cargo test` in `src-tauri` passed: 10 integration tests. `corepack npm exec tsc -- --noEmit` passed. The window was not exercised by hand.
