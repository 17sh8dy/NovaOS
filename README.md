# NovaOS

A desktop environment that runs **as an application** on Windows (Linux later): its own desktop,
taskbar, window manager and apps. It is not a kernel and does not replace the host OS.

**Stack:** Tauri 2 · Rust · React 19 · TypeScript · Vite · Tailwind 4 · Zustand · Motion · SQLite

## Run

```
pnpm install
pnpm tauri dev      # desktop shell
pnpm dev            # browser preview (no native features; storage falls back to localStorage)
```

## Check

```
pnpm lint && pnpm format:check && pnpm typecheck && pnpm test
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

## Layout

| Path             | What                                                          |
| ---------------- | ------------------------------------------------------------- |
| `src/kernel/`    | Window manager store (`windowStore.ts`) and app registry      |
| `src/shell/`     | Desktop, Window, Taskbar, Start menu, display modes, theme    |
| `src/apps/`      | Built-in apps (Calculator, Notes, Settings)                   |
| `src/native/`    | The only bridge to Rust (`native()`), and key/value storage   |
| `src-tauri/src/` | Rust backend: SQLite (`db.rs`), virtual filesystem (`vfs.rs`), commands (`lib.rs`) |

## Display modes

- **Windowed**: normal resizable window.
- **Immersive**: borderless fullscreen. Not kiosk/always-on-top, so Alt+Tab and Win keep working.
  Exit with **Ctrl+Alt+Q**, the taskbar power button, or the Start menu.
- **Shell**: reserved; refused in this release.

## Rules of the road

- Privileged work lives in Rust behind named commands with validated input. The UI never gets raw
  filesystem or shell access (capabilities are `core:default` only).
- Apps are registered through `AppManifest`; a future `.nova` package maps onto it. No third-party
  app execution yet.
- Vexa (browser) is a separate project; do not couple it in.

## Filesystem model

The UI addresses files as `(root id, relative path)`, never as host paths. Roots: `nova` (the
NovaOS user folder in app data), the host's Documents/Pictures/Downloads/Desktop, and folders the
user adds in Settings (whole drives, system folders and the whole user profile are refused).
`vfs::resolve_in` rejects `..`, drive letters, alternate data streams and reserved names, and
canonicalizes through symlinks/junctions so nothing can resolve outside its root. Copy/move never
overwrite, delete goes to the Recycle Bin, and executable file types are never launched.
