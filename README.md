# ShuttleSFTP

A fast, lightweight, cross-platform SFTP/SCP GUI built with **Tauri 2 + Vue 3 + Rust**.

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![License](https://img.shields.io/badge/license-MIT-green)

## Features

- 🚀 **High Performance** — Async Rust backend powered by `russh` + `tokio`
- 📁 **Finder-Style Browser** — macOS Finder-like Miller columns with clickable breadcrumb path bar
- 🗒️ **Details View** — Windows Explorer-style list view (size, permissions, modified time), toggleable
- 👁️ **File Preview** — Inline text preview pane with copy support
- 🖱️ **Drag & Drop Upload** — Drag files from your OS file manager to upload
- ⬇️ **Flexible Download** — Toolbar download, right-click **Download…** / **Save As…** context menu
- ⭐ **Bookmarks** — Right-click any remote folder to bookmark it (with custom alias), then one-click reconnect straight into that path
- 🔑 **Flexible Auth** — Password, private key (with passphrase), SSH agent
- 📋 **SSH Config** — Auto-loads `~/.ssh/config` hosts with fuzzy-search dropdown
- 💾 **Saved Profiles** — Save connections (optionally with credentials) for quick reuse
- 🗂️ **Multi-Tab** — Multiple concurrent SFTP sessions in tabs (labeled by SSH alias)
- 📊 **Transfer Queue** — Live progress, speed, and status for concurrent transfers
- 📦 **Tiny Binary** — ~10MB app bundle (vs ~150MB for Electron alternatives)
- 🖥️ **Cross-Platform** — Native on Windows, macOS, and Linux

## Screenshot

> *Coming soon*

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Framework | Tauri 2 |
| Frontend | Vue 3 + TypeScript + Pinia |
| Backend | Rust (async, tokio) |
| SSH/SFTP | russh 0.62 + russh-sftp 2.3 |
| Bundler | Vite |

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) ≥ 22
- System dependencies (Linux only):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
    libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config
  ```

### Development

```bash
# Install frontend dependencies
npm install

# Run in development mode (hot-reload)
cargo tauri dev   # or: npx tauri dev
```

### Build for Production

```bash
cargo tauri build
```

Output binaries are in `src-tauri/target/release/bundle/`:
- **Windows**: `.exe` (NSIS installer) / `.msi`
- **macOS**: `.dmg` / `.app`
- **Linux**: `.deb` / `.AppImage`

## Project Structure

```
ShuttleSFTP/
├── src/                    # Vue 3 frontend
│   ├── components/
│   │   ├── browser/        # Remote file browser panel
│   │   ├── connection/     # Connect dialog, SSH config
│   │   ├── layout/         # Tab bar, toolbar
│   │   └── transfer/       # Transfer queue UI
│   ├── composables/        # Tauri IPC wrappers
│   ├── stores/             # Pinia state (tabs, transfers)
│   └── types/              # TypeScript interfaces
├── src-tauri/              # Rust backend
│   └── src/
│       ├── ssh/            # SSH session, auth, SFTP ops
│       ├── transfer/       # Transfer engine & progress
│       ├── config/         # SSH config parser, profiles
│       └── commands/       # Tauri IPC command handlers
├── package.json
├── vite.config.ts
└── DESIGN.md               # Detailed architecture doc
```

## Usage

1. Launch the app
2. Click **+** or **Connect** to open a new session
3. Type in the Host field to fuzzy-search your SSH config hosts, or enter connection details manually
4. Browse remote files in Finder-style columns — click a directory to expand it in the next column, click any breadcrumb segment to jump back
5. **Upload**: Drag files from your desktop/file manager into the app, or use the Upload button
6. **Download**: Select files (Ctrl+click for multi-select) → click Download, or right-click → **Download…** / **Save As…**
7. **Bookmark**: Right-click a remote folder → **⭐ Add Bookmark** (alias defaults to the path). Click **⭐ Bookmarks** in the toolbar to see all bookmarks (alias + remote + path) and connect or delete

### Configuration

All settings are stored as plain JSON under `~/.config/shuttle-sftp/` on every platform:

| File | Contents |
|------|----------|
| `profiles.json` | Saved connection profiles |
| `bookmarks.json` | Bookmarked remote paths |

## CI / Releases

Push a version tag to trigger automated builds for all platforms:

```bash
git tag v0.1.0
git push --tags
```

GitHub Actions builds installers for all platforms and publishes them to a GitHub Release automatically.

## Roadmap

- [x] Multi-tab SFTP sessions
- [x] Password + private key auth
- [x] SSH config loading with fuzzy search
- [x] Drag & drop upload
- [x] Download / Save As via context menu
- [x] Finder-style column view with breadcrumb navigation
- [x] Explorer-style details view with file preview
- [x] Transfer queue with progress events
- [x] Bookmarks & favorites (one-click connect to a remote path)
- [ ] SSH agent forwarding
- [ ] Integrated SSH terminal
- [ ] Transfer resume
- [ ] File quick-edit (remote)
- [ ] Proxy support (SOCKS5/HTTP)
- [ ] i18n (English, 中文)

## License

MIT
