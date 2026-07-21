# ShuttleSFTP

A fast, lightweight, cross-platform SFTP/SCP GUI built with **Tauri 2 + Vue 3 + Rust**.

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![License](https://img.shields.io/badge/license-MIT-green)

## Features

- 🚀 **High Performance** — Async Rust backend powered by `russh` + `tokio`
- 📁 **Remote File Browser** — Single-panel design, browse remote servers with ease
- 🖱️ **Drag & Drop** — Drag files from your OS file manager to upload, drag from app to download
- 🔑 **Flexible Auth** — Password, private key (with passphrase), SSH agent
- 📋 **SSH Config** — Auto-loads `~/.ssh/config` hosts
- 🗂️ **Multi-Tab** — Multiple concurrent SFTP sessions in tabs
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
cargo tauri dev
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
3. Select a host from your SSH config or enter connection details manually
4. Browse remote files — double-click to navigate directories
5. **Upload**: Drag files from your desktop/file manager into the app
6. **Download**: Select files → click Download → choose local folder

## CI / Releases

Push a version tag to trigger automated builds for all platforms:

```bash
git tag v0.1.0
git push --tags
```

GitHub Actions will produce platform-specific installers as artifacts.

## Roadmap

- [x] Multi-tab SFTP sessions
- [x] Password + private key auth
- [x] SSH config loading
- [x] Drag & drop upload/download
- [ ] SSH agent forwarding
- [ ] Bookmarks & favorites
- [ ] Integrated SSH terminal
- [ ] Transfer resume
- [ ] File quick-edit (remote)
- [ ] Proxy support (SOCKS5/HTTP)
- [ ] i18n (English, 中文)

## License

MIT
