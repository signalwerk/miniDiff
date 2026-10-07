# AGENTS.md

Context for coding agents working on MiniDiff. **Keep this file current.** Update it
in the same change whenever you make a meaningful change, clarify an assumption,
or learn something non-obvious about the project. Remove anything that has
become stale. Keep it short; don't duplicate what the code or README says plainly.

## What this is

A native macOS diff/merge tool for **text files and folders**: Rust, egui/eframe 0.36,
tree-sitter highlighting. Three modes: file diff, folder diff (tree + file diff),
merge (diff3, conflict resolution). User-facing docs live in `README.md`; the
landing page is in `site/`.

## Decisions (don't undo without the maintainer)

- **Native only.** The web/wasm target was removed on purpose. Don't reintroduce
  trunk, wasm cfgs or web glue.
- **Scope:** text diff and merge only. Binary files just show "identical / differ".
  The image preview was removed on purpose; images may come later, but not now.
- **Contributions:** no code PRs, only prompt contributions in issues, which the
  maintainer runs. The project states it is created with the assistance of LLM agents.
- **Platforms:** releases are macOS-only (universal). Linux/Windows only on request.
- **License:** MIT (`LICENSE`).

## Commands

```sh
cargo run -- samples/left samples/right      # folder diff
cargo run -- a.rs b.rs                       # file diff
cargo run -- samples/merge/conflicted.rs     # conflict markers → merge view
cargo run -- --merge LOCAL REMOTE BASE MERGED
cargo test                                   # unit tests (diff, merge, highlight, updater)
cargo clippy                                 # keep at zero warnings
scripts/bundle-macos.sh [universal] [--install]   # → target/release/bundle/MiniDiff.app
scripts/release.sh patch|minor|major|X.Y.Z        # bump, tag vX.Y.Z, push → CI release
scripts/make_icon.py                              # regenerate assets/icon-*.png (stdlib only)
```

## Layout

- `src/main.rs`: CLI (clap) → `app::Launch`; installs the macOS open handler; exits with `platform::exit_code()`.
- `src/app.rs`: screens (Welcome / File / Folder / Merge), routing of drops and
  args, app bar, persisted `Settings`, updater UI, close/exit-code handling.
- `src/diff.rs`: line diff (`similar`, patience by default); deletions are paired
  with the following insertions, and paired lines get a word diff (≥20% unchanged).
- `src/merge.rs`: diff3 (`three_way`), `two_way` fallback, conflict-marker
  parser, `trim_chunks` (moves lines shared by all three sides out of change chunks).
- `src/folder.rs`: recursive comparison (background thread); `src/source.rs`
  `Entry` = filesystem path or in-memory `MemNode` (used by `src/demo.rs`, which
  embeds `samples/`). `IGNORED_NAMES` lists the skipped folder names.
- `src/highlight.rs`: language table + capture-name → `Hl` mapping. The whole
  file is highlighted, then split into per-line spans.
- `src/ui/`: `file_view.rs`, `folder_view.rs`, `merge_view.rs`, `welcome.rs`, `code.rs`
  (shared line painter), `mod.rs` (`ViewSettings`, small widgets).
- `src/platform/`: global inbox for OS-delivered files, exit code; `macos.rs`
  Apple Event handler.
- `src/update.rs`: self-updater. `src/theme.rs`: palettes (order of `Hl` must
  match the syntax colour arrays).
- `integrations/tower/`, `scripts/install-tower.sh`: Tower custom tool
  (`LOCAL REMOTE [BASE MERGED]`).
- `site/`: landing page; `.github/workflows/release.yml`: release pipeline.

## Gotchas

- **egui 0.36 API** differs from older docs: `App::ui(&mut self, ui, frame)`,
  `egui::Panel::left/top(id).show(ui, …)`, `CentralPanel::no_frame()`,
  `ctx.fonts_mut`, `ui.close()`, `ctx.global_style()`. Check the registry source
  when unsure.
- **Diff/merge bodies are custom-painted** inside `ScrollArea::show_rows` with
  fixed row height and `item_spacing.y = 0`. Horizontal scroll is our own
  `h_off`; the right-edge overview strip doubles as the scrollbar.
- **Glyphs:** the UI font falls back to Hack (added in `install_fonts`); on macOS SF Pro / SF Mono are
  loaded from `/System/Library/Fonts`. Before using a new symbol, verify a bundled
  font has it, or it renders as tofu.
- **Dock / Finder drops** arrive as an `aevt/odoc` Apple Event, which winit doesn't
  expose. `platform::macos::install_open_handler()` registers on
  `NSApplicationWillFinishLaunchingNotification`; it must run before `run_native`.
  Test with `open -a target/release/bundle/MiniDiff.app a b`.
- **Merge-tool exit code:** 1 until the result is saved and unchanged since
  (`MergeView.saved && !unsaved`); git/Tower rely on it. No update check in merge mode.
- **Grammar crates:** they must match tree-sitter 0.27, and their constant names vary
  (`HIGHLIGHT_QUERY` vs `HIGHLIGHTS_QUERY`). After upgrading, `all_queries_compile` must pass.
- **Persisted settings** live in `~/Library/Application Support/MiniDiff/app.ron`
  (written on a clean exit only).
- **Visual checks:** run the binary, then `screencapture -l <windowID>`. Get the
  window ID via `CGWindowListCopyWindowInfo` (a small Swift helper).

## Release & update pipeline

- Only tags `vX.Y.Z` trigger CI; the tag must equal the `Cargo.toml` version
  (`scripts/release.sh` handles this).
- CI builds a universal app, uploads the fixed-name `MiniDiff-macos-universal.zip` and its `.sha256` to the
  GitHub Release, and deploys `site/` + generated `update.json` + CNAME
  `minidiff.signalwerk.ch` to `gh-pages` (`force_orphan`).
- The app fetches `https://minidiff.signalwerk.ch/update.json`, verifies the SHA-256, swaps the
  `.app` via `ditto` next to the bundle, then relaunches with `open -n`.
- **Current state (2026-10-07):** the repo is private, so updates and the download link
  won't work until it is public. No release has been cut yet; GitHub Pages must be
  enabled once after the first release (source: `gh-pages`). The app is ad-hoc
  signed, not notarized.
