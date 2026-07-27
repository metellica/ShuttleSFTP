# ShuttleSFTP

A fast, lightweight, cross-platform SFTP/SCP GUI built with **Tauri 2 + Vue 3 + Rust**.

![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![License](https://img.shields.io/badge/license-MIT-green)

## Features

- 🚀 **High Performance** — Async Rust backend powered by `russh` + `tokio`
- ▣ **Container File Access** — Every host session (SSH or local) exposes its running Docker / containerd / CRI containers as a virtual `/@containers` directory: browse in, edit and transfer files like any other folder
- ⎈ **K8s Pod File Access** — The `/@pods` virtual directory walks `namespace → pod → container` via `kubectl exec` (only needs kubeconfig + `pods/exec` RBAC on the host running kubectl)
- 💻 **Local Sessions** — Open "This Machine" to browse local files plus your local Docker Desktop / nerdctl containers, no SSH required
- 🧱 **Distroless-Proof Rootfs Mode** — On SSH hosts, containers are accessed directly through their rootfs on the host (docker `MergedDir` / containerd runtime v2 task dirs), so images without a shell or `tar` still work; falls back to exec+shell automatically
- ⇄ **Any-to-Any Transfers** — Copy files between any two endpoints (local ⇄ host ⇄ container ⇄ pod): right-click **Copy** then **Paste** in any directory of any tab, drag files onto a folder or another tab, or right-click blank space to paste into the current directory; same-host copies run server-side without relaying through your machine
- 📁 **Finder-Style Browser** — macOS Finder-like Miller columns with clickable breadcrumb path bar
- 🧭 **Editable Path Bar** — Copy the current path, paste & go, or click to type a path directly
- 🗒️ **Details View** — Windows Explorer-style list view (size, permissions, modified time), toggleable
- ✅ **Multi-Select** — Ctrl+click to toggle entries, Shift+click for range selection
- 👁️ **File Preview** — Inline text preview pane with gray line numbers, soft wrap, copy support, and maximize/restore
- ✏️ **Remote Quick-Edit** — Edit text files in place (paste/undo, Ctrl+S) and save straight back to the server
- 🖱️ **Drag & Drop Upload** — Drag files or folders from your OS file manager to upload
- 📥 **Copy / Paste Between Sessions** — Mark files with **Copy** in one tab (right-click or Ctrl/Cmd+C), **Paste** them (right-click or Ctrl/Cmd+V) into any folder of any other tab (host, container or pod); blank-space right-click offers Paste / New Folder / Refresh
- ⬇️ **Flexible Download** — Toolbar download, right-click **Download…** / **Save As…** context menu
- 🗑️ **Delete with Confirmation** — Right-click → **Delete** removes selected files or folders (recursive) after a native confirmation dialog
- 🗂️ **Directory Transfers** — Recursive folder upload/download/save-as, shown as an expandable tree in the queue
- ⏯️ **Pause / Resume / Cancel** — Per file, per folder, or all at once; resume continues from the transferred offset
- 🔁 **Resume After Restart** — Interrupted transfers persist and come back as paused; resume auto-reconnects using saved credentials and opens the matching tab
- 🧹 **Safe Cancel** — Cancelling a download asks whether to delete the partial local file (or the whole folder for directory downloads)
- ⭐ **Bookmarks** — Right-click any remote folder to bookmark it (with custom alias); the bookmarks window groups paths per server (collapsible tree), and connecting opens a tab labeled `user@alias`
- 🖥️ **Integrated Terminal** — Toolbar button opens a shell in the current directory (local PTY or SSH); browsing inside a container or pod auto-attaches via `docker`/`nerdctl`/`crictl`/`kubectl exec`. Multiple terminals per tab (each tab keeps its own set), resizable drawer height
- 🔑 **Flexible Auth** — Password, private key (with passphrase), SSH agent
- 📋 **SSH Config Import** — Pick which `~/.ssh/config` hosts to import (checkbox picker with filter); only imported hosts appear in the fuzzy-search dropdown
- 💾 **Saved Profiles** — Save connections (optionally with credentials) for quick reuse, with or without connecting; aliases are globally unique across profiles and SSH config hosts
- ⧉ **Clone Connections** — One-click ⧉ in the host dropdown duplicates any saved profile or SSH config host as a new editable connection ("name copy")
- 🗂️ **Multi-Tab** — Multiple concurrent SFTP sessions in tabs (labeled by SSH alias)
- 📊 **Transfer Queue** — Live progress with sliding-window speed (network speed shown for local ⇄ remote transfers; server-side copies report live progress without a speed figure), detail view (from/to/size), and open-in-local-folder
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
│   │   ├── terminal/       # Integrated terminal (xterm.js)
│   │   └── transfer/       # Transfer queue UI
│   ├── composables/        # Tauri IPC wrappers
│   ├── stores/             # Pinia state (tabs, transfers, terminals, clipboard)
│   └── types/              # TypeScript interfaces
├── src-tauri/              # Rust backend
│   └── src/
│       ├── fs/             # RemoteFs trait, local/host/rootfs backends
│       ├── ssh/            # SSH session, auth, SFTP ops
│       ├── exec/           # Command runners (local & over SSH)
│       ├── container/      # Container/pod discovery + exec file access
│       ├── terminal/       # Interactive PTY terminals (SSH & local)
│       ├── transfer/       # Transfer engine & progress
│       ├── config/         # SSH config parser, profiles
│       └── commands/       # Tauri IPC command handlers
├── package.json
├── vite.config.ts
└── DESIGN.md               # Detailed architecture doc
```

## Usage

1. Launch the app
2. Click **+** or **Connect** to open a new session — choose **⌁ SSH Host** or **💻 This Machine** (local files + local container engines)
3. Type in the Host field to fuzzy-search your saved and imported hosts, or enter connection details manually. Use **📋 Import SSH config hosts** above the field to choose which `~/.ssh/config` hosts appear in the dropdown (none are shown until imported). Click **⧉** next to any dropdown entry to clone it as a new connection — tweak host/user/auth, then **Save** (no connection needed) or **Connect**. Duplicate alias names are rejected
4. Browse remote files in Finder-style columns — click a directory to expand it in the next column, click any breadcrumb segment to jump back
5. **Containers & pods**: at the root of every session, open **▣ `@containers`** to browse the host's running containers (name, runtime and image shown), or **⎈ `@pods`** to walk `namespace → pod → container`. Files inside behave like any other directory — preview, edit, upload, download, delete
6. **Path bar**: click the empty area (or ✏️) to type a path directly — Enter navigates, Esc cancels. Use 📋 to copy the current path, or right-click the bar for **Copy Path / Paste & Go / Edit Path**
7. **Upload**: Drag files or folders from your desktop/file manager into the app, or use the **Upload** / **Upload Folder** buttons
8. **Download**: Select files or folders (Ctrl+click to multi-select, Shift+click for a range) → click Download, or right-click → **Download…** / **Save As…**
9. **Copy between sessions**: right-click → **📋 Copy** (or **Ctrl/Cmd+C**), then navigate anywhere (another folder, another tab, into a container) and right-click → **📥 Paste** (or **Ctrl/Cmd+V** — pastes into the current directory, or into the selected folder) — or drag the selection onto a folder or onto another tab. Copies between two paths on the same host run server-side (no relay through your machine). Right-click blank space for **Paste / New Folder / Refresh**
10. **Delete**: Right-click the selection → **🗑 Delete** — a confirmation dialog appears before anything is removed (folders are deleted recursively)
11. **Preview & edit**: Click a text file to preview it with line numbers and soft wrap — 🗖 maximizes the pane. Click ✏️ to edit in place (paste/undo work natively), then 💾 or Ctrl+S saves back to the server; ✕ discards
12. **Bookmark**: Right-click a remote folder → **⭐ Add Bookmark** (alias defaults to the path; container/pod paths work too). Click **⭐ Bookmarks** in the toolbar: bookmarks are grouped by server (`user@alias`, or `user@ip:port` when no alias) — click a server row to expand its paths, then connect or delete
13. **Transfers**: The queue at the bottom shows per-file and folder-level progress with live speed. Use ⏸ / ▶ / ✕ on each row (or the header buttons for all), ℹ for details (from/to/size/server), and 📂 to reveal the local file. Interrupted transfers reappear as paused after a restart — ▶ resumes from the last byte, reconnecting automatically when credentials are saved
14. **Terminal**: Click **🖥 Terminal** to open a shell in the current directory — a local PTY for "This Machine" sessions, an SSH shell for remote hosts, and an automatic `exec` attach when you're inside `/@containers/...` or `/@pods/...`. Each browser tab keeps its own terminals: use **+** in the drawer to open more, click a chip to switch, ✕ to close, and drag the drawer's top edge to resize

### Container & pod access notes

- **SSH hosts**: containers are accessed through their rootfs on the host when possible (docker `MergedDir`, containerd runtime v2 task dirs — requires read permission, typically root). This works even for distroless/scratch images. Otherwise access falls back to `docker/nerdctl/crictl exec` + shell tools inside the container
- **Pods**: listed and accessed via `kubectl` on the host (or locally) — needs kubeconfig and `pods/exec` RBAC, no node access required
- **Distroless pods** without shell tools can't be accessed via exec; browse them through their node's `@containers` instead

### Configuration

All settings are stored as plain JSON under `~/.config/shuttle-sftp/` on every platform:

| File | Contents |
|------|----------|
| `profiles.json` | Saved connection profiles |
| `bookmarks.json` | Bookmarked remote paths |
| `imported_ssh_hosts.json` | `~/.ssh/config` hosts imported into the connect dialog |
| `transfers.json` | Transfer queue state (enables resume after restart) |

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
- [x] SSH config host import (user-selected hosts, fuzzy search)
- [x] Drag & drop upload
- [x] Download / Save As via context menu
- [x] Finder-style column view with breadcrumb navigation
- [x] Explorer-style details view with file preview
- [x] Transfer queue with progress events
- [x] Bookmarks & favorites (one-click connect to a remote path)
- [x] Directory upload/download with tree view in the queue
- [x] Pause/resume/cancel transfers (per file, per folder, all)
- [x] Transfer resume (offset continuation, also after app restart)
- [x] Multi-select (Ctrl+click / Shift+click range)
- [x] Editable path bar (copy / paste & go / direct input)
- [x] File quick-edit (remote, with line numbers and maximize)
- [x] Delete files/folders (recursive, with confirmation)
- [x] Local sessions (browse this machine without SSH)
- [x] Container file access (`/@containers` — docker / nerdctl / crictl, rootfs or exec)
- [x] K8s pod file access (`/@pods` — kubectl exec, namespace → pod → container)
- [x] Any-to-any copy (Copy/Paste + drag & drop across sessions, server-side fast path)
- [x] Clone & save connections (duplicate profiles / SSH config hosts, save without connecting, unique aliases)
- [ ] SSH agent forwarding
- [x] Integrated terminal (local PTY / SSH shell, container & pod auto-attach, multi-terminal per tab)
- [ ] Proxy support (SOCKS5/HTTP)
- [ ] i18n (English, 中文)

## License

MIT
