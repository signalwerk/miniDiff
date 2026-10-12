# MiniDiff

A native diff and merge tool for **text files and folders**, built in Rust with
[egui](https://github.com/emilk/egui) and syntax-highlighted with
[tree-sitter](https://tree-sitter.github.io/).

**[Download for macOS](https://github.com/signalwerk/miniDiff/releases/latest/download/MiniDiff-macos-universal.zip)**
· [Homepage](https://minidiff.signalwerk.ch/)

> MiniDiff was created with the assistance of LLM agents.

## Features

- **File diff**: side-by-side or unified view.
  - Tree-sitter syntax highlighting for Rust, JS/TS/TSX, Python, Go, C/C++, Java, Ruby, HTML, CSS, JSON, TOML, YAML, Markdown and shell.
  - Word-level change emphasis.
  - Collapsible unchanged regions and an "ignore whitespace" option.
  - A change-overview strip that also works as the scrollbar.
- **Folder diff**: a tree of changed, added, removed and identical files, with filters and search. Select a file to see its diff; step between changed files with `Mod+↓` / `Mod+↑` (see [Keyboard](#keyboard)).
  Click a **Show files** tag to toggle its status, or `Mod+click` to show only that status.
- **Merge**: a three-way merge (diff3) with columns A · Result · B (plus Base on request).
  - Non-conflicting changes are merged automatically.
  - For each conflict you choose A, B, A+B, B+A or Base, or edit the result by hand.
  - It also reads files that already contain git conflict markers.
- **Ways in**:
  - Drop two items onto the window. On macOS, you can also drop them onto the app icon.
  - Keep several comparisons open in separate windows. Dropping new items opens
    another view; an available start screen is reused. Use **File → New Window**
    on macOS, or `Mod+N` for a new start screen, and `Mod+W` to close the current window.
    On macOS, closing the last window keeps MiniDiff running. Click its Dock icon
    or choose **File → New Window** to reopen it; use **Quit MiniDiff** (`⌘Q`) to exit.
    Git/Tower merge-tool launches still exit when their last window closes.
  - Use the command line (`minidiff a b`).
  - Configure it as the git or Tower diff/merge tool.
- **Self-updating macOS bundles** from GitHub Releases.

## Scope

For now MiniDiff is about **text diffing and merging**. Binary files are not
supported: MiniDiff only reports whether they are identical. Images might become
an exception in the future, but no requests for them are accepted at the moment.

**Platforms:** MiniDiff uses a cross-platform Rust UI toolkit. Published releases
currently provide macOS builds; Linux and Windows ports and testing are welcome.
[Open a GitHub issue](https://github.com/signalwerk/miniDiff/issues) to request a
build, report what works on your platform, or suggest a porting change through
the [prompt contribution workflow](#contributing).

## Install

### macOS

1. Download [MiniDiff-macos-universal.zip](https://github.com/signalwerk/miniDiff/releases/latest/download/MiniDiff-macos-universal.zip) from [GitHub Releases](https://github.com/signalwerk/miniDiff/releases/latest) (Apple Silicon and Intel, macOS 11+).
2. Unzip it and move the app to `/Applications/MiniDiff.app`.
3. The app is not notarized, so open it the first time with right-click → **Open**.

The command-line executable is `/Applications/MiniDiff.app/Contents/MacOS/minidiff`.
See [Command line](#command-line) to make it available as `minidiff` on your `PATH`.

On startup MiniDiff checks for new releases. When one is available, an
**Update** button appears in the app bar: it downloads the release, checks its
SHA-256, replaces the app and restarts it. Automatic checks can be turned off
in **MiniDiff → Preferences…** (shown as **Settings…** on recent macOS versions).

### Other platforms

Prebuilt Linux and Windows releases are not currently published. To try a source
build, install Rust and your platform's native build tools, then run:

```sh
cargo build --release
```

The executable is `target/release/minidiff` (`minidiff.exe` on Windows).
Native dependencies and platform integration may need additional work; please
share build issues and testing results in a GitHub issue. The bundle installer
and self-updater currently target macOS.

## Use from Tower

The included Tower installer targets macOS:

```sh
scripts/install-tower.sh
```

Restart Tower, then go to **Settings → Git Config** and set the **Diff Tool** and
the **Merge Tool** to *MiniDiff*. Tower calls
[`minidiff.sh`](integrations/tower/minidiff.sh) with
`LOCAL REMOTE [BASE MERGED]`. As a merge tool, MiniDiff shows **Save & Close**
and **Cancel**, and exits with code 0 only when the result was saved. Otherwise
Tower keeps the file marked as conflicted.

## Use from git

With `minidiff` on your `PATH`, add the following to your git configuration.
Alternatively, replace `minidiff` in each `cmd` with the path to your executable
(quote paths containing spaces).

```ini
# ~/.gitconfig
[diff]
    tool = minidiff
[difftool "minidiff"]
    cmd = minidiff \"$LOCAL\" \"$REMOTE\"
[merge]
    tool = minidiff
[mergetool "minidiff"]
    cmd = minidiff --merge \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\"
    trustExitCode = true
```

`git difftool -d` passes two folders, which opens the folder view.

## Command line

Run the executable directly or add its directory to your `PATH` to use the
`minidiff` command (`minidiff.exe` on Windows).

On macOS, after installing the GitHub release, you can run
`/Applications/MiniDiff.app/Contents/MacOS/minidiff` directly. To use the shorter
`minidiff` command, create a link in `~/.local/bin` and add that directory to your
`PATH`:

```sh
mkdir -p "$HOME/.local/bin"
ln -sf /Applications/MiniDiff.app/Contents/MacOS/minidiff "$HOME/.local/bin/minidiff"
```

```sh
minidiff a.rs b.rs                       # compare files
minidiff dir1 dir2                       # compare folders
minidiff conflicted.rs                   # resolve conflict markers in place
minidiff --merge LOCAL REMOTE BASE MERGED
minidiff --merge --local L --remote R --base B --output M
```

For a macOS source build, `scripts/bundle-macos.sh --install` installs the app and creates
the same command-line link.

## Keyboard

`Mod` means Command (`⌘`) on macOS and Ctrl on other platforms. `Alt` means
Option (`⌥`) on macOS and Alt elsewhere.

| Action | Keys |
| --- | --- |
| Next / previous change | `n` `p` (or `Alt+↓` `Alt+↑`, `j` `k`) |
| Next / previous file (folder view) | `Mod+↓` `Mod+↑` |
| Side by side ⇄ unified / collapse / ignore whitespace | `u` / `c` / `w` |
| Font size | `+` `−` |
| Toggle file list | `b` |
| Merge: take A / B / A+B / B+A / base | `a` `b` / `3` `4` / `0` |
| Merge: edit / reset chunk | `e` / `r` |
| Save merge | `Mod+S` |
| New window / close window | `Mod+N` / `Mod+W` |
| Swap sides / reload / start screen | `Mod+Shift+S` / `Mod+R` / `Mod+O` |
| Preferences (appearance and automatic updates) | `Mod+,` |

## Contributing

**Code contributions (pull requests) are not accepted.** Prompt
contributions are welcome instead:

1. [Open an issue](https://github.com/signalwerk/miniDiff/issues) and describe
   the change as a prompt for an LLM coding agent: what should change, why,
   and how to verify it.
2. If the maintainer accepts it, they run the prompt and commit the result.

Please keep requests within the [scope](#scope) above.

## Development

```sh
cargo run -- samples/left samples/right     # compare folders
cargo run -- samples/merge/conflicted.rs    # resolve conflict markers
cargo build --release
cargo test
cargo clippy
```

For macOS app bundles:

```sh
scripts/bundle-macos.sh                     # → target/release/bundle/MiniDiff.app
scripts/bundle-macos.sh universal           # arm64 + x86_64 (rustup target add x86_64-apple-darwin)
```

| Path | What |
| --- | --- |
| `src/diff.rs` | Line diff with paired modifications and word-level emphasis |
| `src/merge.rs` | diff3 three-way merge, two-way fallback, conflict-marker parser |
| `src/folder.rs` | Recursive folder comparison (runs on a background thread) |
| `src/highlight.rs` | Tree-sitter highlighting split into per-line spans |
| `src/update.rs` | Self-updater (manifest on GitHub Pages, assets on GitHub Releases) |
| `src/ui/` | egui views: welcome, file, folder, merge; shared code-line renderer |
| `src/platform/` | Platform integration; currently macOS menus and "open documents" Apple Events (Dock icon drops) |
| `site/` | Landing page deployed to [minidiff.signalwerk.ch](https://minidiff.signalwerk.ch) |
| `samples/` | Fixtures, also embedded as the in-app demo |

Ignored when walking folders: `.git`, `.DS_Store`, `node_modules`, `.hg`, `.svn`.

### Icon assets

On macOS, install the SVG renderer and regenerate the app and website assets:

```sh
brew install resvg
scripts/make-icons.sh
```

`assets/icon.svg` is the source. The script uses `resvg` to render the PNGs,
packages the macOS `.icns` with `iconutil`, copies the SVG for website logos and
the favicon, and reads the `deleted-page` and
`added-page` fills into generated Rust colour constants. The app derives its
red/green diff colours from those fills and uses the rendered icon in its UI.
Python 3 and the macOS command-line tools are also required. Run the script after
editing the SVG and commit the generated assets; release builds use them directly.

### Landing-page screenshots

On macOS, regenerate all three images with:

```sh
scripts/screenshots.py
# Optional: choose the actual app window size (in points).
scripts/screenshots.py --width 880 --height 520
# Reuse a binary that supports MINIDIFF_STORAGE_PATH:
scripts/screenshots.py --binary target/debug/minidiff
```

The script builds the app, opens each example in a temporary bundle with isolated
settings (dark theme, 13-point code), captures an 880×520-point content window
on the internal Retina display at 2× pixel density, and updates `site/img/` plus
the HTML image dimensions. It verifies the display and pixel density before
saving. Use `--display main` to select the main monitor instead. It needs Python 3, the
Xcode command-line tools, and Screen Recording permission for the terminal.
Existing app windows and personal settings are left alone. The landing page
shows these narrow-window captures wider than its text column.

### Releases

The current release pipeline publishes macOS bundles:

```sh
scripts/release.sh patch        # or minor / major / 1.2.3
```

The script bumps the version in `Cargo.toml`, runs the tests, and captures fresh
file, folder, and merge screenshots before asking to commit and publish. It includes
the images and their HTML dimensions in the release commit, creates the tag `vX.Y.Z`,
then pushes `main` and the tag. Screenshot capture must succeed before the release
can proceed; run this on macOS with Screen Recording permission and the internal
Retina display available. To capture on the main monitor instead, use
`MINIDIFF_SCREENSHOT_DISPLAY=main scripts/release.sh patch`.
Only version tags trigger
[`.github/workflows/release.yml`](.github/workflows/release.yml). That workflow:

1. builds the universal `MiniDiff.app` and attaches
   `MiniDiff-macos-universal.zip` and its `.sha256` to a GitHub Release;
2. deploys the landing page (`site/`) and `update.json` directly to GitHub Pages
   through the release workflow.

Use **MiniDiff → About MiniDiff** for version information and **MiniDiff → Check
for Updates…** to check manually. **MiniDiff → Preferences…** (`⌘,`) lets you
choose System, Light, or Dark appearance and turn automatic updates on or off.
Recent macOS versions label that menu item **Settings…**.
**Help → Keyboard Shortcuts** shows the available shortcuts. These are in the
macOS menu bar and File actions apply to the focused window.
Automatic checks run on startup and every 24 hours
while the app is open. Automatic checks are
paused when only merge views are open. Available updates offer installation and
restart in the app bar, with unsaved merges protected before restart.

The app reads `https://minidiff.signalwerk.ch/update.json` to find new versions.
Release assets must be downloadable without login, so the repository has to be
public for self-updates to work.

## License

[MIT](LICENSE)
