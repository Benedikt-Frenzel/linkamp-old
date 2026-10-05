# 03: Search title, artist, and album

**What to build:** The listener types in the existing search field and the right-hand list becomes the matches. A match is a case-insensitive substring of the title, the artist, or the album name. Genre, year, and file name do not count. The tree selection stays where it is. Clearing the query restores that selection's tracks. No matches say that no tracks match the query. While a scan is still committing, search includes the tracks indexed so far.

**Blocked by:** 01: Show albums from the first watched folder

**Status:** ready-for-agent

- [ ] The existing search field filters the right-hand track list and does not add a second search box
- [ ] Matching is a case-insensitive substring over title, artist, and album display name only
- [ ] Genre, year, and file name do not produce hits
- [ ] A non-empty query replaces the track list and leaves the tree selection unchanged
- [ ] An empty query shows the selected node's tracks again
- [ ] An empty result says that no tracks match the query
- [ ] Search sees tracks that have been committed while a scan is still running
- [ ] Ctrl+F still focuses the search field
- [ ] Tests cover matching, non-matches, and restore-on-clear through the library index

## Comments

Implemented. `mise exec -- cargo test --test library` in `src-tauri` passed: 14 integration tests. The window was not exercised by hand.
