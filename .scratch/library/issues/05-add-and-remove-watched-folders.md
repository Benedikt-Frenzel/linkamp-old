# 05: Add and remove watched folders

**What to build:** The listener opens Preferences from the Linkamp menu and sees Watched Folders, in English. They can add a folder, which starts a background scan and does not copy the files. Adding a path that is already watched does nothing. Adding a folder inside a watched folder is refused. Adding a parent of existing watched folders replaces those nested entries. Removing a watched folder, including `~/Music`, drops its tracks from the library and leaves every file on disk. The labels Search, Preferences, and Watched Folders stay English.

**Blocked by:** 01: Show albums from the first watched folder

**Status:** ready-for-agent

- [ ] The Linkamp menu has Preferences, and it opens a Watched Folders list
- [ ] The listener can add a folder, and its audio is scanned in the background without copying files
- [ ] Adding a path that is already watched is a no-op
- [ ] Adding a directory inside a watched folder is refused
- [ ] Adding a parent of watched folders replaces those nested entries with the parent
- [ ] Removing a watched folder drops only its tracks from Artists, Albums, Genres, and Years once those views exist, and from Albums in any case
- [ ] Removing a watched folder does not delete, move, or retag files on disk
- [ ] `~/Music` can be removed like any other folder
- [ ] The visible labels are Preferences and Watched Folders
- [ ] This does not become a general settings screen
- [ ] Tests cover the folder rules through the library index, including that files remain on disk

## Comments

Implemented. `mise exec -- cargo test --test library` passed: 24 integration tests. The window was not exercised by hand.
