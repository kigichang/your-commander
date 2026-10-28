# Your Commander

A dual-pane file manager for macOS in the spirit of Total Commander, built with
Tauri 2 (Rust) and React. Two independent panels, keyboard-first workflow,
remote filesystems, and a fast Quick View for the files you actually work with.

> Status: personal project under active development. Apple Silicon build,
> ad-hoc signed.

## Features

**Dual panels, keyboard first**
- Left/right panels with tabs, per-panel sort (name/ext/size/date), brief/full
  view, draggable column widths, wildcard filtering in the editable path bar
- F-keys work without holding `fn` while the app is focused (IOKit, togglable);
  two hotkey presets (Commander / Finder-like) with a full rebinding editor
- Command palette (`⌘K`) covering every action plus recent/favorite folders
- Context menu on rows and blank space, following Finder marking conventions

**File operations**
- Copy / move with background queue: progress, ETA, pause/resume, cancel,
  sticky conflict policies (ask / skip / replace older / keep both), optional
  byte-verify, metadata & Finder-tag preservation, wildcard masks
- Trash or permanent delete; multi-rename with masks, regex, counters and undo
- Symbolic links are preserved end-to-end (list, copy, archive, SFTP)

**Archives**
- Create and extract `.zip`, `.tar`, `.tar.gz`/`.tgz`, `.gz` (7z: extract);
  browse zip/tar archives like folders and copy files straight out of them
- macOS noise (`.DS_Store`, `._*`) is excluded automatically

**Remote**
- SMB: mounts via NetFS into `/Volumes` (shared with Finder), saved
  connections, passwords in the macOS Keychain, share picker when the share
  is left empty
- SFTP: password / key / ssh-agent auth, reads `~/.ssh/config`, saved
  connections, full transfer and sync support

**Quick View (F3)**
- The opposite panel becomes a live preview following the cursor: images,
  text/code, Markdown (highlighting, KaTeX, Mermaid), PlantUML, PDF
- PlantUML rendering is **off by default**: diagram text never leaves the
  machine unless you opt into server or local-jar rendering in Settings

**Tools**
- Folder sync (1:1 compare by size/date or content, per-row direction)
- File search with name/content/date/size filters, streaming results
- Compare two files by content (side-by-side diff), binary-safe

**UI**
- Graphite monochrome theme, dark and light, follows the system by default
- English and Traditional Chinese (zh-TW)

## Install / Build

Requirements: macOS 11+, [Rust](https://rustup.rs), Node.js 20+.

```bash
npm install
npm run tauri dev     # development (Vite + Tauri)
npm run tauri build   # release: .app + .dmg under src-tauri/target/release/bundle/
```

Tests and checks:

```bash
cd src-tauri && cargo test   # backend unit tests
npx tsc -b                   # frontend type check
```

## Keyboard (Commander preset)

| Key | Action |
|---|---|
| `Tab` | Switch panel |
| `F3` | Quick View |
| `F5` / `F6` | Copy / move to the other panel |
| `F7` | New folder |
| `F8` | Move to Trash |
| `⌘K` | Command palette |
| `⌘F` | Find files |
| `⌘,` | Settings (hotkeys are fully rebindable) |

## Architecture

- `src-tauri/` — Rust backend: filesystem providers (local, SFTP via
  russh/russh-sftp, archive), operation queue with cancellation/pause and
  conflict channels, NetFS SMB mounting, NSWorkspace file icons, Keychain,
  directory watcher
- `src/` — React 18 + TypeScript frontend: Zustand stores, TanStack Virtual
  lists, i18next, lucide icons
- Design document: `File Manager Screens.html`; implementation notes: `PLAN.md`

## License

[MIT](LICENSE)
