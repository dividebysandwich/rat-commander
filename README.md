# rat-commander (`rc`)

A self-contained, [Midnight Commander](https://midnight-commander.org/)-style
two-panel terminal file manager written in Rust with
[Ratatui](https://ratatui.rs/). It aims to need **no external tools** for its
core features: the viewer, editor, archive handling, and remote (FTP/SFTP/SCP)
clients are all built in.

The installed executable is named **`rc`** for quick typing.

---

## Features

**Panels & navigation**
- Two panels with **full** and **brief** listing formats
- **Vertical or horizontal** split (Ctrl-T)
- Configurable sort: Unsorted, Name, Extension, Size, Modify/Access/Change time,
  Inode — plus reverse, case-sensitive and executables-first toggles
- Multi-file selection (tag) and **select/unselect by wildcard or regex**
- Command line at the bottom; drop to a full-screen shell with **Ctrl-O**
- **Find File** with a live progress dialog (abortable; partial results kept);
  results are *panelized* into the active panel

**Built-in viewer (F3)**
- Text and **hex** modes, line wrap toggle, and search

**Built-in editor (F4)** — `mcedit`-style
- Block mark/copy/move/delete, search, **search & replace** (literal or regex),
  undo/redo, and a status bar showing the byte under the cursor, line/column and
  totals

**File operations**
- Copy / move / delete with a progress window showing a per-file gauge and a
  **transfer-speed chart** (speed vs. bytes), plus an **abort** button
- `chmod`, `chown`, symlink creation, and make-directory dialogs

**Archives — browsed like directories**
- Open `.zip`, `.tar`, `.tar.gz`, `.tar.bz2`, `.tar.xz`, `.7z` and `.rar`
  archives and walk into them like folders
- Copy files in/out, delete from an archive, and **compress** a selection into a
  new archive (RAR is read-only — no tool can create RAR archives)

**Remote filesystems** (each connection mounts into a panel)
- **SFTP** and **SCP** over SSH, and **FTP/FTPS**
- Copy/move/delete works transparently between local, remote and archive panels

**Look & feel**
- Many color themes (Dracula, Nord, Gruvbox, Solarized, Tokyo Night, Catppuccin,
  One Dark, …) plus **Monochrome**, **Amber CRT** and **Green CRT**
- On truecolor terminals: animated gradient bars/cursor, rounded dialog borders
  and drop shadows
- Optional **CPU histogram + memory** widget in the menu bar (wide screens)
- A configurable **F2 user menu** (Midnight Commander `menu` file format)
- Open a file with the system default app (`xdg-open`) by pressing Enter on it

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
- **macOS** — `rc-<ver>-<arch>.tar.gz`, or the `.pkg` installer
  (installs `rc` to `/usr/local/bin`). Intel and Apple Silicon builds are
  provided. The package is unsigned, so the first launch may require
  *System Settings → Privacy & Security → Open anyway*.

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

## Usage

Run `rc` in a terminal. The active panel has the highlighted border; **Tab**
switches panels. Press **Enter** on a directory to enter it, on an archive to
browse it, or on a file to open it with the system default program. Type a
command and press **Enter** to run it in the active panel's directory.

For a built-in cheat sheet, press **F1**.

### Keyboard shortcuts — panels

| Key | Action |
| --- | --- |
| `F1` | Help |
| `F2` | User menu (configurable) |
| `F3` | View file |
| `F4` | Edit file |
| `F5` | Copy |
| `F6` | Rename / move |
| `F7` | Make directory |
| `F8` | Delete |
| `F9` | Pulldown menu (Left/Right follows the active panel) |
| `F10` | Quit (confirmation) |
| `Ctrl-Q` | Quit immediately |
| `Tab` | Switch active panel |
| `↑ ↓ / PgUp PgDn / Home End` | Move the cursor |
| `Enter` | Open dir / enter archive / open file / run command line |
| `Insert` | Tag file and advance |
| `+` / `-` / `*` | Select / unselect group (wildcard) / invert selection |
| `← →` | Move within the command line |
| `Ctrl-O` | Toggle full-screen shell |
| `Ctrl-R` | Re-read the active panel |
| `Ctrl-S` / `Ctrl-E` | Cycle sort key / toggle reverse |
| `Ctrl-W` | Toggle brief / full listing |
| `Ctrl-T` | Toggle vertical / horizontal split |

### Keyboard shortcuts — viewer (F3)

| Key | Action |
| --- | --- |
| `F2` | Toggle line wrap |
| `F4` | Toggle hex / text mode |
| `F7` | Search |
| `n` | Repeat search |
| `↑ ↓ / PgUp PgDn / Home End` | Scroll |
| `Esc` / `F10` / `q` | Close |

### Keyboard shortcuts — editor (F4)

| Key | Action |
| --- | --- |
| `F2` | Save |
| `F3` | Start / end block mark |
| `F4` | Search & replace |
| `F5` / `F6` / `F8` | Copy / move / delete block |
| `F7` | Search |
| `Ctrl-V` | Paste |
| `Ctrl-Z` / `Ctrl-Y` | Undo / redo |
| `Esc` / `F10` | Quit (prompts if modified) |

### Dialogs

`Tab`/arrows move between fields, `Space` toggles checkboxes and cycles choices,
`Enter` confirms, `Esc` cancels. Progress dialogs can be aborted with `Esc`.

---

## Configuration

Files live in your platform config directory
(`~/.config/rat-commander/` on Linux):

- **`config.toml`** — written from the Settings dialog (F9 → Options →
  Settings). Holds the active theme, truecolor/animation/status-widget toggles,
  external editor/viewer commands, and the confirm-before-delete flag.
- **`menu`** — the **F2 user menu**, created with sensible defaults on first
  run. It uses the Midnight Commander menu format:

  ```
  # comment
  3      Compress the current subdirectory (tar.gz)
          Pwd=`basename "%d"`
          tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"
  ```

  A line starting in column 0 is a menu entry whose first character is the
  hotkey; the indented lines below it are the shell commands. Macros: `%f`/`%p`
  current file, `%d` current directory, `%t` tagged files, `%s` tagged-or-current,
  `%%` a literal percent. (mc condition lines `+ …` / `= …` are accepted and
  ignored; entries are always shown.)

### Themes

Pick a theme in the Settings dialog — it previews live as you cycle through it
(Enter keeps it, Esc reverts). On a 24-bit-color terminal (`COLORTERM=truecolor`)
the bars and cursor render as animated gradients; otherwise solid colors are
used. Truecolor, animations, and the system-status widget can each be toggled in
Settings.

### Remote connections

Open `F9 → Left`/`Right → SFTP/FTP/SCP connection…`, enter host / port / user /
password / remote path, and the panel mounts the server. SSH host keys are
checked against `~/.ssh/known_hosts` (trust-on-first-use; a changed key is
rejected). `Disconnect` returns the panel to the local filesystem.

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
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
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
`cross` for the Raspberry Pi targets).

---

## License

MIT. See `Cargo.toml`.
