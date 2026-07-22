# Copilot Instructions for ShuttleSFTP

## Build & Run Commands

```bash
# Development (starts Vite dev server + Rust backend with hot-reload)
cargo tauri dev

# Production build (outputs platform installers)
cargo tauri build

# Frontend type-check only
npm run type-check

# Rust check/clippy (from src-tauri/)
cd src-tauri && cargo check
cd src-tauri && cargo clippy
```

No test framework is configured yet (neither frontend nor backend).

## Architecture

ShuttleSFTP is a **Tauri 2** desktop app: a Rust async backend communicates with a Vue 3 frontend via Tauri's IPC invoke system.

### Backend (src-tauri/src/)

- **`ssh/`** — SSH connection lifecycle using `russh` + `russh-sftp`. `SessionManager` holds all active sessions in `Arc<Mutex<HashMap>>`, keyed by UUID session IDs (one per tab).
- **`transfer/`** — `TransferEngine` manages concurrent file transfers with a configurable concurrency limit (currently 3). Reports progress via Tauri events.
- **`config/`** — Parses `~/.ssh/config` and manages saved connection profiles.
- **`commands/`** — Tauri `#[tauri::command]` handlers. Each module mirrors one domain (connection, filesystem, transfer, config). Commands receive `State<SessionManager>` or `State<TransferEngine>` via Tauri's DI.
- **`error.rs`** — `AppError` enum with `thiserror`; all commands return `AppResult<T>`.

### Frontend (src/)

- **`composables/useTauri.ts`** — Typed wrappers around `invoke()` for all IPC commands. This is the single boundary between Vue and Rust.
- **`stores/`** — Pinia stores (`tabs.ts`, `transfer.ts`) manage UI state with Composition API style (`defineStore` with `setup` function syntax).
- **`components/`** — Organized by feature domain: `browser/`, `connection/`, `layout/`, `transfer/`.
- **`types/`** — Shared TypeScript interfaces (`connection.ts`, `filesystem.ts`, `transfer.ts`) that mirror Rust serde structs.

### IPC Contract

Frontend types in `src/types/` must stay in sync with Rust structs in `src-tauri/src/`. When adding or modifying an IPC command:
1. Define/update the Rust command in `commands/`
2. Register it in `main.rs` `invoke_handler`
3. Add the typed wrapper in `composables/useTauri.ts`
4. Update corresponding type in `src/types/` if needed

## Conventions

- **Error handling (Rust)**: All public functions return `AppResult<T>`. Use `AppError` variants, never `unwrap()` in command handlers.
- **State management**: Shared Rust state is managed via Tauri's `State<>` injection (`SessionManager`, `TransferEngine`). Sessions are identified by UUID strings.
- **Frontend stores**: Use Pinia Composition API style (setup function, not options API).
- **Path alias**: `@` resolves to `src/` in frontend imports.
- **Tauri plugins**: `tauri-plugin-dialog` (native file pickers) and `tauri-plugin-fs` (local FS access) are enabled.
- **Capabilities (Tauri 2 ACL)**: Plugin commands and events require permissions declared in `src-tauri/capabilities/default.json`. Without matching permissions, plugin calls fail silently at runtime (no compile error). Custom `#[tauri::command]` handlers do not need capability entries.
- **Async runtime**: The backend uses Tokio with `features = ["full"]`. All SSH/SFTP operations are async.
