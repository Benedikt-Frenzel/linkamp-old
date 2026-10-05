# 04: Show album covers

**What to build:** The listener selects an album and sees its cover. The embedded front picture wins. If the file has pictures but none is marked front, the first embedded picture is used. If there is no embedded picture, the album directory is searched for `cover.jpg`, then `folder.jpg`, then `front.jpg`, then any other `.jpg` or `.jpeg`, ignoring case. Parent folders are not searched, and nothing is fetched from the network. An album with no picture shows no cover. Thumbnails are cached at a 256-pixel long edge and refresh when the source picture changes.

**Blocked by:** 01: Show albums from the first watched folder

**Status:** ready-for-agent

- [ ] Selecting an album shows its cover next to the track list
- [ ] An embedded front picture is preferred over a JPG in the folder and over any other embedded picture
- [ ] With no embedded picture, the sidecar preference is `cover.jpg`, then `folder.jpg`, then `front.jpg`, then any other `.jpg` or `.jpeg` in that directory
- [ ] Cover lookup does not walk to parent directories and does not use the network
- [ ] An album with no picture shows no cover
- [ ] The cached thumbnail's long edge is 256 pixels, and replacing the source picture refreshes it
- [ ] Tests cover the choice order and the refresh through the library index, using temporary files and a temporary database

## Comments

Implemented. `mise exec -- cargo test --test library` passed: 20 integration tests. Thumbnails were being joined onto the absolute album path, which writes them beside the music and outside the cache the webview is allowed to read. The cache file name is now a hash of the album id, and the long-edge test checks the file stays in the cache directory. The window was not exercised by hand.
