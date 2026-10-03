# Rat Commander (`rc`)

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/W7J0271P9Q)

A self-contained terminal file manager with modern features and built-in tools,
while staying true to the heritage of classics such as Norton Commander and
[Midnight Commander](https://midnight-commander.org/). Written in Rust with
[Ratatui](https://ratatui.rs/). It aims to need **no external tools** for its
core features: the viewer/editor with syntax highlighting, archive handling,
remote (FTP/SFTP/SCP) clients, disk explorer and process explorer are all built
in.

The installed executable is named **`rc`** for quick typing.

<img alt="image" src="https://img.playspoon.com/25t8uc.gif" />

<img width="1004" height="659" alt="image" src="https://github.com/user-attachments/assets/5b13c3c9-e770-4ce6-ac2b-560e7b5c3bad" />

<img width="1005" height="647" alt="image_2026-09-11_15-42-47" src="https://github.com/user-attachments/assets/7c465859-fb36-44e1-94ee-fa3000e9e8c9" />


---

## Features

- **Two panels** with **full**, **brief**, **details**, **tree**, **3D** and **thumbnail grid** view formats,
  vertical or horizontal split, configurable sort, multi-file selection, type
  markers and file-type colors. Full **mouse** support. The **details** view adds
  a background-loaded **preview** of the other panel's item — a syntax-highlighted
  text head, a centered image thumbnail (true-pixel where graphics are available,
  else half-block art) with an EXIF summary, an archive's file list, or a
  directory tree — and, inside a git work tree, a **git activity calendar**: a
  year of commits to that file or directory, a GitHub-style grid of days.
- **File operations** — copy / move / delete with a progress window and
  transfer-speed chart, rich overwrite handling, chmod / chown / symlink (with
  recursion), and make-directory.
- **Built-in viewer (F3)** — text and hex modes, goto, line wrap, syntax
  highlighting, a **rendered Markdown** mode for `.md` files, a **spreadsheet
  table** for `.csv` / `.tsv` files (delimiter detected, quoted line breaks
  understood, header row pinned, paged from disk however large), and hex-color
  swatches. Pages huge files straight from disk. **Follow mode** (`f`) tails a
  growing file like `tail -f` — pausing while you scroll back, and surviving
  truncation and log rotation — with log lines coloured by severity. **Git
  blame** (`b`) shows who last changed each line, shaded by age; `Enter` on a
  line opens the tree as it was at that commit. Opens **images** (SVG
  included) fullscreen —
  true-pixel where the terminal supports graphics, half-block art otherwise
  (F8 toggles to the raw bytes). Opens **documents** on what they read as:
  Word, PowerPoint, OpenDocument, EPUB and PDF as text with headings, lists and
  tables (F6 jumps between sections, slides or pages), and Excel and
  OpenDocument **spreadsheets** in the table view, sheet by sheet — no office
  suite needed.
- **3D model viewer (F3)** — press F3 on an `.stl` or `.obj` and the mesh opens
  as a solid you can **orbit** with the arrow keys (or by dragging), zoom with
  `+`/`-` and re-frame with `Home`. Binary and ASCII STL and Wavefront OBJ are
  read natively — no converter, no external viewer — and the same three tiers
  apply as everywhere else: true pixels on a graphics terminal, half-blocks on a
  truecolor one, an ASCII ramp otherwise. F8 switches to the raw bytes, and a
  file that will not parse simply opens as hex. Model files also get their own
  colour in the listings and their own solid in the 3D landscape.
- **Audio view (F3, and the Details preview)** — WAV, FLAC, MP3, Ogg Vorbis,
  AAC/ALAC, AIFF and CAF files open as a **spectrogram** or a **waveform** of the
  whole file (F2 switches; Settings → Panels picks the default), drawn while it
  decodes in the background. Play, pause, stop, seek and set the volume from the
  transport row, and **click or drag anywhere on the picture to jump there**. The
  Details view shows the same picture for the file under the other panel's
  cursor, playable without leaving the listing. Decoding is pure Rust; playback
  uses the system audio (the `audio` build feature).
- **Byte map (F4, third mode)** — the whole file as one picture, each cell a span
  coloured by how *dense* its bytes are (Shannon entropy) or by what they mostly
  **are** (zero padding / ASCII / high bytes / mixed). Compressed and encrypted
  regions glow, padding goes flat, and the seams between a container's parts show
  up as visible bands — a structural overview of an ISO, a firmware blob or a raw
  disk image that no hex dump gives you. Move the cursor and the header reports
  the byte offset and entropy under it; press **Enter** and the hex view opens
  *there*. The file is **sampled, not read whole**, so it costs the same bounded
  work on a 4 MB file as on a 40 GB one.
- **Binary view (F3 on an executable)** — ELF, PE and Mach-O programs and
  libraries, universal binaries included, open as what they are made of: format,
  architecture, entry point, linking and **hardening** (PIE, NX, RELRO, canary,
  ASLR, DEP, CFG), then scrollable lists of **sections**, **libraries**,
  **imports**, **exports**, **functions** and **strings**. All three formats are
  read natively on every platform. Functions are recovered from the unwind
  tables even in a stripped file, Rust and C++ names are demangled, and the
  strings list drops the noise a plain `strings` buries the text under. `Enter`
  opens the hex view at a row's bytes, and *Find all* narrows every list to one
  term. Analysis runs in the background; a file that does not parse opens as
  text.
- **Certificates and keys (F3 on a `.pem`, `.crt`, `.key`, `.csr` …)** — X.509
  certificates, certificate requests and private and public keys, PEM (whole
  bundles) or DER, laid out field by field: subject, issuer, validity with the
  **days until expiry** (red once expired, amber within 30 days), SANs, key type
  and size, key usage, fingerprints and SPKI pins. The **Chain** tab checks
  which certificate issued which, **verifies each signature** against its
  issuer and says whether they are in the order a server must send them; a
  private key is matched to its certificate without decrypting anything, and
  its key material is never shown. **SSH** files too: OpenSSH private keys
  (fingerprint and encryption, readable without the passphrase), public keys,
  OpenSSH certificates (principals, validity, signing CA and whether its
  signature holds), and every line of `authorized_keys` and `known_hosts`.
  F8 switches to the raw text. In the editor, **Decode JWT at cursor** shows a
  JSON Web Token's header, claims and expiry.
- **Built-in editor (F4)** — `mcedit`-style block copy/move/delete, clipboard,
  search & replace, undo/redo, syntax highlighting, an **audio tag editor**
  (F4 on an MP3, Ogg, FLAC, M4A… opens a page of its tags — the well-known ones
  and whatever else the file carries, all editable, with `F5` to add any key the
  format supports; `F3` for the bytes behind them), a **spreadsheet grid** for
  `.csv` / `.tsv` files (edit cells in a cell bar, insert and delete rows and
  columns, every change one undo step and saved as plain text), a **live syntax
  check** of JSON, TOML, YAML and XML files that marks every error as you type —
  in a gutter, underlined, explained on the status line, `Alt-E` to the next
  one — and **JSON Schema validation** of JSON, YAML and TOML files marked the
  same way (schemas bundled for docker-compose, GitHub Actions workflows and
  actions, Dependabot, GitLab CI, `Cargo.toml`, `package.json` and
  `tsconfig.json`, or named in the file, or mapped in your config) — JSON
  **pretty-printing** (`Alt-F`), minifying and key sorting that keep the key
  order, the numbers as written and a JSONC file's comments — a
  **shapefiles** (`.shp` opens as the GeoJSON it becomes — attributes and all —
  drawn and edited on the same map, and saved back into the `.shp`/`.shx`/`.dbf`
  set; UTM and Web Mercator files are reprojected on the way in and out), a
  **GeoJSON map** (`Alt-M`) that draws the GeoJSON in a file, or nested anywhere
  in a larger JSON document, over a built-in vector map of the world (land,
  lakes, borders, rivers and cities from Natural Earth) with pan, zoom and
  click-to-pick, in pixels or braille — and **edits** it: draw points, lines and
  polygons, drag, add and remove positions, name and remove features, start a
  FeatureCollection from an empty file, every change written into the text in
  the file's own layout as one undo step — and an
  in-place **hex editor** for arbitrarily large files, with **010 Editor binary
  templates**: the file's structures and fields as a tree beside the bytes —
  names, values, offsets, sizes, types, comments — picked automatically from 307
  bundled templates (or your own), coloured onto the bytes, and editable in
  place — plus a **data inspector** reading the bytes at the cursor as every
  integer width, floats, LEB128, UTF-8/UTF-16, time_t/FILETIME/DOS dates and
  GUIDs in either byte order, each one editable.
- **Multi rename** — batch-rename selected files with a masked, live two-column
  preview, counter, case transform and search-and-replace. Masks can pull from
  the files themselves: a photo's **EXIF** (`[EXIF:YMD]` date taken, and its
  year/month/day/hour parts separately, camera, lens, exposure, ISO, focal
  length, dimensions, GPS) and an audio file's **tags** (`[TAG:Artist]`,
  `[TAG:Title]`, `[TAG:Album]`, `[TAG:Track]`…), so photos rename to the day
  they were shot and music to what it is. **F1** in the dialog lists every
  placeholder.
- **Search** — one dialog for the editor (F7/F4) *and* the viewer (F7): literal,
  **regex**, **hex** or **wildcard**, with case / whole-word / backwards options.
  **Find all** highlights every line holding the term and keeps it highlighted
  while you work; repeating a search steps to the next occurrence and wraps.
- **Git-aware panels** — inside a git work tree each file is tagged with its VCS
  state (`>` modified, `+` staged, `?` untracked, `!` conflict) in colour, the
  current **branch + ahead/behind** shows on the panel border, and one-key actions
  **stage/unstage** (`Ctrl-G`) or open a side-by-side **diff against HEAD**
  (`Alt-D`).
- **Stage a single hunk** — in the `Alt-D` diff, `s` stages the hunk under the
  cursor, `x` discards it and `u` unstages the whole file, so a change worth
  committing on its own can be split out without leaving the file manager. The
  patch comes from `git diff` byte for byte, so CRLF files and files with no
  trailing newline stage as cleanly as any other.
- **Git functions (`Alt-G`, or File → Git)** — **status**, **log**, **add**, 
  **unstage**, **rm**, **restore**, **commit**, **stash** (save, list, apply,
  pop, drop), **fetch**, **pull**, **push** 
  (with `--force-with-lease` or `--force`), **sync** (pull + push), 
  **checkout**, **reset**, **init** and **clone**. 
- **Browse git history** — the Git menu's *Browse a revision* mounts the
  repository's **past into a panel**: every commit is a directory, named so that
  sorting by name sorts by time, and stepping into one shows the tree exactly as
  it was. Because history is just another filesystem to `rc`, everything else
  keeps working there — **F3** views a file as it was, **F5** copies it back out
  into the present, *Compare files* diffs it against your working copy, and
  *Find file* searches it. No checkout, no stash, no second clone. It is
  read-only, and says so before a transfer starts rather than part-way through.
- **The time machine** — press **`t`** in a 3D panel and the landscape starts
  moving through the repository's history. Scrub with `[`/`]` (or `{`/`}` for ten
  at a time) and the shape re-forms as you go: directories swell as they fill up,
  whole subtrees grow out of their parent at the commit that created them and
  fade away again as you scrub back past it. A track under the scene says where
  you are and the title names the commit. Sizes come from git, so a commit's tree
  is walked once and revisited for free — dragging across two hundred commits
  costs a handful of reads, not one apiece.
- **Thumbnail grid** — a view format showing a directory as pictures: photos and
  other images as thumbnails (true pixels on a graphics terminal, half-block art
  elsewhere) and STL/OBJ models rendered in 3D, loaded in the background a page
  at a time, with three cell sizes.
- **Activity log** — a panel format listing, live, everything created, written,
  removed and renamed anywhere under the other panel's directory: *what is this
  installer writing?* Bursts fold into one row with a count, a sparkline shows
  the event rate, `Enter` jumps the other panel to the file, `Insert` pauses.
- **A landscape that reacts** — while a 3D panel is up, the directories in it
  **light up as things are written into them**, fading out again over about a
  second. Point one panel at a build directory, put the other in 3D, and you can
  watch a compile happen. Activity anywhere below a box surfaces *on* that box,
  so work deep in a tree is still visible. It needs a recursive filesystem watch,
  so it can be turned off in Settings (*3D view: show filesystem activity*) on a
  very large or network-mounted tree — and it falls back on its own if the
  system refuses the watch.
- **Command palette (Ctrl-P)** — one fuzzy-search box over every menu action,
  every setting (switch theme/language/graphics or flip a toggle in place), your
  directory **bookmarks**, the open remote connections, and your saved remote
  servers (reconnect); type a few letters and press Enter.
- **Directory tabs** — each panel keeps as many open directories as you like:
  **Ctrl-N** opens a tab, **Alt-K** closes one, **Ctrl-PageDown**/**Ctrl-PageUp**
  cycle them and **Alt-J** lists them to pick from (**Ctrl-Tab** works too on
  terminals that don't reserve it for their own tabs). A tab remembers its
  directory, view format, sort, filter, marks and cursor, and local tabs come
  back on the next run. The strip only appears once a panel has more than one.
- **Panelize a command's output** — run `rg -l TODO`, `git ls-files -m`,
  `find . -mtime -1` or `dpkg -L nginx` and the files it names become the panel
  listing, ready for F3, tagging, F5 and F8. The command has to print paths one
  per line (so plain `ls`, not `ls -la`); `find -print0` and `git ls-files -z`
  are understood, so names with newlines in them survive.
- **Find file**, **Compare directories**, **Find duplicates**, and a
  side-by-side **Compare files** diff with in-place merging — binary files are
  compared byte by byte instead, as hex and ASCII side by side, paged from disk
  however large, every difference coloured and stepped through while a scan
  finds them in the background, and the binary template's field under the
  cursor named.
- **Synchronize directories** — mirror one panel's tree onto the other, in
  **one-way** (optionally deleting whatever the source doesn't have) or
  **two-way** (newer file wins) mode. The plan is **previewed in full** — every
  copy and delete, with totals — before a byte moves, and then runs through the
  ordinary transfer engine, so it shows progress, aborts, and can be sent to the
  **background**: *"mirror this folder to my SFTP server while I keep working"* is
  two dialogs.
- **Checksum** — compute a CRC32/MD5/SHA-1/SHA-256/SHA-512 digest of a file with
  a progress bar, and optionally verify it against a pasted reference checksum.
- **Send over LAN** (File menu) — share the highlighted file with a nearby phone
  or laptop: a one-shot HTTP server starts on a free port bound to your LAN IP,
  and the download URL is shown as a **QR code** (pixel graphics, or half-block
  cell art as a fallback). Select several files or a directory and they are
  zipped first (with a progress bar); the box shows a live download count and the
  server stops when you close it.
- **Receive over LAN** (File menu) — the other way round: scan the QR code with a
  phone and pick photos or files on the page it opens, and they land in the
  active panel's directory, with progress on both ends. The URL carries a random
  token, a taken name gets a ` (1)` instead of being overwritten, and a transfer
  that is cut off leaves nothing behind.
- **Auto-refreshing panels** — a panel re-reads itself when something else changes
  the directory it is showing, so a build or a `git checkout` in another window
  shows up without `Ctrl-R`.
- **System clipboard (`Ctrl-Ins`)** — copy the cursor's path, its bare name, or every
  marked path (one per line) to the **system** clipboard; in the editor `Ctrl-C` /
  `Ctrl-X` put the marked block there too. It uses the terminal's **OSC 52**
  sequence rather than a clipboard daemon, so it needs no X or Wayland session and
  **works over SSH** — copying on a remote server lands the text on the clipboard
  of the machine in front of you.
- **Archives** — browse and *edit* `.zip`, `.tar(.gz/.bz2/.xz/.zst)` and `.7z`
  like directories: copy and move files in and out, make and delete
  subdirectories, rename, move things around inside the archive, and compress a
  selection. RAR is read-only, and a plain `.zst` opens as the single file it
  holds, already decompressed.
- **Packages browsed like directories** — press Enter on a `.deb` or an `.rpm`
  and see what it installs, read natively so mc's scripts, `dpkg-deb` and `rpm`
  are no longer needed. A `.deb` shows its files at the root with the control
  files under `/DEBIAN`; an `.rpm` shows its payload, in both the classic layout
  and the indexed one rpm 4.14 and later write. An Arch `.pkg.tar.zst` is just a
  `.tar.zst` and opens as one. Both are read-only.
- **More things browsed like directories** — press Enter on a **disc image**
  (`.iso`, read natively with Joliet long names and Rock Ridge permissions and
  symlinks, so mc's `iso9660` script and `isoinfo` are no longer needed), a
  **SQLite database** (tables as folders, rows as `column = value` files, a
  `_schema.sql` at the top, big tables paged, opened read-only and *immutable* so
  a live database is safe to look at), or a **JSON/TOML document** (objects as
  folders, scalars as files, so a setting six levels down is something you `cd`
  to and **F3**). Each confirms what the file really is before claiming it, so a
  `.db` that isn't one opens the way it always did. All read-only, and they say
  so before a transfer starts.
- **Remote filesystems** — SFTP, SCP, FTP and **FTPS** (explicit TLS: certificates
  the system trusts are accepted, and a self-signed one is shown by its SHA-256
  and pinned once you trust it, the way `known_hosts` works), each mounted into a
  panel; copy/move/delete works transparently across local, remote and archive
  panels.
  On an **SFTP/SCP** panel, the command line and **Ctrl-O** run a shell
  on the **remote host** over the same SSH connection — its output on the same
  console backdrop, no second login. Hosts from **`~/.ssh/config`** are offered
  in the connect form and the command palette, and their `HostName`, `User`,
  `Port`, `IdentityFile` and **`ProxyJump`** chains are honoured.
- **3D view** — a panel format that draws the directory the *other* panel is in. Two styles,
  chosen in Settings → Panels: **Cubes**, a tree of boxes joined by lines, and
  **Spare no expense**, an homage to IRIX's *fsn* — pale platforms standing on a
  ground plane under a sky gradient, joined by lines running over the ground,
  with the files on them drawn as solids **shaped and coloured by file type**.
  True-pixel on a graphics terminal, half-blocks or an ASCII ramp elsewhere.
- **Disk explorer** (treemap of disk usage), **process explorer** (btop-style
  system monitor), and a **disk manager** (Linux) to mount/unmount/format/sync
  drives and **flash or image** raw disk images.
- **Network connections** (Linux) — listening ports with their programs and all
  active connections with their type, service, live per-connection traffic rate
  (with a sparkline) and a details view; filter, sort, kill the owning process,
  and an optional root password for full visibility. A **per-service overview
  diagram** (Tab) groups connections into colour-coded cards showing each peer IP
  and its direction, with clickable/navigable addresses and reverse-DNS lookups.
- **Look & feel** — many color themes (fully customizable via `themes.toml` or
  the visual theme editor), truecolor gradients on **any element** — panel and
  dialog backgrounds, frames, cursor bars, menus, inputs and buttons each take
  their own two-color ramp, in one of four directions, animated or still, and
  nearly every preset ships with a set — an optional CPU/memory status widget,
  optional **Nerd Font file-type icons** in the listings, a configurable
  **F2 user menu**, and a Norton-Commander-style **screensaver** — starfield,
  matrix rain, a bouncing clock or growing pipes, all in plain text.
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
  Network Connections are available. The command line and `Ctrl-O` shell run the
  classic *suspend-and-run* way on Windows (the TUI pauses while `cmd.exe` runs,
  then resumes) rather than the persistent behind-the-panels console used on
  Unix, so there is no live console backdrop there.

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
| `Shift-F4` | Edit a new file (asks for the name) |
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
| `Ctrl-P` | Command palette (fuzzy-search every action, setting, bookmark, connection) |
| `Ctrl-\` | Directory hotlist (bookmarks): jump / add / remove |
| `Alt-←` / `Alt-→` (or `Alt-y` / `Alt-u`) | Go back / forward through the panel's visited directories |
| `Alt-H` | Directory history: pick any visited directory from a list |
| `Alt-I` | Point the other panel at this panel's directory |
| `Alt-O` | Show the cursor's directory on the other panel, and step down one entry |
| `Alt-T` | Cycle view format (full / brief / details / tree / 3D / thumbnails) |
| `Alt-Shift-I` | Set / clear the panel's persistent listing filter |
| `Alt-Shift-H` | Shell history window (recall a command without running it) |
| `Alt-G` | Open the **Git menu** (status, log, commit, push/pull, checkout, …) |
| `Ctrl-G` / `Alt-D` | Git: stage/unstage the selection · diff the file against HEAD (`s` stages a hunk there) |
| `Ctrl-Ins` | Copy the selected paths (or the cursor's) to the system clipboard |
| `Ctrl-R` | Re-read the active panel |
| `Alt-S` / `Ctrl-S` | Quick search the active panel (jump to the first matching name) |
| `Ctrl-E` | Toggle reverse sort order |
| `Ctrl-X` | Toggle vertical / horizontal split |
| `Ctrl-U` | Swap the two panels |
| `Ctrl-F1` / `Ctrl-F2` | Hide / show the left / right panel (reveals the console) |
| `Ctrl-F4` | Toggle half-height panels (reveals the console below) |
| `Ctrl-F5` | Show / hide the command prompt (hidden: typing starts a quick search) |
| `Alt-F1` / `Alt-F2` | Drive / volume / connection picker (left / right panel) |

### Viewer (F3)

| Key | Action |
| --- | --- |
| `F1` | Help (opens the user manual) |
| `F2` | Toggle line wrap — (audio) switch Spectrogram / Waveform |
| `F4` | Cycle text / hex / byte-map mode (and the binary view, for an executable) |
| `F5` | Goto (line / percent / byte offset) |
| `F7` | Search (`n` repeats) |
| `f` | Follow the file as it grows (`tail -f`) |
| `b` | Git blame column; `Enter` opens the cursor line's commit |
| `F8` | (Markdown) toggle Raw / Render — (CSV/TSV) toggle Table / Raw — (image) toggle Image / Raw — (model) toggle Model / Raw — (audio) toggle Audio / Raw — (map) toggle Density / Bytes — (binary) toggle demangled / raw names — (certificates) toggle Certs / Raw |
| `Space` / `s` | (audio) play / pause — stop |
| `← →` / `PgUp PgDn` / `Home End` | (audio) seek 5 s / 30 s / to either end; click or drag on the picture to seek |
| `+` / `-` / `↑ ↓` | (audio) volume |
| `← → ↑ ↓` / `Tab` / `<` `>` | (table) move the cell cursor / step through the cells / narrow or widen the column; `F2` toggles the header row |
| `Tab` / `Shift-Tab` / `1`–`7` | (binary) switch between Info, Sections, Libraries, Imports, Exports, Functions and Strings |
| `Enter` / `Esc` | (binary) open the hex view at the highlighted row / drop a *Find all* filter |
| `← → ↑ ↓` / `Enter` | (map) move the cursor / open the hex view at that offset |
| `← → ↑ ↓` | (model) orbit the camera |
| `+` / `-` | (model) zoom in / out |
| `Home` | (model) re-frame the model |
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
| `Shift-F5` | Insert a file at the cursor |
| `F7` / `Shift-F7` | Search / search again |
| `F9` | Pulldown menu |
| `Shift-F9` | Toggle word wrap |
| `Ctrl-F9` | Toggle in-place hex editor |
| `F5` / `Shift-F5` (hex) | Choose / rerun the binary template |
| `F6` (hex) | The template variable under the cursor (tree: `Enter` edits, `←`/`→` close/open) |
| `F8` (hex) | Data inspector: the bytes at the cursor as numbers, characters, dates, GUIDs (`Enter` edits, `b` switches byte order) |
| `Alt-E` / `Alt-Shift-E` | (JSON, TOML, YAML, XML) next / previous syntax error |
| `Alt-F` | (JSON) pretty-print; Format → JSON also minifies and sorts keys |
| `Alt-M` | Show the file's GeoJSON on a world map (drag/wheel to pan and zoom, click a feature, `Enter` to go to it); `e` edits it: `1`/`2`/`3` draw a point/line/polygon, drag positions, `Del` removes, `Ctrl-Z` undoes |
| `Alt-G` | Toggle the spreadsheet grid (CSV/TSV): `Enter` or typing edits a cell, `F5`/`F6` insert a row/column, `F8`/`Shift-F8` delete one, `F3` toggles the header row |
| `Alt-T` / `F3` | (audio files) Toggle the tag page and the file's bytes: `Enter` or typing edits a tag, `F5` adds one, `F8` clears one, `F2` writes them back |
| `Ins` | Toggle insert / overwrite |
| `Ctrl-C` / `Ctrl-X` / `Ctrl-V` | Copy / cut block to clipboard, paste |
| `Ctrl-Z` / `Ctrl-Y` | Undo / redo |
| `Ctrl-A` | Mark the whole file |
| `Ctrl-N` / `Ctrl-F` | New buffer / copy block to a file |
| `Ctrl-S` / `Ctrl-L` | Toggle syntax highlighting / repaint the screen |
| `Alt-L` / `Alt-B` | Go to line / matching bracket |
| `Alt-P` / `Alt-T` / `Alt-U` | Format paragraph / sort block / paste command output |
| `Alt-K` / `Alt-J` / `Alt-I` / `Alt-O` | Bookmark: toggle, next, previous, flush |
| `Esc` / `F10` | Quit (prompts if modified) |

### Dialogs

`Tab`/arrows move between fields **and onto the OK/Cancel buttons**, `Space`
toggles checkboxes and cycles choices, `Enter` confirms, `Esc` cancels (and
aborts progress dialogs, including long-running git network operations and the
directory-sync scan). The OK/Cancel and Yes/No buttons are also clickable.

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

Requires a recent stable Rust toolchain (edition 2024, **Rust ≥ 1.89**), plus a
C/C++ compiler for the bundled `unrar` and SQLite libraries — add
`--no-default-features` to build without RAR and SQLite-browsing support if
you'd rather not have one. On Linux, audio playback links the system ALSA
library, so install its headers first (`sudo apt install libasound2-dev` on
Debian/Ubuntu, `alsa-lib` on Arch); `--no-default-features` leaves playback out
too (the audio view still draws files), and `--features audio` adds it back.

The quickest route is to build straight from the repository:

```sh
cargo install --git https://github.com/dividebysandwich/rat-commander
```

Or clone first, if you want to hack on it:

```sh
git clone https://github.com/dividebysandwich/rat-commander
cd rat-commander
cargo install --path .      # installs `rc` into ~/.cargo/bin
# or just run it:
cargo run --release
```

Either way `rc` lands in `~/.cargo/bin`, so make sure that's on your `PATH`.

---

## Building & packaging

```sh
cargo build --release            # target/release/rc
cargo test                       # run the test suite
cargo clippy --all-targets       # lints
```

Release binaries are stripped and optimized via the `[profile.release]` settings
in `Cargo.toml`.

Every push to `main` and every pull request runs
`.github/workflows/ci.yml`, which is exactly:

```sh
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

CI does not gate on `cargo fmt`, but the tree *is* rustfmt-clean: `rustfmt.toml`
sets `use_small_heuristics = "Max"` so the compact style the source is written in
survives, and `cargo fmt` is the easiest way to match it.

### Cross-compiling and packages

The `.github/workflows/release.yml` workflow builds every artifact. To reproduce
a build locally:

```sh
# Debian package (native arch)
cargo install cargo-deb
cargo build --release --target x86_64-unknown-linux-gnu
cargo deb --no-build --target x86_64-unknown-linux-gnu

# Raspberry Pi (cross-compiled) – needs Docker + `cross`
# (--no-default-features drops RAR, whose C++ lib won't cross-compile here;
# Cross.toml installs the target's ALSA headers for the `audio` feature)
cargo install cross
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_PATH_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig \
cross build --release --no-default-features --features audio --target aarch64-unknown-linux-gnu
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
C++ `unrar` library doesn't build with those cross toolchains. Audio playback
(`audio`, on by default) links ALSA on Linux — the `.deb` depends on `libasound2`
— and is kept in the 64-bit Raspberry Pi package. The 32-bit (`armhf`) package
leaves it out and is built with `cargo deb --variant=noaudio`, which drops the
ALSA dependency: Debian 13 and the Raspberry Pi OS built on it moved 32-bit ALSA
to a 64-bit `time_t`, which a 32-bit Rust binary cannot safely call. It still
draws audio files; it just cannot play them.

---

## Configuration

Configuration lives in your platform config directory
(`~/.config/rat-commander/` on Linux): **`config.toml`** (written from the
Settings dialog), **`themes.toml`** (editable color themes), **`lang/`**
(one editable TOML per UI language), **`templates/`** (the hex editor's binary
templates, editable, plus your own), and **`menu`** (the F2 user menu, in
Midnight Commander format). See the
**[user manual](doc/MANUAL.md#configuration)** for details.

---

## Third-party content

The binary templates in `assets/templates/` come from SweetScape's public
template repository, whose terms place them in the public domain; a few carry
their own notes (see `assets/templates/README.md`). The JSON Schemas in `assets/schemas/` are
separate works bundled unmodified under their own licenses — Apache-2.0
(SchemaStore, compose-spec) and MIT (GitLab CI); see
`assets/schemas/README.md`.

## License

GNU General Public License, version 2 (GPL-2.0-only). See the `LICENSE` file.

The built-in world map is made from [Natural Earth](https://www.naturalearthdata.com/)
data, which is in the public domain; `assets/world/make_world.py` rebuilds it.

The bundled binary templates come from SweetScape's
[010 Editor template repository](https://www.sweetscape.com/010editor/repository/templates/),
whose contributors release them into the public domain; a few carry their own
notes in their headers, kept intact. `assets/templates/README.md` lists them
with their authors, and `assets/templates/fetch_templates.py` fetches them
again.
