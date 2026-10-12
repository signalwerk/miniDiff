# AGENTS.md

Context for coding agents working on MiniDiff. **Keep this file current.** Update it
in the same change whenever you make a meaningful change, clarify an assumption,
or learn something non-obvious about the project. Remove anything that has
become stale. Keep it short; don't duplicate what the code or README says plainly.

## What this is

A native macOS diff/merge tool for **text files and folders**: Rust, egui/eframe 0.36,
wgpu (Metal on macOS), tree-sitter highlighting. Three modes: file diff, folder diff (tree + file diff),
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
scripts/make-icons.sh                             # SVG → app/site icons + colours (resvg, macOS)
scripts/screenshots.py                            # macOS native captures → site/img/ (880×520 pt)
```

## Layout

- `src/main.rs`: CLI (clap) → `app::Launch`; installs the macOS open handler; exits with `platform::exit_code()`.
- `src/app.rs`: screens (Welcome / File / Folder / Merge), routing of drops and
  args, deferred native viewports with independent `DiffWindow` state and shared settings /
  updater, app bar, close/exit-code handling. New opens reuse a Welcome window or
  create a viewport; drops never replace an active comparison or merge.
- `src/diff.rs`: line diff (`similar`, patience by default); deletions are paired
  with the following insertions, and paired lines get a word diff (≥20% unchanged).
- `src/merge.rs`: diff3 (`three_way`), `two_way` fallback, conflict-marker
  parser, `trim_chunks` (moves lines shared by all three sides out of change chunks).
- `src/folder.rs`: recursive comparison (background thread); `src/source.rs`
  `Entry` = filesystem path or in-memory `MemNode` (used by `src/demo.rs`, which
  embeds `samples/`). `IGNORED_NAMES` lists the skipped folder names.
  Welcome offers Files, Folders, and Merge demo buttons; the file demo compares
  the embedded TypeScript `format.ts` pair without requiring files on disk.
  That pair includes an indentation-only change in `formatId` and a long shared
  helper block so Whitespace and Collapse have visible effects in Files/Folders demos.
  Embedded demos must never enter Recent; only filesystem comparisons are recorded.
  Merge demos are explicitly marked disposable: no Save button/shortcut or unsaved
  guard on close, Home, Quit, or updater restart. Real merges retain their save guards.
- `src/highlight.rs`: language table + capture-name → `Hl` mapping. The whole
  file is highlighted, then split into per-line spans.
- `src/ui/`: `file_view.rs`, `folder_view.rs`, `merge_view.rs`, `welcome.rs`, `code.rs`
  (shared line painter), `mod.rs` (`ViewSettings`, small widgets).
- `src/platform/`: global inbox for OS-delivered files, exit code; `macos.rs`
  Apple Event handler.
- `src/update.rs`: self-updater. `src/theme.rs`: palettes (order of `Hl` must
  match the syntax colour arrays).
- `assets/icon.svg`: icon source; `deleted-page` / `added-page` fills generate
  `assets/icon-colors.rs` for semantic diff colours. `scripts/make-icons.sh` uses
  resvg and iconutil; commit generated PNGs/ICNS so CI needs no renderer.
  Native bundles, welcome logos, and website icons share these assets.
  The website uses the copied SVG directly for its logo and favicon.
- `integrations/tower/`, `scripts/install-tower.sh`: Tower custom tool
  (`LOCAL REMOTE [BASE MERGED]`).
- `site/`: landing page; `.github/workflows/release.yml`: release pipeline.
  Installation docs use GitHub Releases; Tower setup details belong in the README.
  Website copy stays platform-neutral; macOS appears only on the download button
  and its metadata line. Platform-specific installation steps live in the README.
  README usage is platform-neutral (`Mod` = Command on macOS, Ctrl elsewhere);
  distinguish published macOS bundles/integrations from unverified source ports.
  GitHub description / homepage and the README link to `https://minidiff.signalwerk.ch/`.
  Final style: Midnight, IBM Plex Mono headings / Work Sans body via
  `fonts.signalwerk.ch` (see signalwerk/fonts.cdn); no custom letter spacing or
  style lab. GitHub links open in a new tab without arrow glyphs. LLM attribution
  stays in the README; the page footer shows only the name and license.
  Keep captions and download metadata as readable as the footer text.
  Keep the page short, with no privacy
  page. Screenshots have task headings, use narrow native windows, stack, and extend beyond the text
  column (1200px maximum), with images directly in figures and no decorative
  screenshot frames. `scripts/screenshots.py` regenerates them and their
  HTML dimensions using temporary bundles and isolated dark-theme profiles.
  Captures default to the internal Retina display and verify 2× pixel density;
  persisted global window positions must be multiplied by that display’s scale.

## Gotchas

- **egui 0.36 API** differs from older docs: `App::ui(&mut self, ui, frame)`,
  `egui::Panel::left/top(id).show(ui, …)`, `CentralPanel::no_frame()`,
  `ctx.fonts_mut`, `ui.close()`, `ctx.global_style()`. Check the registry source
  when unsure.
- **Diff/merge bodies are custom-painted** inside `ScrollArea::show_rows` with
  fixed row height and `item_spacing.y = 0`. Horizontal scroll is our own
  `h_off`; the right-edge overview strip doubles as the scrollbar.
  Hover fills overlay text; use `from_rgba_unmultiplied_const` for translucent
  palette colours so an unscaled white RGB value cannot wash out the line.
- **Windows:** closing the original window hides eframe's root viewport while
  other windows remain. On macOS closing the last window keeps the app running;
  Dock reopen/New Window/OS opens restore the root. Explicit Quit, updater restart,
  and CLI merge-tool sessions still exit. Cancelling an unsaved close cancels Quit
  and wakes the root controller even from a deferred window. Unsaved merge
  confirmation belongs to its window. OS opens and all window close guards run in
  `App::logic` too (background/occluded windows can skip `App::ui`). File menu /
  `⌘N` creates a new window.
  File/folder pickers use asynchronous rfd sheets created on the main thread;
  poll their per-window results in logic, with a repaint waker. Blocking `runModal`
  inside the UI callback re-enters winit on focus/drag events and aborts on macOS.
  Welcome has one Choose button per side using `pick_file_or_folder`; rfd only
  supports the combined picker on macOS (source ports currently fall back to files).
- **Renderer:** use wgpu for native windows. The OpenGL/glutin backend can panic
  when disposing of child windows while the root viewport is hidden on macOS.
  Keep eframe's `links` feature enabled (defaults are disabled) so hyperlinks
  and `OpenUrl` actions launch the user's browser.
- **Button layout:** egui 0.36 subtracts the theme border width from padding
  before applying explicit button strokes. Keep theme border widths equal across
  states and frameless icon / Recent button strokes at zero to avoid hover shifts.
- **Glyphs:** the UI font falls back to Hack (added in `install_fonts`); on macOS SF Pro / SF Mono are
  loaded from `/System/Library/Fonts`. Before using a new symbol, verify a bundled
  font has it, or it renders as tofu.
- **Dock / Finder drops** arrive as an `aevt/odoc` Apple Event, which winit doesn't
  expose. `platform::macos::install_open_handler()` registers on
  `NSApplicationWillFinishLaunchingNotification`; it must run before `run_native`.
  Test with `open -a target/release/bundle/MiniDiff.app a b`.
- **Merge-tool exit code:** 1 until the result is saved and unchanged since
  (`MergeView.saved && !unsaved`); git/Tower rely on it. Automatic update checks
  pause while only merge views are open; explicit File-menu checks still work.
- **Menus:** `src/platform/menu.rs` installs one AppKit menu bar on the main
  thread after winit initialization. Native actions enter a logic inbox and target
  the last focused viewport (including when the original window is hidden).
  MiniDiff menu: About, Preferences, Check for Updates; File: window actions;
  Help: Keyboard Shortcuts. Appearance and automatic-update settings live in
  Preferences (`⌘,`). No per-window File/help menu, branding, or theme button.
  Recent macOS versions automatically display Preferences as Settings in the menu.
  Welcome omits the empty app bar; it appears there only for update controls or a manual check.
- **Information windows:** Preferences and Shortcuts are app-wide singleton native windows,
  centered by AppKit on each open and independent of comparisons (including a hidden root).
  Wake/focus the root controller until their first frame is created, then hide it again;
  focus the utility after that wake-up. Center within the display's visible frame and clamp size.
  They share About's padding, heading style, and right-aligned Close footer.
  Shortcuts are grouped by task with readable key badges; only the list scrolls,
  keeping the heading and footer visible in small windows.
  Its scrollbar appears only when the list overflows the available height.
  Preferences uses separate Appearance/Updates sections and the shared segmented control.
  Its Check for Updates button uses the manual updater, reports status in the dialog,
  and stays disabled while checking/installing or awaiting restart.
  The unsaved-merge warning shares that frame/footer, identifies the file, and places
  primary Save on the right of Cancel/Discard. Save failures remain visible in the warning.
  Unresolved-conflict confirmation shares `ui::dialog_modal`/`dialog_footer`, identifies
  the output file, explains conflict-marker output, and makes Keep resolving primary.
  Failed Save anyway attempts leave the warning open with the error visible.
  Merge Save buttons use plain text without a save icon.
- **Folder filters:** leave space around search; use full status names on chips.
  Normal clicks toggle one status; Mod-click enables only that status (Changed includes errors).
  Group Changed / Identical separately from Only in A / Only in B, wrapping in narrow sidebars.
  File and folder app bars have Swap on the left and Reload on the right, without comparison breadcrumbs.
  Merge has the same Swap button/shortcut for Local and Remote; resolutions and custom edits
  retain their output. Swapping unresolved marker blocks changes output and marks it unsaved.
  Merge inline emphasis reuses file-diff line pairing/word diffs, against Base in three-way
  merges or A/B without Base. Cached ranges follow source lines into the result and swap with inputs.
  Merge uses a gentle spotlight and step counter for the current change. Included inputs
  have labeled controls and full-chunk outlines (both for A+B/B+A, Base when selected).
  Clicking the unresolved-conflict count jumps to the next unresolved conflict, wrapping to the first.
  Side chips reserve both Use/Included label widths and keep a fixed inset from the outline,
  so selection does not resize or move them.
  Don't stack unused-input dimming with the spotlight on unfocused chunks.
  Comparison colours are shared: A/Local/removals red, B/Remote/additions green,
  identical/Base/neutral result labels gray, changes/warnings amber, focus/manual edits blue.
  Diff and merge overview markers use the same painter: A red, B green, both split red/green.
  They describe changed inputs, never resolution status; result lines retain their source colours.
- **Updates:** MiniDiff menu has an About version modal and a manual update check.
  Automatic checks run on startup and every 24 hours (shared across windows,
  monotonic clock, logic-frame timer); respect `auto_update` and never interrupt
  checking/installing/installed states. Restart guards unsaved merges in all windows.
- **Grammar crates:** they must match tree-sitter 0.27, and their constant names vary
  (`HIGHLIGHT_QUERY` vs `HIGHLIGHTS_QUERY`). After upgrading, `all_queries_compile` must pass.
- **Persisted settings** live in `~/Library/Application Support/MiniDiff/app.ron`
  (written on a clean exit only).
- **Capture profiles:** `MINIDIFF_STORAGE_PATH` overrides eframe’s RON file path
  for screenshot automation. Set initial sizes in the isolated profile; resizing
  an occluded Metal window can capture a stretched, stale frame.
- **Visual checks:** run the binary, then `screencapture -l <windowID>`. Get the
  window ID via `CGWindowListCopyWindowInfo` (a small Swift helper).

## Release & update pipeline

- Only tags `vX.Y.Z` trigger CI; the tag must equal the `Cargo.toml` version
  (`scripts/release.sh` handles this).
  The release script must refresh all three website screenshots before confirmation,
  commit their PNGs/HTML dimensions with the version, and stop if capture fails.
  `MINIDIFF_SCREENSHOT_DISPLAY=main` selects an external/main monitor (default: internal).
- CI builds a universal app, uploads the fixed-name `MiniDiff-macos-universal.zip` and its `.sha256` to the
  GitHub Release, and deploys `site/` + generated `update.json` + CNAME
  `minidiff.signalwerk.ch` directly with the official Pages actions. Pages uses
  Actions (`build_type: workflow`), with `pages: write` / `id-token: write`.
  The `github-pages` environment allows the `main` branch and `v*` tags.
- The app fetches `https://minidiff.signalwerk.ch/update.json`, verifies the SHA-256, swaps the
  `.app` via `ditto` next to the bundle, then relaunches with `open -n`.
- The repository is public and the custom domain has enforced HTTPS.
- The app is ad-hoc signed, not notarized. `cargo test published_release -- --ignored`
  verifies the public manifest, checksum, universal architectures, and real bundle replacement in a temporary
  directory after publishing a release (requires network and macOS).
