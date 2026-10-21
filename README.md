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

| Path             | What                                                                               |
| ---------------- | ---------------------------------------------------------------------------------- |
| `src/kernel/`    | Window manager store (`windowStore.ts`) and app registry                           |
| `src/shell/`     | Desktop, Window, Taskbar, Start menu, display modes, theme                         |
| `src/apps/`      | Built-in apps (Calculator, Notes, Settings)                                        |
| `src/native/`    | The only bridge to Rust (`native()`), and key/value storage                        |
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

## Importing host files

Nova Files → **Import** brings Windows/Linux files and folders in through the native picker
(the dialog runs in Rust; the UI only ever holds an opaque selection id, never a host path).

- **Copy** (default): independent copies in `NovaFiles/Imports`. Source files are opened read-only
  and nothing in the engine can write or delete a source path. Each top-level item is built in a
  hidden `.nova-staging-*` folder and moved into place only when complete, so a cancel, crash or
  full disk never leaves a half-copied item. Names never overwrite (`name (2)`). Folder structure,
  empty folders and file modification times are preserved; permissions are not (copies are plain
  editable files). A read-only scan first reports file counts, size, free space, skipped links and
  unreadable items.
- **Link**: references the original in place, nothing copied. Requires an explicit acknowledgement
  that edits/renames/deletes change the originals; linked items sit under a warning "Linked"
  heading. Unlinking removes only the reference.
- Links/junctions are never followed or copied; special files are skipped; locked or unreadable
  files are reported and the rest continue; a full disk stops the job and removes the in-flight item.
- Metadata (per-item and per-file size and original mtime, status, errors) lives in SQLite
  (`imports`, `import_files`). Interrupted jobs are cleaned up at next start.
- Text opens in Nova Editor, images in Nova Viewer, other files in the host's default app.
  **NovaOS cannot run imported Windows or Linux programs**: `.exe`, `.bat`, `.sh` and similar are
  stored as data and refused by the launcher.

## Custom wallpapers and the selection rectangle

- **Wallpaper → Add wallpaper…** opens the native picker in Rust. The image must really be a
  PNG/JPG/WebP/GIF/BMP (checked by its bytes, not its extension), at most 30 MB. It is _copied_
  into `NovaFiles/Pictures/Wallpapers` under a normalized safe name (never overwriting); the
  original is only read. Settings store `file:<name>`; the image is loaded back through the
  sandboxed filesystem. If the file is missing or won't decode, the desktop falls back to the
  default wallpaper. Fit (fill/fit/stretch/tile/center) and dimming are settings. Removing a
  wallpaper sends it to the Recycle Bin.
- **Selection rectangle**: drag on the empty desktop to box-select icons (Ctrl adds). Color
  follows the accent or is custom; fill strength and an on/off switch are in Appearance.

## Nova Studio

One app for every customization (Settings keeps system pages: General, Files & Storage,
Keyboard shortcuts, Privacy & Security, Data & Backup, About).

- **Studios** in one window: Themes, Windows, Taskbar, Desktop, Motion, Cursor, Sounds, Interface,
  plus Home and Library. Every control is defined once in `apps/studio/controls.tsx`; the studio
  pages, global search and Home favorites all render from that list.
- **Live preview**: a miniature desktop rendered with the same `themeStyle()` the real desktop
  uses. Hover it to try your pointer; click it to try ripples.
- **Instant or Draft**: changes apply as you make them (with Undo/Redo, Ctrl+Z) or wait in the
  preview until Apply. **Advanced** adds exact number and hex fields.
- **Favorites** (star any setting), **Library** of built-in and saved looks, share a look as text or
  a `.novalook.json` file in Nova Files → Documents (custom wallpapers are not included).
- Real effects only: custom SVG pointer, click ripples, synthesized UI sounds (off by default),
  window shadow/title bar/border/animation/speed, floating taskbar, icon labels and spacing, fonts,
  and per-app pinning to the taskbar and desktop (Start-menu buttons, right-click, or Studio).
- Settings are validated by `sanitize()` (schema v2); old `unpinned` data migrates automatically.

## NovaOS's own pointer, and community links

- **Pointer engine** (Nova Studio → Cursor): fullscreen webviews can freeze or hide the system
  pointer, especially custom image cursors. In **Immersive Mode** NovaOS now hides the native
  pointer and draws its own (`shell/NovaCursor.tsx`), following every pointer move with the right
  shape (arrow, I-beam over text, hand over links, resize arrows) in your chosen style/color/size.
  Safety: the native pointer is only hidden after NovaOS's has a position to draw, and focus loss
  never hides it. Options: Automatic (Immersive only, default), Always NovaOS, Windows.
- **Links**: a few quiet "Join the Discord" links where they belong (Settings → About, Nova Studio
  Home, Library). The UI never passes a URL to Rust; it names a key and Rust opens only the
  allow-listed address (`open_link`, `src-tauri/src/lib.rs`, `src/links.ts`).
