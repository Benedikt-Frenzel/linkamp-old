import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";

type SectionId =
  | "now-playing"
  | "artists"
  | "albums"
  | "genres"
  | "years"
  | "playlists";

type ScanStatus = "notScanned" | "scanning" | "complete";

type Album = {
  id: string;
  name: string;
  year: number | null;
  trackCount: number;
};

type Track = {
  path: string;
  title: string;
  artist: string | null;
  album: string;
  genre: string | null;
  year: number | null;
  trackNumber: number | null;
  durationMs: number;
};

type ArtistEntry = {
  name: string | null;
  albums: Album[];
  trackCount: number;
};

type GenreEntry = {
  name: string | null;
  trackCount: number;
};

type YearEntry = {
  year: number | null;
  trackCount: number;
};

type LibrarySnapshot = {
  scanStatus: ScanStatus;
  albums: Album[];
  artists: ArtistEntry[];
  genres: GenreEntry[];
  years: YearEntry[];
};

type Selection =
  | { kind: "album"; id: string }
  | { kind: "artist"; name: string | null }
  | { kind: "genre"; name: string | null }
  | { kind: "year"; year: number | null };

const SECTIONS: { id: SectionId; label: string }[] = [
  { id: "now-playing", label: "Now Playing" },
  { id: "artists", label: "Artists" },
  { id: "albums", label: "Albums" },
  { id: "genres", label: "Genres" },
  { id: "years", label: "Years" },
  { id: "playlists", label: "Playlists" },
];

const INITIAL_LIBRARY: LibrarySnapshot = {
  scanStatus: "notScanned",
  albums: [],
  artists: [],
  genres: [],
  years: [],
};

const UNKNOWN_KEY = "\0unknown";

type QueueTrack = Pick<Track, "path" | "title" | "artist" | "album" | "durationMs">;

type RepeatMode = "off" | "all" | "one";

type PlayerSnapshot = {
  queue: QueueTrack[];
  currentIndex: number | null;
  playing: boolean;
  shuffle: boolean;
  repeat: RepeatMode;
};

const INITIAL_PLAYER: PlayerSnapshot = {
  queue: [],
  currentIndex: null,
  playing: false,
  shuffle: false,
  repeat: "off",
};

const NEXT_REPEAT: Record<RepeatMode, RepeatMode> = {
  off: "all",
  all: "one",
  one: "off",
};

const REPEAT_LABEL: Record<RepeatMode, string> = {
  off: "Off",
  all: "All",
  one: "One",
};

function emptyMessage(
  section: SectionId,
  scanStatus: ScanStatus,
  hasEntries: boolean,
) {
  if (section === "now-playing") return "Nothing is playing.";
  if (section === "playlists") return "Playlists you save will show up here.";
  if (scanStatus === "scanning" && !hasEntries) return "Scanning…";
  if (scanStatus === "notScanned" && !hasEntries) return "The library has not been scanned.";
  if (!hasEntries) return "No audio files were found.";
  if (section === "albums") return "Select an album.";
  if (section === "artists") return "Select an artist.";
  if (section === "genres") return "Select a genre.";
  return "Select a year.";
}

function formatDuration(durationMs: number) {
  const seconds = Math.floor(durationMs / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function VirtualTrackList({
  tracks,
  resetKey,
  selectedPath,
  currentPath,
  onSelect,
  onActivate,
}: {
  tracks: QueueTrack[];
  resetKey: string;
  selectedPath: string | null;
  currentPath?: string | null;
  onSelect: (path: string) => void;
  onActivate: (path: string) => void;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const rowHeight = 44;
  const overscan = 8;

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const updateHeight = () => setViewportHeight(viewport.clientHeight);
    updateHeight();
    const observer = new ResizeObserver(updateHeight);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setScrollTop(0);
    viewportRef.current?.scrollTo({ top: 0 });
  }, [resetKey]);

  const rawStart = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const start = tracks.length === 0 ? 0 : Math.min(rawStart, tracks.length - 1);
  const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
  const end = Math.min(tracks.length, start + visibleCount);

  return (
    <section className="track-list" aria-label="Tracks">
      <div className="track-columns" role="row">
        <span>Title</span>
        <span>Artist</span>
        <span>Album</span>
        <span>Duration</span>
      </div>
      <div
        className="track-viewport"
        ref={viewportRef}
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        role="table"
        aria-rowcount={tracks.length}
      >
        <div className="track-spacer" style={{ height: tracks.length * rowHeight }}>
          {tracks.slice(start, end).map((track, offset) => {
            const index = start + offset;
            const selected = track.path === selectedPath;
            const current = track.path === currentPath;
            return (
              <button
                type="button"
                className={`track-row${selected ? " selected" : ""}${current ? " current" : ""}`}
                key={track.path}
                role="row"
                aria-rowindex={index + 1}
                aria-selected={selected}
                aria-current={current ? "true" : undefined}
                style={{ transform: `translateY(${index * rowHeight}px)` }}
                onClick={() => onSelect(track.path)}
                onDoubleClick={() => onActivate(track.path)}
              >
                <span role="cell" title={track.title}>{track.title}</span>
                <span role="cell" title={track.artist ?? "Unknown"}>{track.artist ?? "Unknown"}</span>
                <span role="cell" title={track.album}>{track.album}</span>
                <span role="cell" className="track-duration">{formatDuration(track.durationMs)}</span>
              </button>
            );
          })}
        </div>
      </div>
    </section>
  );
}

export function App() {
  const [section, setSection] = useState<SectionId>("now-playing");
  const [player, setPlayer] = useState<PlayerSnapshot>(INITIAL_PLAYER);
  const [menuOpen, setMenuOpen] = useState(false);
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);
  const [presetView, setPresetView] = useState(false);
  const [query, setQuery] = useState("");
  const [library, setLibrary] = useState<LibrarySnapshot>(INITIAL_LIBRARY);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [expandedArtists, setExpandedArtists] = useState<Set<string>>(new Set());
  const [tracks, setTracks] = useState<Track[]>([]);
  const [tracksLoading, setTracksLoading] = useState(false);
  const [selectedTrackPath, setSelectedTrackPath] = useState<string | null>(null);
  const [coverUrl, setCoverUrl] = useState<string | null>(null);
  const [preferencesOpen, setPreferencesOpen] = useState(false);
  const [watchedFolders, setWatchedFolders] = useState<string[]>([]);
  const [preferencesError, setPreferencesError] = useState<string | null>(null);
  const [changingFolders, setChangingFolders] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const trackRequest = useRef(0);

  useEffect(() => {
    void invoke<boolean>("always_on_top").then(setAlwaysOnTop).catch(() => {});
  }, []);

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    let unlisten: (() => void) | undefined;
    let refreshRequest = 0;

    function scheduleRefresh(delay: number) {
      if (timer !== undefined) window.clearTimeout(timer);
      timer = window.setTimeout(() => void refreshLibrary(), delay);
    }

    async function refreshLibrary() {
      const requestId = ++refreshRequest;
      try {
        const snapshot = await invoke<LibrarySnapshot>("library_snapshot");
        if (stopped || requestId !== refreshRequest) return;
        setLibrary(snapshot);
        if (snapshot.scanStatus !== "complete") scheduleRefresh(250);
      } catch {
        if (!stopped && requestId === refreshRequest) scheduleRefresh(500);
      }
    }

    void refreshLibrary();
    void listen("library-changed", () => void refreshLibrary()).then((stop) => {
      unlisten = stop;
    });
    return () => {
      stopped = true;
      if (timer !== undefined) window.clearTimeout(timer);
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const requestId = trackRequest.current + 1;
    trackRequest.current = requestId;
    const apply = (rows: Track[]) => {
      if (trackRequest.current !== requestId) return;
      setTracks(rows);
      setTracksLoading(false);
      setSelectedTrackPath((current) =>
        current !== null && rows.some((track) => track.path === current) ? current : null,
      );
    };
    const trimmed = query.trim();
    if (trimmed.length > 0) {
      setTracksLoading(true);
      invoke<Track[]>("search_tracks", { query: trimmed })
        .then(apply)
        .catch(() => apply([]));
      return;
    }
    if (!selection) {
      apply([]);
      return;
    }
    const exists =
      selection.kind === "album"
        ? library.albums.some((album) => album.id === selection.id)
        : selection.kind === "artist"
          ? library.artists.some((artist) => artist.name === selection.name)
          : selection.kind === "genre"
            ? library.genres.some((genre) => genre.name === selection.name)
            : library.years.some((year) => year.year === selection.year);
    if (!exists) {
      setSelection(null);
      apply([]);
      return;
    }
    setTracksLoading(true);
    const request: Promise<Track[]> =
      selection.kind === "album"
        ? invoke("album_tracks", { albumId: selection.id })
        : selection.kind === "artist"
          ? invoke("artist_tracks", { artist: selection.name })
          : selection.kind === "genre"
            ? invoke("genre_tracks", { genre: selection.name })
            : invoke("year_tracks", { year: selection.year });
    request.then(apply).catch(() => apply([]));
  }, [library, selection, query]);

  useEffect(() => {
    if (selection?.kind !== "album") {
      setCoverUrl(null);
      return;
    }
    const albumId = selection.id;
    let cancelled = false;
    void invoke<{ path: string; sourceHash: string } | null>("album_cover", { albumId })
      .then((cover) => {
        if (cancelled) return;
        setCoverUrl(
          cover ? `${convertFileSrc(cover.path)}?v=${encodeURIComponent(cover.sourceHash)}` : null,
        );
      })
      .catch(() => {
        if (cancelled) return;
        setCoverUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selection, library]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void invoke<PlayerSnapshot>("player_snapshot").then(setPlayer).catch(() => {});
    void listen<PlayerSnapshot>("player-changed", (event) => setPlayer(event.payload)).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target;
      const typing =
        target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      if (typing) return;
      if (event.key === " ") {
        event.preventDefault();
        void runPlayerCommand("player_toggle");
      } else if (event.key === "Enter" && selectedTrackPath) {
        event.preventDefault();
        void activateTrack(selectedTrackPath);
      } else if (event.ctrlKey && event.key === "ArrowRight") {
        event.preventDefault();
        void runPlayerCommand("player_next");
      } else if (event.ctrlKey && event.key === "ArrowLeft") {
        event.preventDefault();
        void runPlayerCommand("player_previous");
      } else if (event.key.toLowerCase() === "v") {
        setPresetView((value) => !value);
      } else if (event.key.toLowerCase() === "f") {
        setPresetView(true);
        void invoke("set_fullscreen", { on: true });
      } else if (event.key === "Escape") {
        void invoke("set_fullscreen", { on: false });
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectedTrackPath, section, query, player.queue, tracks]);

  useEffect(() => {
    function onPointerDown(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) setMenuOpen(false);
    }
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, []);

  async function toggleAlwaysOnTop() {
    const next = !alwaysOnTop;
    await invoke("set_always_on_top", { on: next });
    setAlwaysOnTop(next);
  }

  async function refreshWatchedFolders() {
    const folders = await invoke<string[]>("watched_folders");
    setWatchedFolders(folders);
  }

  function showPreferences() {
    setMenuOpen(false);
    setPreferencesError(null);
    setPreferencesOpen(true);
    void refreshWatchedFolders().catch((error: unknown) => {
      setPreferencesError(String(error));
    });
  }

  async function addWatchedFolder() {
    const folder = await open({
      directory: true,
      multiple: false,
      title: "Add Watched Folder",
    });
    if (folder === null) return;

    setChangingFolders(true);
    setPreferencesError(null);
    try {
      await invoke<boolean>("add_watched_folder", { folder });
      await refreshWatchedFolders();
    } catch (error) {
      setPreferencesError(String(error));
    } finally {
      setChangingFolders(false);
    }
  }

  async function removeWatchedFolder(folder: string) {
    setChangingFolders(true);
    setPreferencesError(null);
    try {
      await invoke<boolean>("remove_watched_folder", { folder });
      await refreshWatchedFolders();
    } catch (error) {
      setPreferencesError(String(error));
    } finally {
      setChangingFolders(false);
    }
  }

  async function runPlayerCommand(command: string) {
    try {
      setPlayer(await invoke<PlayerSnapshot>(command));
    } catch {
      // A missing device or unreadable file must not make the window unusable.
    }
  }

  async function toggleShuffle() {
    try {
      setPlayer(await invoke<PlayerSnapshot>("player_set_shuffle", { on: !player.shuffle }));
    } catch {
      // Keep the window responsive if the player refuses the change.
    }
  }

  async function cycleRepeat() {
    try {
      const next = NEXT_REPEAT[player.repeat];
      setPlayer(await invoke<PlayerSnapshot>("player_set_repeat", { mode: next }));
    } catch {
      // Keep the window responsive if the player refuses the change.
    }
  }

  async function activateTrack(path: string) {
    const visible = section === "now-playing" && !isSearching ? player.queue : tracks;
    try {
      const snapshot = await invoke<PlayerSnapshot>("play_tracks", {
        tracks: visible,
        startPath: path,
      });
      setPlayer(snapshot);
    } catch {
      // Keep browsing if the selected file can no longer be decoded.
    }
  }

  function chooseAlbum(albumId: string) {
    setSection("albums");
    setSelection({ kind: "album", id: albumId });
  }

  function chooseArtist(name: string | null) {
    setSection("artists");
    setSelection({ kind: "artist", name });
    const key = name ?? UNKNOWN_KEY;
    setExpandedArtists((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function chooseGenre(name: string | null) {
    setSection("genres");
    setSelection({ kind: "genre", name });
  }

  function chooseYear(year: number | null) {
    setSection("years");
    setSelection({ kind: "year", year });
  }

  const selectionMatchesSection =
    selection !== null &&
    ((selection.kind === "album" && section === "albums") ||
      (selection.kind === "artist" && section === "artists") ||
      (selection.kind === "genre" && section === "genres") ||
      (selection.kind === "year" && section === "years"));
  const showTracks = selectionMatchesSection;
  const trimmedQuery = query.trim();
  const isSearching = trimmedQuery.length > 0;
  const nowPlayingTracks = section === "now-playing" && !isSearching;
  const visibleTracks: QueueTrack[] = nowPlayingTracks ? player.queue : tracks;
  const currentPath = player.currentIndex === null ? null : player.queue[player.currentIndex]?.path ?? null;
  const entryCount =
    section === "albums"
      ? library.albums.length
      : section === "artists"
        ? library.artists.length
        : section === "genres"
          ? library.genres.length
          : section === "years"
            ? library.years.length
            : 0;
  const trackResetKey = `${
    selection === null
      ? "none"
      : selection.kind === "album"
        ? `album:${selection.id}`
        : selection.kind === "artist"
          ? `artist:${selection.name ?? ""}`
          : selection.kind === "genre"
            ? `genre:${selection.name ?? ""}`
            : `year:${selection.year ?? ""}`
  }|${trimmedQuery}`;

  return (
    <div className="shell">
      <header className="transport">
        <div className="menu" ref={menuRef}>
          <button
            type="button"
            className="word"
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            onClick={() => setMenuOpen((open) => !open)}
          >
            Linkamp
          </button>
          {menuOpen ? (
            <div className="menu-panel" role="menu">
              <button type="button" role="menuitem" onClick={showPreferences}>
                Preferences
              </button>
              <button type="button" role="menuitemcheckbox" aria-checked={alwaysOnTop} onClick={() => void toggleAlwaysOnTop()}>
                Always on Top
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenuOpen(false);
                  void invoke("quit");
                }}
              >
                Quit
              </button>
            </div>
          ) : null}
        </div>

        <div className="buttons" role="group" aria-label="Transport">
          <button type="button" aria-label="Previous" onClick={() => void runPlayerCommand("player_previous")}>
            Previous
          </button>
          <button
            type="button"
            className="accent"
            onClick={() => void runPlayerCommand("player_toggle")}
            aria-label={player.playing ? "Pause" : "Play"}
            aria-pressed={player.playing}
          >
            {player.playing ? "Pause" : "Play"}
          </button>
          <button type="button" aria-label="Next" onClick={() => void runPlayerCommand("player_next")}>
            Next
          </button>
        </div>

        <div className="modes" role="group" aria-label="Playback modes">
          <button
            type="button"
            className={player.shuffle ? "mode active" : "mode"}
            aria-pressed={player.shuffle}
            onClick={() => void toggleShuffle()}
            title="Shuffle"
          >
            Shuffle
          </button>
          <button
            type="button"
            className={player.repeat === "off" ? "mode" : "mode active"}
            aria-label={`Repeat: ${REPEAT_LABEL[player.repeat]}`}
            onClick={() => void cycleRepeat()}
            title={`Repeat: ${REPEAT_LABEL[player.repeat]}`}
          >
            {player.repeat === "off" ? "Repeat" : `Repeat ${REPEAT_LABEL[player.repeat]}`}
          </button>
        </div>

        <div className="seek" aria-hidden="true">
          <span className="time">0:00</span>
          <span className="bar" />
          <span className="time">0:00</span>
        </div>

        <canvas className="spectrum" width={96} height={28} aria-hidden="true" />

        <label className="volume">
          Volume
          <input type="range" min={0} max={100} defaultValue={80} aria-label="Volume" />
        </label>
      </header>

      <div className="body">
        {preferencesOpen ? (
          <main className="preferences" aria-labelledby="preferences-title">
            <div className="preferences-heading">
              <div>
                <p className="eyebrow">Preferences</p>
                <h1 id="preferences-title">Watched Folders</h1>
              </div>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setPreferencesOpen(false)}
              >
                Done
              </button>
            </div>
            <p className="preferences-copy">
              Linkamp reads music from these folders in place. Your files are not copied or changed.
            </p>
            {preferencesError ? <p className="preferences-error" role="alert">{preferencesError}</p> : null}
            <ul className="folder-list">
              {watchedFolders.map((folder) => (
                <li key={folder}>
                  <span title={folder}>{folder}</span>
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={changingFolders}
                    onClick={() => void removeWatchedFolder(folder)}
                    aria-label={`Remove ${folder}`}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
            {watchedFolders.length === 0 ? (
              <p className="no-folders">No folders are being watched.</p>
            ) : null}
            <div>
              <button
                type="button"
                className="primary-button"
                disabled={changingFolders}
                onClick={() => void addWatchedFolder()}
              >
                Add Folder…
              </button>
            </div>
          </main>
        ) : (
          <>
        <nav className="tree" aria-label="Library">
          <div className="tree-items">
            {SECTIONS.map((item) => (
              <div key={item.id}>
                <button
                  type="button"
                  className={
                    item.id === section && !selectionMatchesSection ? "selected" : undefined
                  }
                  aria-current={item.id === section ? "page" : undefined}
                  onClick={() => setSection(item.id)}
                >
                  {item.label}
                </button>
                {item.id === "albums" ? (
                  <div className="album-nodes">
                    {library.albums.map((album) => (
                      <button
                        type="button"
                        key={album.id}
                        className={
                          selection?.kind === "album" && selection.id === album.id
                            ? "selected"
                            : undefined
                        }
                        onClick={() => chooseAlbum(album.id)}
                        title={album.id}
                      >
                        <span>{album.name}</span>
                        {album.year === null ? null : <small>{album.year}</small>}
                      </button>
                    ))}
                  </div>
                ) : null}
                {item.id === "artists" ? (
                  <div className="album-nodes">
                    {library.artists.map((artist) => {
                      const key = artist.name ?? UNKNOWN_KEY;
                      const expanded = expandedArtists.has(key);
                      return (
                        <div key={key}>
                          <button
                            type="button"
                            className={
                              selection?.kind === "artist" && selection.name === artist.name
                                ? "selected"
                                : undefined
                            }
                            aria-expanded={expanded}
                            onClick={() => chooseArtist(artist.name)}
                          >
                            <span>{expanded ? "▾ " : "▸ "}{artist.name ?? "Unknown"}</span>
                          </button>
                          {expanded ? (
                            <div className="album-nodes">
                              {artist.albums.map((album) => (
                                <button
                                  type="button"
                                  key={album.id}
                                  onClick={() => chooseAlbum(album.id)}
                                  title={album.id}
                                >
                                  <span>{album.name}</span>
                                  {album.year === null ? null : <small>{album.year}</small>}
                                </button>
                              ))}
                            </div>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                ) : null}
                {item.id === "genres" ? (
                  <div className="album-nodes">
                    {library.genres.map((genre) => (
                      <button
                        type="button"
                        key={genre.name ?? UNKNOWN_KEY}
                        className={
                          selection?.kind === "genre" && selection.name === genre.name
                            ? "selected"
                            : undefined
                        }
                        onClick={() => chooseGenre(genre.name)}
                      >
                        <span>{genre.name ?? "Unknown"}</span>
                      </button>
                    ))}
                  </div>
                ) : null}
                {item.id === "years" ? (
                  <div className="album-nodes">
                    {library.years.map((entry) => (
                      <button
                        type="button"
                        key={entry.year ?? UNKNOWN_KEY}
                        className={
                          selection?.kind === "year" && selection.year === entry.year
                            ? "selected"
                            : undefined
                        }
                        onClick={() => chooseYear(entry.year)}
                      >
                        <span>{entry.year ?? "Unknown"}</span>
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
          {library.scanStatus === "scanning" ? <p className="scan-state">Scanning…</p> : null}
        </nav>

        <main className="tracks">
          {selection?.kind === "album" && coverUrl ? (
            <aside className="cover" aria-label="Album cover">
              <img src={coverUrl} alt="" />
            </aside>
          ) : null}
          <section className="tracks-pane">
            <label className="search">
              <span className="visually-hidden">Search</span>
              <input
                ref={searchRef}
                value={query}
                placeholder="Search"
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            {presetView ? (
              <div className="preset" role="region" aria-label="Visualizer">
                <p>Preset view</p>
                <p className="muted">Shaders arrive with playback.</p>
              </div>
            ) : nowPlayingTracks ? (
              player.queue.length > 0 ? (
                <VirtualTrackList
                  tracks={visibleTracks}
                  resetKey="now-playing"
                  selectedPath={selectedTrackPath}
                  currentPath={currentPath}
                  onSelect={setSelectedTrackPath}
                  onActivate={(path) => void activateTrack(path)}
                />
              ) : (
                <div className="empty"><p>Nothing is playing.</p></div>
              )
            ) : isSearching ? (
              tracksLoading ? null : tracks.length > 0 ? (
                <VirtualTrackList
                  tracks={tracks}
                  resetKey={trackResetKey}
                  selectedPath={selectedTrackPath}
                  onSelect={setSelectedTrackPath}
                  onActivate={(path) => void activateTrack(path)}
                />
              ) : (
                <div className="empty"><p>{`No tracks match "${trimmedQuery}"`}</p></div>
              )
            ) : showTracks ? (
              tracksLoading ? null : tracks.length > 0 ? (
                <VirtualTrackList
                  tracks={tracks}
                  resetKey={trackResetKey}
                  selectedPath={selectedTrackPath}
                  onSelect={setSelectedTrackPath}
                  onActivate={(path) => void activateTrack(path)}
                />
              ) : library.scanStatus === "complete" && entryCount === 0 ? (
                <div className="empty"><p>No audio files were found.</p></div>
              ) : null
            ) : (
              <div className="empty">
                <p>{emptyMessage(section, library.scanStatus, entryCount > 0)}</p>
              </div>
            )}
          </section>
        </main>
          </>
        )}
      </div>
    </div>
  );
}
