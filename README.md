# Linkamp

A desktop player for a local music library. The window is the Winamp 5 media library: transport on top, tree on the left, tracks on the right. See `plan.md` for the full scope.

The repository language is English.

## Run

Install the Linux build dependencies once:

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  pkg-config
```

Rust is pinned in `mise.toml`.

```bash
mise install
corepack npm install
corepack npm run tauri dev
```

Closing the window hides Linkamp to the tray. Quit is in the Linkamp menu and the tray menu.

`src-tauri/linkamp.desktop` is the English desktop entry used when the app is installed.
