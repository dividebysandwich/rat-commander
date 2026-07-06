# Rat Commander (`rc`)

A self-contained terminal file manager with modern features and built-in tools,
while staying true to the heritage of classics such as Norton Commander and
[Midnight Commander](https://midnight-commander.org/). Written in Rust with
[Ratatui](https://ratatui.rs/). It aims to need **no external tools** for its
core features: the viewer/editor with syntax highlighting, archive handling,
remote (FTP/SFTP/SCP) clients, disk explorer and process explorer are all built
in.

The installed executable is named **`rc`** for quick typing.

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/467309a5-e43b-4096-b58b-95199a980eda" />

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/5b13c3c9-e770-4ce6-ac2b-560e7b5c3bad" />

---

## What it can do

- **Two panels** with **full**, **brief**, **details** and **tree** view formats,
  vertical or horizontal split, configurable sort, multi-file selection, type
  markers and file-type colors. Full **mouse** support.
- **File operations** — copy / move / delete with a progress window and
  transfer-speed chart, rich overwrite handling, chmod / chown / symlink (with
  recursion), and make-directory.
- **Built-in viewer (F3)** — text and hex modes, search, goto, line wrap, syntax
  highlighting, a **rendered Markdown** mode for `.md` files, and hex-color
  swatches. Pages huge files straight from disk.
- **Built-in editor (F4)** — `mcedit`-style block copy/move/delete, clipboard,
  search & replace (literal or regex), undo/redo, syntax highlighting, and an
  in-place **hex editor** for arbitrarily large files. Launch straight into it
  with **`rc /edit <file>`** (or the installed **`rcedit <file>`** shortcut), or
  **`rc /edit`** with no file for a blank buffer that prompts for a name on the
  first save; closing the editor then exits.
- **Multi rename** — batch-rename selected files with a masked, live two-column
  preview, counter, case transform and search-and-replace.
- **Find file**, **Compare directories**, **Find duplicates**, and a
  side-by-side **Compare files** diff with in-place merging.
- **Checksum** — compute a CRC32/MD5/SHA-1/SHA-256/SHA-512 digest of a file with
  a progress bar, and optionally verify it against a pasted reference checksum.
- **Archives** — browse `.zip`, `.tar(.gz/.bz2/.xz)`, `.7z` and `.rar` like
  directories; copy in/out, delete, and compress a selection.
- **Remote filesystems** — SFTP, SCP and FTP/FTPS, each mounted into a panel;
  copy/move/delete works transparently across local, remote and archive panels.
- **Disk explorer** (treemap of disk usage), **process explorer** (btop-style
  system monitor), and a **disk manager** (Linux) to mount/unmount/format/sync
  drives and **flash or image** raw disk images.
- **Network connections** (Linux) — listening ports with their programs and all
  active connections with their type, service, live per-connection traffic rate
  (with a sparkline) and a details view; filter, sort, kill the owning process,
  and an optional root password for full visibility. A **per-service overview
  diagram** (Tab) groups connections into colour-coded cards showing each peer IP
  and its direction, with clickable/navigable addresses and reverse-DNS lookups.
- **Look & feel** — many color themes (fully customizable via `themes.toml`),
  truecolor animated gradients, an optional CPU/memory status widget, and a
  configurable **F2 user menu**.
- **Terminal graphics** — on terminals with a **Kitty**, **Sixel** or **iTerm2**
  graphics protocol, the progress bars, process-explorer graphs, transfer speed
  graph and the disk-explorer **treemap** (a nested "pillow" map of each folder's
  biggest files) are drawn as true-pixel gradient images, falling back
  automatically to block-character rendering elsewhere (can be forced off in settings).
- **Localization** — Configurable UI language with 18
  languages built in (English, German, French, Spanish, Portuguese, Dutch,
  Czech, Slovak, Hungarian, Serbian, Ukrainian, Russian, Japanese, Chinese
  traditional & simplified, Hindi, Persian, Arabic); translations live in
  editable `lang/*.toml` files and new languages can be dropped in. Right-to-left
  scripts (Arabic, Persian) are shaped and bidi-reordered for display on
  terminals without native bidi support (a **Reshape RTL text** setting turns
  this off when the terminal handles bidi itself).
- **Windows support** — Full support for windows drives using the familiar
  Alt-F1/Alt-F2 Norton Commander shortcuts. All features except Drive Manager and
  Network Connections are available.

For a full, feature-by-feature walkthrough see the **[user manual](doc/MANUAL.md)** —
also available in-program by pressing **F1**.

---

## Keyboard shortcuts

On terminals where the function keys are awkward to reach, every `Fn` shortcut
also has a Midnight-Commander-style alias: press **Esc** then a digit — `Esc 1`
… `Esc 9` for `F1`…`F9`, and `Esc 0` for `F10` (or a quick **Alt**+digit).

### Panels

| Key | Action |
| --- | --- |
| `F1` | Help (the user manual) |
| `F2` | User menu (configurable) |
| `F3` | View file |
| `F4` | Edit file |
| `F5` | Copy |
| `F6` | Rename / move |
| `Shift-F6` / `Ctrl-F6` | Multi rename (selected files) |
| `F7` | Make directory |
| `F8` | Delete |
| `F9` | Pulldown menu (Left/Right follows the active panel) |
| `F10` | Quit (confirmation) |
| `Ctrl-Q` | Quit immediately |
| `Tab` | Switch active panel |
| `↑ ↓ / PgUp PgDn / Home End` | Move the cursor |
| `Enter` | Open dir / enter archive / open file / run command line |
| `cd <dir>` + `Enter` | Change the active panel's directory |
| `Insert` / `Ctrl-T` | Tag file and advance |
| `+` / `-` / `*` | Select / unselect group (wildcard) / invert selection |
| `Ctrl-O` | Toggle the persistent subshell |
| `Ctrl-R` | Re-read the active panel |
| `Ctrl-S` / `Ctrl-E` | Cycle sort key / toggle reverse |
| `Ctrl-W` | Cycle view format (full / brief / details / tree) |
| `Ctrl-X` | Toggle vertical / horizontal split |
| `Ctrl-U` | Swap the two panels |
| `Alt-F1` / `Alt-F2` | Drive / connection picker (left / right panel) |

### Viewer (F3)

| Key | Action |
| --- | --- |
| `F2` | Toggle line wrap |
| `F4` | Toggle hex / text mode |
| `F5` | Goto (line / percent / byte offset) |
| `F7` | Search (`n` repeats) |
| `F8` | (Markdown) toggle Raw / Render |
| `Esc` / `F10` / `q` | Close |

### Editor (F4)

| Key | Action |
| --- | --- |
| `F1` | Editor shortcut help |
| `F2` | Save |
| `Shift-F2` / `Ctrl-F2` | Save as… (browse + name) |
| `F3` | Start / end block mark |
| `F4` | Search & replace |
| `F5` / `F6` / `F8` | Copy / move / delete block |
| `F7` | Search |
| `Ctrl-C` / `Ctrl-V` | Copy block to clipboard / paste |
| `Ctrl-Z` / `Ctrl-Y` | Undo / redo |
| `F9` | Toggle in-place hex editor |
| `Shift-F9` / `Ctrl-F9` | Toggle word wrap |
| `Esc` / `F10` | Quit (prompts if modified) |

### Dialogs

`Tab`/arrows move between fields **and onto the OK/Cancel buttons**, `Space`
toggles checkboxes and cycles choices, `Enter` confirms, `Esc` cancels (and
aborts progress dialogs). The OK/Cancel and Yes/No buttons are also clickable.

See the **[user manual](doc/MANUAL.md)** for the process-explorer,
disk-explorer and hex-editor key tables, and for what every feature does.

---

## Installation

### Pre-built packages

Grab a release from the **Releases** page:

- **Linux** — `rc-<ver>-<arch>.tar.gz` archive, or a `.deb`
  (`amd64`, `arm64` for Raspberry Pi 64-bit, `armhf` for 32-bit):
  ```sh
  sudo dpkg -i rat-commander_<ver>_arm64.deb
  ```
- **Windows** — `rc-<ver>-x86_64-pc-windows-msvc.zip`, or the `.msi` installer
  (adds `rc` to your PATH).
- **macOS** — `rc-<ver>-<arch>.tar.gz`, or the `.pkg` installer (installs `rc`
  to `/usr/local/bin`). Intel and Apple Silicon builds are provided. The package
  is unsigned, so the first launch may require *System Settings → Privacy &
  Security → Open anyway*.

### From source

Requires a recent stable Rust toolchain (edition 2024, **Rust ≥ 1.85**):

```sh
git clone https://github.com/dividebysandwich/rat-commander
cd rat-commander
cargo install --path .      # installs `rc` into ~/.cargo/bin
# or just run it:
cargo run --release
```

---

## Building & packaging

```sh
cargo build --release            # target/release/rc
cargo test                       # run the test suite
cargo clippy --all-targets       # lints
```

Release binaries are stripped and optimized via the `[profile.release]` settings
in `Cargo.toml`.

### Cross-compiling and packages

The `.github/workflows/release.yml` workflow builds every artifact. To reproduce
a build locally:

```sh
# Debian package (native arch)
cargo install cargo-deb
cargo build --release --target x86_64-unknown-linux-gnu
cargo deb --no-build --target x86_64-unknown-linux-gnu

# Raspberry Pi (cross-compiled) – needs Docker + `cross`
# (--no-default-features drops RAR, whose C++ lib won't cross-compile here)
cargo install cross
cross build --release --no-default-features --target aarch64-unknown-linux-gnu
cargo deb --no-build --no-strip --target aarch64-unknown-linux-gnu

# Windows MSI – on Windows with the WiX toolset
dotnet tool install --global wix --version 4.0.5
wix build packaging/windows/rc.wxs -d Version=0.1.0 \
    -d BinDir=target/x86_64-pc-windows-msvc/release -o rc.msi

# macOS .pkg – on macOS
pkgbuild --identifier com.rat-commander.rc --version 0.1.0 \
    --install-location /usr/local/bin --root <dir-containing-rc> rc.pkg
```

Some dependencies (`unrar`, `bzip2`, `xz2`, archive backends) compile bundled
C/C++ sources, so a C/C++ toolchain is required (provided automatically by
`cross` for the Raspberry Pi targets). RAR support is an optional build feature
(`rar`, on by default), omitted from the Raspberry Pi (arm) packages because the
C++ `unrar` library doesn't build with those cross toolchains.

---

## Configuration

Configuration lives in your platform config directory
(`~/.config/rat-commander/` on Linux): **`config.toml`** (written from the
Settings dialog), **`themes.toml`** (editable color themes), **`lang/`**
(one editable TOML per UI language), and **`menu`** (the F2 user menu, in
Midnight Commander format). See the
**[user manual](doc/MANUAL.md#configuration)** for details.

---

## License

GNU General Public License, version 2 (GPL-2.0-only). See the `LICENSE` file.
