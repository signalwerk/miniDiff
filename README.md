# MiniDiff

A native diff and merge tool for **text files and folders**, built in Rust with
[egui](https://github.com/emilk/egui) and syntax-highlighted with
[tree-sitter](https://tree-sitter.github.io/).

**[Download for macOS](https://github.com/signalwerk/miniDiff/releases/latest/download/MiniDiff-macos-universal.zip)**
· [Website](https://minidiff.signalwerk.ch)

> MiniDiff was created with the assistance of LLM agents.

## Features

- **File diff**: side-by-side or unified view.
  - Tree-sitter syntax highlighting for Rust, JS/TS/TSX, Python, Go, C/C++, Java, Ruby, HTML, CSS, JSON, TOML, YAML, Markdown and shell.
  - Word-level change emphasis.
  - Collapsible unchanged regions and an "ignore whitespace" option.
  - A change-overview strip that also works as the scrollbar.
- **Folder diff**: a tree of changed, added, removed and identical files, with filters and search. Select a file to see its diff; step between changed files with `⌘↓` / `⌘↑`.
- **Merge**: a three-way merge (diff3) with columns A · Result · B (plus Base on request).
  - Non-conflicting changes are merged automatically.
  - For each conflict you choose A, B, A+B, B+A or Base, or edit the result by hand.
  - It also reads files that already contain git conflict markers.
- **Ways in**:
  - Drop two items onto the window or the app icon.
  - Use the command line (`minidiff a b`).
  - Configure it as the git or Tower diff/merge tool.
- **Self-updating** from GitHub Releases.

## Scope

For now MiniDiff is about **text diffing and merging**. Binary files are not
supported: MiniDiff only reports whether they are identical. Images might become
an exception in the future, but no requests for them are accepted at the moment.

**Platforms:** macOS builds are published with every release. Linux and Windows
builds can be provided if beta testers
[open a GitHub issue](https://github.com/signalwerk/miniDiff/issues) and give feedback.

## Install

1. Download [MiniDiff-macos-universal.zip](https://github.com/signalwerk/miniDiff/releases/latest/download/MiniDiff-macos-universal.zip) (Apple Silicon and Intel, macOS 11+).
2. Unzip it and move `MiniDiff.app` to `/Applications`.
3. The app is not notarized, so open it the first time with right-click → **Open**.

On startup MiniDiff checks for new releases. When one is available, an
**Update** button appears in the title bar: it downloads the release, checks its
SHA-256, replaces the app and restarts it. Automatic checks can be turned off
in the **?** menu.

## Use from Tower

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

```ini
# ~/.gitconfig
[diff]
    tool = minidiff
[difftool "minidiff"]
    cmd = /Applications/MiniDiff.app/Contents/MacOS/minidiff \"$LOCAL\" \"$REMOTE\"
[merge]
    tool = minidiff
[mergetool "minidiff"]
    cmd = /Applications/MiniDiff.app/Contents/MacOS/minidiff --merge \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\"
    trustExitCode = true
```

`git difftool -d` passes two folders, which opens the folder view.

## Command line

```sh
minidiff a.rs b.rs                       # compare files
minidiff dir1 dir2                       # compare folders
minidiff conflicted.rs                   # resolve conflict markers in place
minidiff --merge LOCAL REMOTE BASE MERGED
minidiff --merge --local L --remote R --base B --output M
```

`scripts/bundle-macos.sh --install` installs the app and links the
command to `~/.local/bin/minidiff`.

## Keyboard

| Action | Keys |
| --- | --- |
| Next / previous change | `n` `p` (or `⌥↓` `⌥↑`, `j` `k`) |
| Next / previous file (folder view) | `⌘↓` `⌘↑` |
| Side by side ⇄ unified / collapse / ignore whitespace | `u` / `c` / `w` |
| Font size | `+` `−` |
| Toggle file list | `b` |
| Merge: take A / B / A+B / B+A / base | `a` `b` / `3` `4` / `0` |
| Merge: edit / reset chunk | `e` / `r` |
| Save merge | `⌘S` |
| Swap sides / reload / start screen | `⌘⇧S` / `⌘R` / `⌘O` |
| Theme (auto / light / dark) | `⌘⇧L` |

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
cargo test
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
| `src/platform/` | macOS "open documents" Apple Event handler (Dock icon drops) |
| `site/` | Landing page deployed to [minidiff.signalwerk.ch](https://minidiff.signalwerk.ch) |
| `samples/` | Fixtures, also embedded as the in-app demo |

Ignored when walking folders: `.git`, `.DS_Store`, `node_modules`, `.hg`, `.svn`.

### Releases

```sh
scripts/release.sh patch        # or minor / major / 1.2.3
```

The script bumps the version in `Cargo.toml`, runs the tests, commits, and
creates the tag `vX.Y.Z`. Then it pushes `main` and the tag. Only version tags trigger
[`.github/workflows/release.yml`](.github/workflows/release.yml). That workflow:

1. builds the universal `MiniDiff.app` and attaches
   `MiniDiff-macos-universal.zip` and its `.sha256` to a GitHub Release;
2. deploys the landing page (`site/`) and `update.json` to the `gh-pages` branch.

The app reads `https://minidiff.signalwerk.ch/update.json` to find new versions.
Release assets must be downloadable without login, so the repository has to be
public for self-updates to work.

## License

[MIT](LICENSE)
