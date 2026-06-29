# rat-commander (`rc`)

A self-contained, [Midnight Commander](https://midnight-commander.org/)-style
two-panel terminal file manager written in Rust with
[Ratatui](https://ratatui.rs/). It aims to need **no external tools** for its
core features: the viewer, editor, archive handling, and remote (FTP/SFTP/SCP)
clients are all built in.

The installed executable is named **`rc`** for quick typing.

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/467309a5-e43b-4096-b58b-95199a980eda" />

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/5b13c3c9-e770-4ce6-ac2b-560e7b5c3bad" />



---

## Features

**Panels & navigation**
- Two panels with **full** and **brief** listing formats
- **Vertical or horizontal** split (Ctrl-T)
- Configurable sort: Unsorted, Name, Extension, Size, Modify/Access/Change time,
  Inode — plus reverse, case-sensitive and executables-first toggles
- Multi-file selection (tag) and **select/unselect by wildcard or regex**
- `ls -F`-style **type markers** before each name (so types read by symbol, not
  just color): `/` directory, `*` executable, `@` symlink, `!` broken symlink,
  and a leading space for plain files (keeps names aligned)
- **File-type colors** by extension (theme-derived): archives purple, documents
  dark yellow, images cyan, audio/video green
- Each panel shows the volume's **free / total disk space** on its bottom border,
  and the selected file on a separated mini-status line
- Command line at the bottom; type **`cd <dir>`** to change the active panel
  (supports `~`, `..`, absolute and relative paths); drop to a full-screen shell
  with **Ctrl-O**
- **Mouse support**: left-click a file to move the cursor, right-click to
  invert its mark, drag to carry the cursor (right-drag inverts the mark of every
  file it passes over); click the menu bar to open menus and pick items, and click
  **OK/Cancel** (or **Yes/No**) buttons in dialogs
- **Find File** with a live progress dialog (abortable; partial results kept);
  results are *panelized* into the active panel (with a `..` entry to return to
  the normal listing). On **remote** panels the search matches **file names**
  only (content search is local-only), so you can locate files on a server too
- **Compare directories** (Command menu): mark the files that differ between the
  two panels — **Quick** (present in one panel only), **Size only** (also marks
  the larger of two differently-sized files), or **Content** (marks both files
  whenever their bytes differ)
- **Compare files** (Command menu): a side-by-side diff of the files under the
  cursor in each panel. Changed/added blocks are highlighted and connected by
  gutter guides; **↑/↓** moves through the document and selects the active change,
  **Ctrl-↑/↓** jumps to the previous/next change, **Ctrl-←** applies the active
  change from the right file to the left (or deletes a left-only block), **Ctrl-→**
  applies it the other way. Edits happen in memory; **F2** asks to save and writes
  changed files back to disk, **Esc** closes (prompting save/discard/cancel when
  there are unsaved changes)

**Built-in viewer (F3)**
- Text and **hex** modes, line wrap toggle, and search
- **Syntax highlighting** (via [`syntect`](https://github.com/trishume/syntect),
  the engine behind `bat`) for recognized source files, with a bundled theme
  chosen to suit the active light/dark UI. Highlighting is incremental and
  size-capped so it stays responsive. Beyond syntect's ~75 default languages,
  extra formats — **TOML**, **INI/config**, **Dockerfile**, **HCL/Terraform**,
  **GraphQL**, **Protobuf**, **CMake**, **TypeScript/TSX**, **Kotlin**, **Swift**,
  **SCSS/Sass**, **Elixir**, **Zig** and **Nix** — are bundled as embedded
  `.sublime-syntax` files, and more can be dropped in to extend it
- **Paged from disk** — local files are read on demand (only a per-line offset
  index is kept), so arbitrarily large files open instantly without loading into
  memory; search streams the file in windows too
- Opening a **large remote file** (View or Edit) streams it with a **progress
  dialog you can abort** (Esc); the viewer then pages from the downloaded copy

**Built-in editor (F4)** — `mcedit`-style
- Block mark/copy/move/delete, search, **search & replace** (literal or regex),
  undo/redo, and a status bar showing the byte under the cursor, line/column and
  totals
- **Syntax highlighting** (same `syntect` engine) that updates incrementally as
  you type — only the edited line onward is re-highlighted — and coexists with
  block selection highlighting
- **Hex editor mode** (F9): an offset / hex / ASCII view that edits **in place** —
  only the visible window is read and only changed bytes are written back, so
  arbitrarily large files can be hex-edited (files too big to load as text open
  straight into hex mode). `Tab` switches between the hex and ASCII columns;
  editing is overwrite-only (length-preserving), with streaming **search and
  replace** over hex-byte or text patterns

**File operations**
- Copy / move / delete with a progress window showing a per-file gauge and a
  **transfer-speed chart** (speed vs. bytes)
- **Overwrite confirmation** when a destination exists: overwrite **Yes/No**, or
  **Append**, or apply a rule to all remaining files (**All**, **Older**,
  **None**, **Smaller**, **Size differs**) with an optional "don't overwrite with
  a zero-length file" guard
- `chmod`, `chown`, symlink creation, and make-directory dialogs

**Archives — browsed like directories**
- Open `.zip`, `.tar`, `.tar.gz`, `.tar.bz2`, `.tar.xz`, `.7z` and `.rar`
  archives and walk into them like folders
- Copy files in/out, delete from an archive, and **compress** a selection into a
  new archive (RAR is read-only — no tool can create RAR archives)
- RAR support is an optional build feature (`rar`, on by default); it is omitted
  from the Raspberry Pi (arm) packages because the C++ `unrar` library doesn't
  build with those cross toolchains



**Remote filesystems** (each connection mounts into a panel)
- **SFTP** and **SCP** over SSH, and **FTP/FTPS**
- Copy/move/delete works transparently between local, remote and archive panels
- When the destination panel is remote, the copy/move dialog prefills a
  `scheme://path` target (e.g. `scp-0:///home/user`). **Delete the `scheme://`
  prefix** to redirect the copy to a **local** path instead (absolute, or
  relative to the source directory) — handy for pulling a file down to disk while
  a remote connection stays open

<img width="576" height="217" alt="image" src="https://github.com/user-attachments/assets/792884cc-f9b9-495c-9cbc-9171d802a290" />



**Look & feel**
- Many color themes (Dracula, Nord, Gruvbox, Solarized, Tokyo Night, Catppuccin,
  One Dark, …) plus **MidnightCommander Classic** (bright classic-MC look),
  **Monochrome**, **Amber CRT** and **Green CRT**, and the playful
  **Rainbow**, **Candy**, **Neon**, **Forest**, **Freedom** and **Movienight**
- On truecolor terminals: animated gradient bars/cursor, rounded dialog borders
  and drop shadows
- Optional **CPU histogram + memory** widget in the menu bar (wide screens)
- A configurable **F2 user menu** (Midnight Commander `menu` file format)
- Open a file with the system default app (`xdg-open`) by pressing Enter on it

<img width="1006" height="657" alt="image" src="https://github.com/user-attachments/assets/579e86aa-f3d8-4f19-9f45-be46e0c36a42" />



**Process explorer** (Command menu → *Process explorer…*)
- A full-screen list of processes with **CPU%, memory and thread count**, sortable
  by **name, CPU, memory, threads or PID** (the sort hotkey is shown in the column
  header, e.g. `[C]PU%`), and **kill** (SIGTERM, or SIGKILL with `K`)
- A btop-style layout: an animated **CPU-load line graph** and **per-core meters**
  (the CPU model name is shown on the core panel's border) on top, with **memory,
  disk-I/O and network sparklines** stacked down the left (the network panel shows
  download above, upload below) and the process table on the right — all using
  truecolor load colors when available (Linux `/proc`)
- The **update interval** is adjustable with **`+`/`-`** (100 ms steps, min 100 ms)
  and shown on the top-right border; when a **battery** is present its charge and a
  mini bar graph are shown centered on the top border

  <img width="1007" height="514" alt="image" src="https://github.com/user-attachments/assets/3dd795cf-bf59-4f8b-94de-b9d49ad5f989" />



**Disk explorer** (Command menu → *Disk explorer…*)
- A full-screen **treemap** of the current directory's subdirectories: each box's
  area is proportional to the subtree's **on-disk size**, labeled with the name
  and a human-readable size (e.g. `2.1 GB`). Symlinks are never followed or counted
- Boxes that are **large enough** also list their **biggest files** inside, each
  with its path relative to that box and its size — so you can spot the space
  hogs without diving in
- The **top bar** always shows the selected box's name, size and share of the
  total — so the selection is legible even when its box is too small for a label
- **Arrow keys** move the selection between boxes, **Enter** dives into the
  selected subdirectory, **Backspace** goes up, **`g`** (or **Ctrl-Enter** where
  the terminal reports it) exits and points the active file panel at the selected
  directory, **Esc** closes

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/3673b354-1fb7-4445-8e5e-ebdb356c0b96" />


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
command and press **Enter** to run it in the active panel's directory — except
**`cd <dir>`**, which changes the active panel itself (so the directory change
sticks, unlike running `cd` in a subshell).

The **mouse** works too: **left-click** a file to move the cursor to it (and
activate that panel), **right-click** to invert its mark, and **drag** to carry
the cursor along — dragging with the right button **inverts the selection** of
every file it sweeps across (each file flips once). Click a menu-bar title to
open it and click an entry to run it, and click the **OK/Cancel** / **Yes/No**
buttons in dialogs.

For a built-in cheat sheet, press **F1**.

On terminals where the function keys are awkward to reach, every `Fn` shortcut
also has a Midnight-Commander-style alias: press **Esc** then a digit — `Esc 1`
… `Esc 9` for `F1`…`F9`, and `Esc 0` for `F10`. This works in the panels, the
viewer, and the editor. (A quick `Alt`+digit does the same thing.)

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
| `Esc` then `1`…`9` / `0` | Function-key alias for `F1`…`F9` / `F10` (Midnight Commander style) |
| `Ctrl-Q` | Quit immediately |
| `Tab` | Switch active panel |
| `↑ ↓ / PgUp PgDn / Home End` | Move the cursor |
| `Enter` | Open dir / enter archive / open file / run command line |
| `cd <dir>` + `Enter` | Change the active panel's directory |
| `Insert` | Tag file and advance |
| `+` / `-` / `*` | Select / unselect group (wildcard) / invert selection |
| `← →` | Move within the command line |
| `Ctrl-O` | Toggle the persistent subshell (press `Ctrl-O` again to return) |
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
| `F9` | Toggle hex editor mode (in place) |
| `Esc` / `F10` | Quit (prompts if modified) |

### Keyboard shortcuts — hex editor (F9 in the editor)

| Key | Action |
| --- | --- |
| `0`–`9`, `a`–`f` | Overwrite the current byte's nibble (hex column) |
| typed character | Overwrite the current byte (ASCII column) |
| `Tab` | Switch between the hex and ASCII columns |
| `← ↑ ↓ → / PgUp PgDn` | Move | 
| `Home` / `End` | Start / end of row |
| `Ctrl-Home` / `Ctrl-End` | Start / end of file |
| `F7` | Search (hex bytes like `48 65` or text) |
| `F4` | Replace all (same-length, overwrite-only) |
| `F2` | Save changed bytes in place |
| `F9` | Back to text mode |
| `Esc` / `F10` | Quit (prompts if modified) |

### Keyboard shortcuts — process explorer

| Key | Action |
| --- | --- |
| `↑ ↓ / PgUp PgDn / Home End` | Move the selection |
| `c` / `m` / `n` / `p` | Sort by CPU / memory / name / PID (press again to reverse) |
| `r` | Reverse sort order |
| `k` / `F8` / `F9` / `Del` | Kill the selected process (SIGTERM, with confirmation) |
| `K` | Force-kill (SIGKILL, with confirmation) |
| `Esc` / `F10` / `q` | Close |

### Keyboard shortcuts — disk explorer

| Key | Action |
| --- | --- |
| `← ↑ ↓ →` | Move the selection between boxes |
| `Enter` | Dive into the selected subdirectory |
| `Backspace` | Go up to the parent directory |
| `g` / `Ctrl-Enter` | Exit and open the selected directory in the active panel |
| `Esc` / `F10` / `q` | Close |

### Dialogs

`Tab`/arrows move between fields, `Space` toggles checkboxes and cycles choices,
`Enter` confirms, `Esc` cancels. Progress dialogs can be aborted with `Esc`. You
can also **click** the OK/Cancel (or Yes/No) buttons with the mouse.

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

Previously used servers are remembered in `config.toml` (passwords are **not**
stored). In the connection dialog, pick one from the **history dropdown** — open
it by clicking the **▼** chevron on the Host field or pressing **↓** while the
Host field is focused; selecting an entry fills in the host/port/user/path.

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
`cross` for the Raspberry Pi targets).

---

## License

GNU General Public License, version 2 (GPL-2.0-only). See the `LICENSE` file.
