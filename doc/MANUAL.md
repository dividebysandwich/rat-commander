# Rat Commander — User Manual

Rat Commander (`rc`) is a two-panel terminal file manager in the tradition of
Norton Commander and Midnight Commander, with a batch of modern built-in tools:
a file viewer and editor with syntax highlighting, archive browsing, remote
(SFTP / FTP / SCP) clients, a disk-usage explorer, a process explorer, and (on
Linux) a disk manager. It needs no external programs for its core features.

Press **F1** inside the program to read this manual at any time.


## The screen

The window is divided into four areas, top to bottom:

- **Menu bar** (top row) — `Left  File  Command  Options  Right`. Open it with
  **F9** (or `Alt` + its first letter).
- **Two panels** — the heart of the program. Each shows the contents of one
  directory. The **active panel** has a highlighted (brighter) border; it is the
  one your keystrokes act on. The other panel is usually the destination for
  copy/move operations.
- **Command line** (second from bottom) — type a shell command here and press
  Enter to run it in the active panel's directory. Recently run commands are
  remembered across sessions: cycle them with `Alt-P` / `Alt-N`, or press `Alt-H`
  to pick one from the **Shell History** window. `Alt-Enter` drops the name under
  the cursor onto the command line.
- **Function-key bar** (bottom row) — shows what F1–F10 do in the current
  context. The labels also work as buttons: click one to run it.

Each panel's bottom border shows the volume's **free / total** disk space, and a
mini status line under the listing shows the full name of the highlighted file.


## Getting started — two-panel basics

- **Switch the active panel** with **Tab**. Everything you do (open, copy,
  select…) happens in the active panel.
- **Move the cursor** with the arrow keys, **PgUp** / **PgDn**, **Home** /
  **End**.
- **Enter a directory** by moving the cursor onto it and pressing **Enter** (or
  double-clicking it). The `..` entry at the top goes up to the parent.
- **Open a file** with **Enter** to hand it to the system default application
  (`xdg-open` on Linux). Use **F3** to view it or **F4** to edit it in the
  built-in tools instead.
- **Copy** the highlighted file (or the selected files) to the *other* panel
  with **F5**; **move/rename** with **F6**; **delete** with **F8**. Because the
  other panel is the default destination, the usual workflow is: point one panel
  at the source, the other at the destination, then press F5/F6.
- **Make a directory** with **F7**.

The active panel always provides the *source* for operations, and the inactive
panel the *destination* — so two panels make copying and moving between two
places fast and obvious.

### The mouse

The mouse works throughout:

- **Left-click** a file to move the cursor to it (and activate that panel);
  **double-click** to open it (like Enter).
- **Right-click** a file to invert its mark (tag/untag it).
- **Drag** with the left button to carry the cursor; **right-drag** flips the
  mark of every file it sweeps over (each file once).
- Click a **menu-bar title** to open it and an entry to run it.
- Click the bottom **F-key bar** to run that function.
- Click **OK / Cancel** (or **Yes / No**) buttons in dialogs, and the
  **Abort** button on a scan/search progress dialog.
- In any dialog with input fields — **Copy/Move**, **Make directory**,
  **Chmod**, **Chown**, **Checksum**, **Select/Unselect group**, the
  **FTP/SFTP/SCP connection** form, **Settings**, **Confirmations**, **Find
  file**, editor **Search/Replace**, and more — click a **text field** to focus
  it and place the caret, click a **checkbox** or **radio** to toggle it, click a
  **dropdown** to open and pick from it, and click **OK/Cancel** to finish.
- Click an entry in the **user menu** (F2) or the **shell-history** window to run
  / recall it.
- In the **disk explorer**, click a box to select it and **double-click** to
  enter that subdirectory.


## Keyboard shortcuts

On terminals where the function keys are awkward, every `Fn` shortcut also has a
Midnight-Commander-style alias: press **Esc** then a digit — `Esc 1` … `Esc 9`
for `F1`…`F9`, and `Esc 0` for `F10` (works in the panels, viewer and editor).
A quick **Alt** + digit does the same.

### Panels

- `F1` — Help (this manual)
- `F2` — User menu (configurable)
- `F3` — View file
- `F4` — Edit file
- `F5` — Copy
- `F6` — Rename / move
- `Shift-F6` / `Ctrl-F6` — Multi rename (the selected files)
- `F7` — Make directory
- `F8` — Delete
- `F9` — Pulldown menu (Left/Right follows the active panel)
- `F10` — Quit (with confirmation)
- `Ctrl-Q` — Quit immediately
- `Tab` — Switch the active panel
- `↑ ↓` / `PgUp PgDn` / `Home End` — Move the cursor
- `Enter` — Open dir / enter archive / open file / run the command line
- `cd <dir>` + `Enter` — Change the active panel's directory
- `Insert` / `Ctrl-T` — Tag the file and advance
- `+` / `-` / `*` — Select / unselect a group (by wildcard) / invert the selection
- `← →` — Move within the command line
- `Alt-Enter` — Copy the name under the cursor onto the command line (appended,
  and shell-quoted when it contains spaces or special characters)
- `Alt-P` / `Alt-N` — Recall the previous / next command from history into the
  command line, replacing its contents (press again to keep cycling)
- `Alt-H` — Open the **Shell History** window just above the command line: move
  with `↑`/`↓` (or `Alt-P`/`Alt-N`) and press `Enter` to copy the chosen command
  into the command line **without running it**; `Esc` or `Alt-H` closes it
- `Alt-S` / `Ctrl-S` — **Quick search** the active panel: opens an empty search
  box; each letter you type filters, jumping the cursor to the first file whose
  name starts with it (case-insensitive; `Shift` for uppercase works). The box
  stays open even when empty — `Backspace` trims it, and only `Esc` or an arrow
  key dismisses it. `Enter` opens the match
- `Ctrl-O` — Toggle the persistent subshell (press again to return)
- `Ctrl-R` — Re-read (refresh) the active panel
- `Ctrl-E` — Toggle reverse sort order (choose the sort key from the panel menu)
- `Ctrl-W` — Cycle the view format (full → brief → details → tree)
- `Ctrl-X` — Toggle vertical / horizontal split
- `Ctrl-U` — Swap the two panels
- `Alt-F1` / `Alt-F2` — Drive / connection picker for the left / right panel
- `Alt` + a menu letter (`F`/`O`/`C`/`L`/`R`) — Open that top menu (Midnight-
  Commander style); `F9` opens the menu bar too

### Editing text input lines

The command line and every dialog input field (copy/move destination, make
directory, find, rename, connection details, …) share the same Emacs/readline
key bindings:

- `Ctrl-A` / `Ctrl-E` — Move to the beginning / end of the line
- `Ctrl-B` / `Ctrl-F` — Move one character left / right
- `Alt-B` / `Alt-F` — Move one word backward / forward
- `Ctrl-H` / `Backspace` — Delete the previous character
- `Ctrl-D` / `Delete` — Delete the character under the cursor
- `Alt-Backspace` / `Alt-Ctrl-H` — Delete the previous word
- `Ctrl-@` (or `Ctrl-Space`) — Set the mark for cutting
- `Ctrl-W` — Cut the text between the mark and the cursor into the kill buffer
- `Alt-W` — Copy that text into the kill buffer (without removing it)
- `Ctrl-K` — Kill (cut) from the cursor to the end of the line
- `Ctrl-Y` — Yank (paste) the kill buffer at the cursor

The kill buffer is shared, so text cut in one field can be yanked into another.
On the **command line only**, `Ctrl-E`, `Ctrl-W` and `Alt-F` keep their panel
meaning (reverse sort / cycle view / File menu) while the line is empty, and
switch to editing as soon as it has text.

### Viewer (F3)

- `F2` — Toggle line wrap
- `F4` — Toggle hex / text mode
- `F5` — Goto (line / percent / byte offset)
- `F6` — (Markdown files) show the document outline — a tree of the headings.
  Use `↑ ↓` / `PgUp PgDn` / `Home End` or the mouse to pick a heading, `Enter`
  (or a click) to jump to it, `Esc` / `F6` to dismiss. Opening this manual with
  `F1` lands on its outline.
- `F7` — Search
- `F8` — (Markdown files) toggle Raw / Render
- `n` — Repeat the last search
- `↑ ↓` / `PgUp PgDn` / `Home End` — Scroll
- `F3` / `Esc` / `F10` / `q` — Close (F3 toggles the viewer, as in the panels)

### Editor (F4)

- `F1` — Editor shortcut help (any key closes it)
- `F2` — Save
- `Shift-F2` / `Ctrl-F2` — Save as… (browse + name)
- `F3` — Start / end a block mark
- `F4` — Search & replace
- `F5` — Copy the block to the cursor
- `F6` — Move the block to the cursor
- `F8` — Delete the block
- `F7` — Search
- `Ctrl-C` / `Ctrl-V` — Copy the block to the clipboard / paste
- `Ctrl-Z` / `Ctrl-Y` — Undo / redo
- `Shift+arrows` (or `Shift+Ctrl-arrows`) — Mark text while moving
- `Ctrl-Home` / `Ctrl-End` — Start / end of the document
- `Ctrl-← / →` — Move by word
- `F9` — Toggle the in-place hex editor
- `Shift-F9` / `Ctrl-F9` — Toggle word wrap
- `Esc` / `F10` — Quit (prompts if modified)

While **Shift** or **Ctrl** is held, the F-key bar relabels **F2 → Save as**
and **F9 → Wrap** to show those alternates.

The editor remembers where you left the cursor in each of the last **50** local
files (in `editor-positions.toml`); re-opening a file restores the cursor and
scrolls so it sits in the vertical center of the view.

### Hex editor (F9 in the editor)

- `0`–`9`, `a`–`f` — Overwrite the current byte's nibble (hex column)
- typed character — Overwrite the current byte (ASCII column)
- `Tab` — Switch between the hex and ASCII columns
- `← ↑ ↓ →` / `PgUp PgDn` — Move; `Home` / `End` — start / end of row
- `Ctrl-Home` / `Ctrl-End` — Start / end of file
- `F7` — Search (hex bytes like `48 65` or text)
- `F4` — Replace all (same length, overwrite-only)
- `F2` — Save the changed bytes in place
- `F9` — Back to text mode
- `Esc` / `F10` — Quit (prompts if modified)

### Process explorer

- `↑ ↓` / `PgUp PgDn` / `Home End` — Move the selection
- `Tab` — Switch between the flat list and the process tree
- `→` / `←` / `Enter` / `Space` (tree mode) — Expand / collapse the selected process (`←` on a collapsed row jumps to its parent)
- `*` (tree mode) — Collapse every subtree, or expand them all again
- `c` / `m` / `t` / `n` / `u` / `p` — Sort by CPU / memory / threads / program / user / PID (again to reverse)
- `r` — Reverse the sort order
- `+` / `-` — Adjust the refresh interval
- `k` / `F8` / `F9` / `Del` — Kill the selected process (SIGTERM, with confirm)
- `K` — Force-kill (SIGKILL, with confirm)
- `Esc` / `F10` / `q` — Close

### Disk explorer

- `← ↑ ↓ →` — Move the selection between boxes
- `Enter` — Dive into the selected subdirectory
- `Backspace` — Go up to the parent
- `g` / `Ctrl-Enter` — Exit and open the selected directory in the active panel
- `Esc` / `F10` / `q` — Close

### Dialogs

`Tab` / arrows move between fields, `Space` toggles checkboxes and cycles
choices, `Enter` confirms, `Esc` cancels. Progress dialogs can be aborted with
`Esc`. You can also click the buttons with the mouse.

### Theme editor

Opened from **Options → Edit themes…**. `Tab` / `Shift-Tab` cycle the four panes
(theme picker, color list, color picker, buttons).

- **Theme picker** — `↑ ↓` / `← →` choose the theme to edit; `Home` / `End` jump
  to the first / last. Switching with unsaved edits prompts to save, discard, or
  cancel.
- **Color list** — `↑ ↓` / `PgUp PgDn` / `Home End` select the element to
  recolor; `Enter` / `→` jump to the color picker.
- **Color picker** (truecolor) — `↑ ↓` pick the R / G / B channel; `← →` adjust
  it by 1, `Shift-←` / `Shift-→` by 20, `PgUp` / `PgDn` by 16, `Home` / `End`
  set it to 0 / 255; `Enter` returns to the list. You can also **type a six-digit
  hex code** (e.g. `1a2b3c`) to set the color directly — `Backspace` edits it and
  `Esc` cancels the entry. On a 16-color terminal the picker is a swatch grid
  moved through with the arrows.
- **Buttons** — `← →` move between **Save**, **Save as…** and **Cancel**;
  `Enter` / `Space` activates.
- `F2` / `Ctrl-S` — Save; `Esc` / `F10` — Close (prompts if there are unsaved
  changes).
- **Mouse** — click a row in the color list to select it and the wheel scrolls
  it; click a channel bar to set its value; click a swatch; click **Save** /
  **Save as…** / **Cancel** or the confirmation-dialog buttons.

The right-hand **preview** updates live, showing whichever surface the selected
element affects: the file panels, a demo dialog, or a small editor.


## Selecting (tagging) files

Most operations act on the **selection** — the set of *tagged* files — or, when
nothing is tagged, on the file under the cursor.

- **Tag the current file and advance** with **Insert** (so you can tag a run of
  files quickly).
- **Right-click** a file (or right-drag across several) to toggle tags with the
  mouse.
- **Select a group** with **`+`**: a dialog asks for a pattern. By default it is
  a shell wildcard (`*.txt`, `img_??.png`); untick *Using shell patterns* to use
  a regular expression. *Files only* limits it to files, and *Case sensitive*
  controls matching.
- **Unselect a group** with **`-`** (same dialog).
- **Invert the whole selection** with **`*`**.

Tagged files are shown in the selection color. The mini status line reports how
many are tagged and their combined size.


## View formats and sorting

Each panel can show its listing three ways; cycle them with **Ctrl-W** or pick
one from the **Left** / **Right** menu:

- **Full** — one file per row with name, size and modification time.
- **Brief** — names only, in multiple columns (more files at a glance).
- **Details** — the panel shows no listing of its own; instead it displays
  **information about whatever the *other* panel points at**:
  - on a **file**, a full overview — name, path, type, size (and exact byte
    count), permissions, owner/group, timestamps and inode;
  - on a **directory**, the **total recursive size** of everything beneath it,
    computed in the background and updated live as it scans (so even large or
    remote trees stay responsive);
  - on a **multi-file selection**, a tally of the combined size and the number
    of files and directories included.

  This is useful for inspecting a file's metadata, or measuring how much space a
  folder or a set of tagged items uses, while you browse with the other panel.
- **Tree** — the directory structure is visualized as a tree, arrow keys navigate,
  pressing enter changes the opposite panel's directory and opens up the directory
  structure underneath.

**Sorting** is configurable from the **Left** / **Right** menu; **Ctrl-E** toggles
reverse order. The keys are: Unsorted, Name, Extension, Size,
Modify / Access / Change time, or Inode — with reverse, case-sensitive and
executables-first toggles.

Filenames carry an `ls -F`-style **type marker** so kinds read by symbol, not
just color: `/` directory, `*` executable, `@` symlink, `!` broken symlink, and
a leading space for plain files (keeps names aligned). File **names are colored**
by type as well: archives, documents, images and audio/video each get a hue.


## File operations

### Copy, move, delete

**F5** copies, **F6** moves/renames, **F8** deletes the selection (or the file
under the cursor). Copy and move open a dialog with the destination prefilled to
the other panel's directory — edit it to copy somewhere else, then confirm.

Long operations show a **progress window** with a per-file gauge, an overall
gauge, and a live **transfer-speed chart**. Press **Esc** to abort; a partly
written destination file is cleaned up.

**Overwrite handling.** When a destination already exists, a prompt offers
**Yes** / **No** for that file, **Append**, or a rule applied to all remaining
files — **All**, **Older** (only if the source is newer), **None**,
**Smaller**, or **Size differs** — with an optional guard that refuses to
overwrite a file with a zero-length one.

These operations work **transparently between local, remote and archive
panels**, so copying a file onto an SFTP server or out of a `.zip` is the same
F5 you already use.

### Background file transfers

A **To Background** button is available on most progress dialogs and sends the
currently running operation into the background. Multiple copy or file transfer
operations can run in parallel. A total progress bar will be shown on the top 
menu bar, and a list of all running background operations can be shown via 
**File → Background operations**

### Make directory, rename

**F7** makes a directory. **F6** on a single file renames it (or moves it if you
give a path). For renaming many files at once, use Multi rename (below).

### Permissions, ownership, symlinks

From the **File** menu:

- **Chmod** — set the permission bits of the selected files with checkboxes (the
  resulting octal mode is shown). A **Recurse into directories** checkbox applies
  the change through any directories in the selection.
- **Chown** — set the owner and group of the selected files (by name or numeric
  id), with the same recursion option.
- **Symlink** — create a symbolic link in the *other* panel pointing at the file
  under the cursor (both fields are prefilled and editable).


## Multi rename

*File menu → Multi rename…*, or **Shift-F6** / **Ctrl-F6**.

Batch-renames the **tagged** files using a naming mask, with a
live two-column preview — original names on the left, projected names on the
right — that scroll together so you can check each result before committing.

**Useful for** numbering a set of photos, normalizing extensions or case,
stripping or inserting text across many files at once.

**Usage:** Tag the files, open Multi rename, type a mask, watch the
right column update, then press **Execute**.

**The mask** is plain text plus placeholders that pull pieces from each original
name:

- `[N]` — the name without extension; `[N1-3]` a slice of it (characters 1–3),
  `[N3-]` from character 3 to the end, `[N2]` a single character
- `[E]` — the extension; `[E1-2]` a slice of it
- `[C]` — a running counter
- `[YMD]` — the date (`YYYYMMDD`)
- `[hms]` — the time (`HHMMSS`)

**Options.**

- **Case** — leave the case unchanged, force lowercase, or force UPPERCASE.
- **Counter** — set the start value, the step, and the number of digits
  (zero-padded) for `[C]`.
- **Search & replace** — replace a substring in the generated names, with a
  case-sensitivity toggle.

Renames run in two phases through temporary names, so swaps and renumberings
can't clobber a file that hasn't been renamed yet, and an existing file outside
the batch is never overwritten.


## Find file

*Command menu → Find file…*

Searches a directory tree for files matching a name pattern
(and optionally containing some text), then *panelizes* the results into the
active panel.

**Useful for** locating a file when you only remember part of its name, or
finding every file that mentions a string.

**Usage:** Open the dialog, set the start directory, the file-name
pattern, and (optionally) content to look for, then run it. A live progress
dialog counts matches; press **Esc / Enter** to stop early — the results found
so far are kept. The matches replace the panel listing with a flat list; a `..`
entry at the top returns to normal browsing.

**Options** include recursive search, case sensitivity, skip-hidden, and shell-
wildcard vs. regular-expression name matching. On a **remote** panel the search
matches **file names only** (content search is local).


## Compare directories

*Command menu → Compare directories…*

Compares the two panels' directories and **tags the files that
differ**, so you can act on just those.

**Useful for** spotting what changed between two copies of a tree, or what is
missing on one side.

**Modes.**

- **Quick (name)** — tag files present in one panel but not the other.
- **Size only** — also tag the larger of two files that share a name but differ
  in size.
- **Content** — tag both files whenever their bytes differ.


## Find duplicates

*Command menu → Find duplicates…*

Tags the files that are **identical between the two panel
directories**, by criteria you choose.

**Useful for** finding copies of the same file in two places before deleting the
redundant ones.

**Usage:** Point the two panels at the directories to compare, open
the dialog, choose what "identical" means, and run it. A cancellable progress
dialog runs the comparison — important for content comparison and remote
filesystems, where it can take a while.

**Options.** File names are always compared; tick any of **size**, **date/time**
and **content** to require those to match too (with none ticked, only names are
compared). A **Case-sensitive** name-match toggle is on by default.


## Compare files (side-by-side diff)

*Command menu → Compare files…*

Opens a full-screen, side-by-side **diff** of the two files
under the cursor in each panel, with changed and added blocks highlighted and
connected by gutter guides.

**Useful for** reviewing differences between two versions of a file and merging
selected changes between them.

**Operation.**

- `↑ ↓` moves through the document and selects the active change.
- `Ctrl-↑ / ↓` jumps to the previous / next change.
- `Ctrl-←` applies the active change from the right file to the left (or deletes
  a left-only block); `Ctrl-→` applies it the other way.
- Edits happen in memory; **F2** asks to save and writes the changed file(s)
  back to disk. **Esc** closes (prompting save / discard / cancel when there are
  unsaved changes).


## Checksum a file

*File menu → Checksum…*

Computes a checksum of the file under the cursor and, if you
paste a reference checksum, tells you whether they match — handy for verifying a
download against the digest published alongside it.

**Operation.**

- Pick the algorithm (**CRC32**, **MD5**, **SHA-1**, **SHA-256**, **SHA-512**),
  optionally paste a checksum into *Compare to* to check against, and press
  **OK**.
- A progress bar tracks the calculation while the file is read (**Esc** aborts).
- The result dialog shows the computed digest. When you supplied a comparison
  value it also shows a green **✓ MATCH** or red **✗ MISMATCH** verdict (the
  comparison ignores case and whitespace). Press **OK** to close it.

Works on local files, files inside archives, and files on a remote panel.


## The viewer (F3)

A read-only file viewer with text and hex modes, search,
syntax highlighting, and a Markdown render mode.

**Useful for** quickly reading a file — including very large ones — without
loading it into an editor.

**Operation and options.**

- **Text / Hex** — **F4** toggles. Hex mode shows an offset / hex / ASCII dump.
- **Line wrap** — **F2** toggles soft wrapping.
- **Search** — **F7** searches; **`n`** repeats. Search streams the file, so it
  works on huge files too.
- **Goto** — **F5** jumps to a line number, a percentage through the file, or a
  decimal/hex byte offset (in hex mode the line number is a 16-byte row).
- **Syntax highlighting** colors recognized source files, using a bundled theme
  matched to the active light/dark UI. It covers syntect's default languages
  plus bundled extras (TOML, INI, Dockerfile, HCL/Terraform, GraphQL, Protobuf,
  CMake, TypeScript/TSX, Kotlin, Swift, SCSS/Sass, Elixir, Zig, Nix and more).
- **Markdown view** — `.md` files open *rendered*: the markup (`#`, `**`, `` ` ``,
  links, …) is hidden, headings are colored by level, emphasis and inline code
  are styled, and list bullets and rules are drawn. Press **F8** (*Raw*) to see
  the raw source (still syntax-highlighted) and **F8** again (*Render*) to go
  back.
- **Hex-color swatches** — any `#rgb` / `#rrggbb` / `#rrggbbaa` token in the
  text has its `#` painted in the color it names, so colors in code and configs
  are visible at a glance.

The viewer is **paged from disk** — local files are read on demand, so even
multi-gigabyte files open instantly. Viewing a large file over a remote
connection streams it to a temporary copy first, behind a progress dialog you
can abort.


## The editor (F4)

An `mcedit`-style text editor with block operations, search
and replace, undo/redo, syntax highlighting, and an in-place hex editor.

**Useful for** quick edits without leaving the file manager.

**Launching straight into the editor.** Open a file in the editor without going
through the panels by starting the program as **`rc /edit <file>`** (a missing
file opens an empty buffer so you can create it). Omit the filename entirely —
**`rc /edit`** — to start on a blank, untitled buffer; the first save then acts
as **Save as**, prompting you for a name. The packages and installers also set
up an **`rcedit`** shortcut — a symlink to `rc` on Linux/macOS, a small
`rcedit.cmd` on Windows — so **`rcedit <file>`** (or bare **`rcedit`**) does the
same thing. In this mode, closing the editor exits the program (it does not drop
to the panels).

**Marking a block.** Mark text either with **Shift+arrows** (and
**Shift+Ctrl-arrows**) while moving, or with **F3** to start/end a mark. A
marked block **stays selected as you move the cursor** and **stays anchored to
its text across edits** — inserting or deleting before, after or inside it never
clears the selection (F3 again toggles a block off).

**Block operations.**

- **F5** — copy the block to the cursor position.
- **F6** — move the block to the cursor position.
- **F8** — delete the block.
- **Ctrl-C** / **Ctrl-V** — copy the block to the clipboard / paste it.

**Search and replace.** **F7** searches; **F4** opens search & replace, which
can be a literal or a regular expression.

**Saving.** **F2** writes the file in place. **Save as** (**Shift-F2** or
**Ctrl-F2**) opens a browser — navigate directories and type a file name,
prefilled with the current one — to write the buffer somewhere else; the editor
then continues editing the new file. If a normal save fails (a read-only
location, a permission error, …), the Save-as browser opens automatically with
the reason shown, so you can redirect the write without losing your work.

**Word wrap.** **Shift-F9** (or **Ctrl-F9**) toggles virtual word wrap: long
lines are shown across several screen rows without changing the file, and each
*continued* row ends in a **`>`** marker so soft wraps are distinguishable from
real line breaks. Cursor movement, scrolling and the mouse all follow the
visible (wrapped) rows; `WRAP` shows on the status line while it is on.

**Help.** **F1** brings up a list of the editor's keyboard shortcuts and what
they do; any key closes it. While **Shift** or **Ctrl** is held, the F-key bar
relabels **F2 → Save as** and **F9 → Wrap** to advertise those alternates (on
terminals that report held modifier keys via the enhanced keyboard protocol).

**Other.** **Ctrl-Z** / **Ctrl-Y** undo and redo. The status bar shows the byte
under the cursor, the line and column, and the totals. Syntax highlighting
updates incrementally as you type.

**Hex editor (F9).** Toggles an in-place offset / hex / ASCII editor. Only the
visible window is read and only changed bytes are written back, so arbitrarily
large files can be hex-edited (and a file too big to load as text opens straight
into hex mode). Editing is overwrite-only (length-preserving). **Tab** switches
between the hex and ASCII columns; **F7** searches for hex bytes (`48 65 6c`) or
text, **F4** replaces all (same length), **F2** saves the changed bytes.


## Archives — browsed like directories

Lets you walk into `.zip`, `.tar`, `.tar.gz`, `.tar.bz2`,
`.tar.xz`, `.7z` and `.rar` archives as if they were folders.

**Useful for** inspecting, extracting from, or adding to an archive without
unpacking it first.

**Operation.** Press **Enter** on an archive file to browse it. Copy files
**out** (F5 to a normal panel) or **in** (F5 from a normal panel into the archive
panel); **F8** deletes from the archive. To build a new archive, tag a selection
and use *File menu → Compress…*, choosing the format by the name you type
(`.zip`, `.7z`, `.tar.gz`, `.tar.bz2`, `.tar.xz`).

RAR archives are **read-only** — you can browse and extract them, but no tool can
create RAR archives. (RAR support is an optional build feature, on by default.)


## File associations and extfs (rc.ext)

Beyond the built-in archive formats above, Rat Commander can open a file type
with a command of your choosing — or browse it **as a directory** using a
Midnight-Commander **extfs** script. Both are configured in the **`rc.ext`**
file (Midnight Commander's `mc.ext` format), created with a few examples on first
run.

**Useful for** stepping into formats the built-in browser doesn't cover
(`.iso`, `.rpm`, `.deb`, `.lha`, …), and for wiring **Enter** / **F3** / **F4**
on a file type to your own commands.

**extfs — browse via scripts.** An `Open` rule of the form
`Open=%cd %p/<prefix>://` mounts the file with the extfs script named `<prefix>`
and shows its contents like a folder. Rat Commander runs the **same scripts as
Midnight Commander**, looked up in `~/.local/share/mc/extfs.d`,
`/usr/lib/mc/extfs.d` (and the other MC directories) plus your own
`~/.config/rat-commander/extfs.d`. So with MC installed, `.iso` (`iso9660`),
`.rpm` (`rpm`), `.deb` (`deb`) and the rest work out of the box. Inside a mount
you browse, copy **out** with **F5** (extract) or **in** with **F5** (add),
**F8** to delete and **F7** to make a directory — whatever that script supports;
an unsupported action reports a clear error. **..** at the top steps back out to
the file. (extfs scripts are shell/Perl/Python programs, so this is a Unix
feature.)

**Open / View / Edit commands.** A rule can also just run a command: `Open` on
**Enter**, `View` on **F3**, `Edit` on **F4**. A `View` beginning with
`%view{ascii}` (or `%view{hex}`) pipes the command's **output** into the built-in
viewer — e.g. `View=%view{ascii} unzip -v %f` shows the archive's contents
listing; a plain `Open` / `View` / `Edit` command runs in the foreground. When a
rule matches a file, its `View` / `Edit` takes precedence over the built-in
viewer / editor for that type.

Native archive browsing (the formats above) takes precedence over an extfs
`Open` rule, so `.zip` still opens with the fast built-in handler while `rc.ext`
covers everything else. The file format is detailed under
*Configuration → The rc.ext file format*.


## Remote filesystems (SFTP / FTP / SCP)

Mounts a remote server into a panel, so you browse and transfer
files over **SFTP** or **SCP** (SSH) or **FTP / FTPS** exactly like local files.

**Useful for** managing files on a server without a separate client — copy/move/
delete works transparently between local, remote and archive panels.

**Connecting.** Open the **Drive / connection picker** with **Alt-F1** (left
panel) or **Alt-F2** (right panel), or pick a protocol from the panel's
**Left** / **Right** menu. Enter host, port, user, password and an optional
remote path. Previously used servers are remembered (passwords are **not**
stored): open the **history dropdown** with the **▼** on the Host field, or by
pressing **↓** while the Host field is focused, to refill the form.

**FTP** connections have a **Passive mode (PASV)** checkbox (on by default): in
passive mode the client opens the data connection, which is what works behind
most NAT/firewalls; untick it for **active** mode, where the server connects
back. The choice is remembered per server. (SFTP and SCP tunnel their data over
the single SSH connection, so they have no such option.)

SSH host keys are checked against `~/.ssh/known_hosts` (trust-on-first-use; a
changed key is rejected).

**Connections behave like drives.** Every open connection stays alive as a
button in the picker, so you can switch a panel between **Local** and any server
at will — like drive letters. The **Local** button returns a panel to the local
filesystem *without* closing the connection (it even restores the local
directory you were last in); the connection is only closed by its own **✕
Disconnect** button, which asks for confirmation first. Several servers can be
open at once, and each remembers the directory you were last browsing on it. The
open connections (and a disconnect entry for each) also appear in the **Left** /
**Right** panel menus.

To keep things simple, **one panel is always local**: while one panel is on a
remote connection, the other panel's picker offers only Local and drive letters.
Return the remote panel to Local first to open a connection on the other side.
This avoids server-to-server transfers.

**Pulling a file down.** When the destination panel is remote, the copy/move
dialog prefills a `scheme://path` target (e.g. `scp-0:///home/user`). **Delete
the `scheme://` prefix** to redirect the copy to a **local** path instead — handy
for grabbing a file to disk while the remote connection stays open.


## The command line and subshell

The line at the bottom runs shell commands in the active panel's directory:
type a command and press **Enter**. The one special case is **`cd <dir>`**, which
changes the *active panel* (so the change sticks, unlike `cd` in a subshell);
`~`, `..`, and absolute or relative paths are supported.

For interactive work, **Ctrl-O** drops to a **full-screen persistent subshell**
in the current directory; press **Ctrl-O** again to return to the panels with
your shell session still alive.


## The user menu (F2)

**F2** opens a configurable **user menu** of shell commands. It is created with
sensible defaults on first run and uses the Midnight Commander `menu` file
format (see *Configuration* below). Each entry can run commands against the
current file, the current directory or the tagged files via macros. Useful for
one-key access to your own scripts and recurring tasks.


## Disk explorer

*Command menu → Disk explorer…*

Draws a full-screen **treemap** of the current directory: each
box's area is proportional to a subdirectory's total on-disk size, labeled with
the name and a human-readable size.

**Useful for** finding what is using your disk space.

**Operation.** Boxes that are large enough also show their **biggest files**
inside, each with its size, so you can spot space hogs without diving in. On a
terminal with graphics support (see *Terminal graphics*), the **whole treemap is
drawn as pixel "pillow" boxes**: each directory is a softly cushion-shaded box
**in its own hue**, subdivided into recessed, semi-transparent **sub-boxes** for
its largest files (sized by their share, with names labeled where they fit), so
every box reads as a little map of its own contents and much finer detail is
visible than with characters. It falls back to character-cell boxes on a plain terminal. The top
bar always shows the selected box's name, size and share of the total. **Arrow
keys** move the selection, **Enter** dives into a subdirectory, **Backspace**
goes up, **`g`** (or **Ctrl-Enter**) exits and points the active panel at the
selected directory, **Esc** closes. With the **mouse**, click a box to select it
and **double-click** to dive into it. Symlinks are never followed or counted.


## Process explorer

*Command menu → Process explorer…*

A full-screen, btop-style system monitor with a process table
and live graphs. It works on **Linux, Windows and macOS**.

**Useful for** seeing what's running and what's using the CPU, memory, disk and
network — and killing a runaway process.

**Operation.** The table has two layouts, toggled with **`Tab`**. **Flat** (the
default) is a single sortable list with **Pid**, **Program**, **Command**,
**Threads**, **User** and **MemB** columns. **Tree** shows the parent/child
process hierarchy, **fully unfolded** by default, with branch lines and a fold
box on each parent: **`[-]`** when open, **`[+]`** when folded; press **`→`**
(or **`Enter`**/**`Space`**) to unfold a subtree and **`←`** to fold it (or, on an
already-folded row, to jump to the parent), and **`*`** to fold/unfold the whole
tree at once. Individual threads are collapsed into their process's **Threads**
count rather than listed separately. Each row also shows CPU%, memory and a
per-process CPU sparkline; sort by **program, CPU, memory, threads, user or PID**
— in the tree, the sort orders each set of siblings while children stay grouped
under their parent. The layout adds a CPU-load line
graph and per-core meters, a memory sparkline, and two **centre-line graphs**
that split a metric into its two directions around a drawn **horizontal axis
line**: the **Disk** panel grows **writes upward (▲)** and **reads downward
(▼)**, and the **Net** panel grows **uploads upward (▲)** and **downloads
downward (▼)**, each direction scaled to their shared peak. **`+`/`-`** adjust the refresh interval.
**`k`** kills the selected process, **`K`** force-kills it; both ask to confirm.

A couple of details are platform-specific: on **Unix**, `k`/`K` send SIGTERM
/SIGKILL (graceful vs. forced), while on **Windows** both terminate the process
outright; the **battery** readout and per-process **thread counts** are shown on
Linux and read as unavailable on other platforms.


## Disk manager (Linux)

*Command menu → Disk manager…*

A two-pane manager of block devices and mounts: a
**disk → partition tree** on the left (each partition shows its filesystem type
and volume label) and the **current mounts** on the right. **Tab** switches panes.

**Useful for** mounting, unmounting, formatting and syncing drives without
leaving the file manager.

**Operation.** **Enter** (or double-click) a device for an action menu —
**Mount** / **Format** / **Flash image** / **Create image** when it's free, or
**Unmount** / **Flash image** / **Create image** when mounted. **Enter** on a
mount offers **Unmount** / **Sync**. Mounting prompts for a path (offering to
create it if missing); unmounting asks to confirm, and unmounting an **essential
system mount point** (`/`, `/boot`, …) raises a warning.

**Format** writes a fresh **FAT32, FAT16, VFAT, NTFS, EXT4/3/2 or BTRFS**
filesystem, with a volume label and filesystem-specific options (quick format,
bytes-per-inode), behind a destructive-action confirmation.

Privileged operations need root: when not run as root they use **`sudo`** —
non-interactively where possible, otherwise prompting for a password. Passwords
are never stored.


## Network connections (Linux)

*Command menu → Network connections…*

A full-screen view of the machine's sockets, split into two lists: **Listening
ports** (every open port with its owning program and **service name**) on top,
and active **Connections** below — each with its **type**
(`tcp`/`tcp6`/`udp`/`udp6`), state, local and peer address (with the peer's
service), program, the **incoming/outgoing traffic** it has carried (cumulative
bytes), the **live in/out rate** (bytes/sec), and a **per-connection rate
sparkline** of its recent throughput. The header shows totals and the current
overall down/up rate.

**Useful for** seeing what is listening on the machine, what it's talking to,
which programs are moving the most data, and spotting a busy or unexpected
connection at a glance.

**Operation.** On opening it asks for a **root password**. Enter one to see
*every* socket's owning program (full visibility); leave it **blank** to run in
**user mode**, where the connection lists are still complete but a program name
is shown only for your own sockets.

- **Tab** cycles the three views: **Listening ports → Connections → Overview
  diagram**. In the two lists, **←→** switch the focused list and **↑↓ /
  PgUp/PgDn / Home/End** (and the mouse wheel) scroll it.
- **`/`** starts a live **filter** — type to narrow the lists by program,
  address, port, state or service; **Enter** keeps it, **Esc** clears it. The
  filter also reshapes the overview diagram.
- **`s`** cycles the focused list's **sort** column, **`S`** reverses it.
- **`p`** cycles the protocol filter (all → tcp → udp), **`e`** toggles
  established-only, **`h`** toggles hiding loopback sockets.
- **Enter** opens a **details** popup for the selected socket (full command line,
  user, cumulative + live traffic, a rate graph, and the raw `ss` counters);
  any key closes it.
- **`k`** terminates the selected socket's owning process (SIGTERM), **`K`**
  force-kills it (SIGKILL) — both ask to confirm.
- **`r`** refreshes now, **`+`/`-`** change the auto-refresh interval, **Esc**
  closes the view.

**Overview diagram.** The third view (reach it with **Tab**) arranges the active
connections into a **responsive grid of service cards** — one card per service,
titled by its `proto :port name`. Each card lists the IP addresses talking to
that service, with a **◀** for **inbound** peers (someone connected to a port you
listen on) and a **▶** for **outbound** ones (you connected out). Colour encodes
the protocol: **cyan = TCP**, **green = UDP**, **yellow = both**. The diagram is
drawn with true terminal graphics when available, and with box-drawing characters
otherwise.

- **↑↓←→** move the cursor between IP addresses (nearest in that direction);
  **Home/End** jump to the first/last; **PgUp/PgDn** and the mouse wheel scroll.
- **Enter** or a **mouse click** on an address opens an **IP details** popup —
  direction, service, owning program(s), socket count, cumulative and live
  traffic, and a **reverse-DNS** hostname (resolved in the background via the
  system resolver; shows *resolving…* until it arrives, then caches the result).
- **`k` / `K`** act on the selected address's owning process, exactly as in the
  lists.

Data comes from `ss` (iproute2); the tool is offered only on Linux. The root
password, if given, is held in memory for the session so periodic refreshes can
re-run `sudo` without re-prompting, and is discarded when the view closes.


## Flash and image a disk (Linux)

**Flash an image to a disk.** Press **Enter** on a raw image file (`.iso`,
`.img`, `.raw`, `.bin`, `.dd`, …) to open a **target picker** listing every block
device and partition with its name, vendor/model, serial, label, filesystem and
size. Devices too small for the image can't be selected. Choosing a target asks
to confirm; a **non-removable** (fixed/system) disk raises an extra warning
first. The same flow is reachable from the disk manager's **Flash image** action,
which opens a small file browser to pick the image.

**Create an image of a device.** In the disk manager, the **Create image** action
on a device or partition opens a save browser to choose a directory and file
name (defaulting to `<device>.img`), then streams the device out to that file.


## Windows: drive letters

On Windows the **Drive / connection picker** (**Alt-F1** / **Alt-F2**, or the
panel menu's **Drive…** entry) shows the available **drive letters** on its first
row, with the current drive highlighted. Use the arrow keys or press a
drive-letter key to switch the panel to that drive. The **Local** button, any
open remote connections and the SFTP / FTP / SCP buttons appear below.


## Configuration

Configuration files live in your platform config directory
(`~/.config/rat-commander/` on Linux):

- **`config.toml`** — written by the Settings dialog. Holds the active theme and
  language, the truecolor / animation / status-widget toggles, the external
  editor and viewer commands, the confirmation flags, and the remembered remote
  servers (without passwords). It also holds `command_history_max` (default
  `100`) — the maximum number of command-line entries kept in the persistent
  history; set it to `0` to disable history.
- **`history`** — the persistent command-line history, one command per line
  (recalled with `Alt-P` / `Alt-N` / `Alt-H`), trimmed to `command_history_max`.
- **`editor-positions.toml`** — the editor's cursor-position memory for the last
  50 files edited (see *Editor*).
- **`themes.toml`** — your editable themes (see *Themes*).
- **`lang/`** — the localization files, one TOML per language (see *Language*).
- **`menu`** — the F2 user menu (see below).
- **`rc.ext`** — file associations for Open/View/Edit actions and extfs mounts
  (see *The rc.ext file format*).

### Settings (Options → Settings…)

Choose the **theme** and **language**, toggle **truecolor**, **animations**, the
**system-status widget** and **Reshape RTL text** (see *Language*), pick the
**Graphics** mode (see *Terminal graphics* below), set an **external editor /
viewer** command (used instead of the built-in ones), and choose whether to use
the internal viewer/editor.

The **Theme**, **Language** and **Graphics** fields are dropdowns: press
**Enter** to open the scrollable list, **↑/↓** (or the mouse wheel) to move
through it, **Enter** to pick, **Esc** to close. They **preview live** as you
move the highlight — the UI re-colors / re-translates / re-draws immediately — so
**Enter** keeps the highlighted one and **Esc** (closing the dialog) reverts to
what you started with. In every dialog the **OK** and **Cancel** buttons are part
of the keyboard focus ring: **Tab** / **↑↓** move onto them and **Enter** or
**Space** activates the highlighted one (**Enter** still submits from a field and
**Esc** always cancels).

### Terminal graphics

Where the terminal supports a graphics protocol, the **progress bars**, the
**process-explorer graphs** (CPU, per-core, memory, disk and network), the
file-transfer **speed graph**, the **disk-explorer treemap** and the **dialog
buttons** (OK, Cancel, Yes/No, …) are drawn as true-pixel images with smooth
gradients instead of block characters. Buttons pick up the theme's button colors
and gain a drop shadow, with a soft glow around the focused one; their labels are
drawn with an anti-aliased font (Latin, Cyrillic and Greek). A button whose
translated label is in a script that font can't draw (e.g. Arabic or CJK) simply
falls back to a regular text button so it stays readable. It uses the
**Kitty**, **Sixel** or
**iTerm2** protocol — so Kitty, Ghostty, WezTerm, Konsole, foot, recent
xterm/VTE, iTerm2 and similar all get the richer rendering — and falls back
automatically to the classic cell rendering everywhere else, so nothing is lost
on a plain terminal.

The **Graphics** setting controls this: **Auto** (default — use pixel graphics if
the terminal supports them, else cells), **Off** (always use cells), or a forced
**Kitty** / **Sixel** / **iTerm2**. Turn it **Off** if your terminal mis-renders
the images. The setting previews live and reverts on **Esc**, exactly like the
theme. (In `config.toml` the key is `graphics = "auto"`.)

### Language

The UI language is chosen in Settings and applied immediately. Translations live
in the **`lang/`** directory of the config folder, one file per language.
**18 languages** are written there on first run — English, German, French,
Spanish, Portuguese, Dutch, Czech, Slovak, Hungarian, Serbian, Ukrainian,
Russian, Japanese, Chinese (traditional and simplified), Hindi, Persian and
Arabic. Each file starts with a `name` (what the language is called in the
chooser) and a `[strings]` table mapping the English source text to its
translation — any missing entry falls back to English, so a partial translation
still works. To **add a language**, copy an existing file (e.g. `en.toml`) to a
new name, change its `name`/`code`, translate the values, and it appears in the
Settings chooser automatically. In menu labels the `&` marks the keyboard-
accelerator letter (the non-Latin catalogs put it in a trailing `(&X)` so the
accelerators stay typeable).

**Right-to-left scripts** (Arabic, Persian) are handled with the **Reshape RTL
text** setting, on by default. When on, RTL text is Arabic-shaped (letters are
mapped to their joined presentation forms) and bidi-reordered into visual order
just before it is drawn, so it reads correctly on a terminal that has no bidi
support of its own — most terminals. The accelerator underline is dropped in
this mode (reshaping moves the marked letter), but the accelerator **key** still
works. If your terminal already does its own bidi (mlterm, a recent VTE-based
terminal, Konsole), turn **Reshape RTL text** off so the text isn't processed
twice. The setting has no effect for left-to-right languages.

### Confirmations (Options → Confirmations…)

Toggle which actions ask first: **delete** (on), **overwrite** (on), **execute /
open with default app** (off), **unmount** (on) and **exit** (on).

### Themes (Options → Edit themes…)

Rat Commander ships many themes — Dracula, Nord, Gruvbox, Solarized, Tokyo
Night, Catppuccin, One Dark and more — plus a classic Midnight Commander look,
Monochrome, Amber/Green CRT, and some playful ones. On a truecolor terminal the
bars and cursor render as animated gradients.

**Options → Edit themes…** opens a **visual theme editor**. It starts on the
theme in use; pick any UI element from the color list and set its color with the
RGB **color picker** (a 16-color swatch grid on non-truecolor terminals), while a
**live preview** on the right shows whichever surface that element affects — the
file panels, a demo dialog, or a small editor. **Save** writes the change
(applying it at once when you are editing the active theme), **Save as…** stores
it under a new name that then appears in the theme chooser, and **Cancel** / `Esc`
leaves — prompting to save, discard, or cancel if there are unsaved edits (as
does switching the picker to another theme). The full key list is under
[Theme editor](#theme-editor) above.

Themes are stored in **`themes.toml`**, generated with all the presets on first
run. Each `[[theme]]` holds an explicit `#rrggbb` color for **every UI element** —
`panel_bg`, `menu_bg`, `dialog_bg`, `dialog_border_fg` / `dialog_border_bg`,
`input_bg` / `input_fg`, `cursor_bg` / `cursor_fg`, `menu_selection_bg` /
`menu_selection_fg`, the file-type colors, the gradient endpoints, and so on. You
can also edit the file directly — open it with **F4** in a panel, and saving
live-reloads it. Delete the file to regenerate the presets.

### The F2 user-menu format

The `menu` file uses the Midnight Commander format. A line starting in column 0
is a menu entry whose first character is its hotkey; the indented lines below it
are the shell commands to run:

```
# a comment
3      Compress the current subdirectory (tar.gz)
        Pwd=`basename "%d"`
        tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"
```

Macros expand before the command runs: `%f` / `%p` the current file, `%d` the
current directory, `%t` the tagged files, `%s` the tagged-or-current files, and
`%%` a literal percent. (Condition lines `+ …` / `= …` are accepted and ignored;
entries always show.)

### The rc.ext file format

The `rc.ext` file maps file names to actions, in Midnight Commander's `mc.ext`
format. A line starting in column 0 is a **matcher**; the indented `Key=Value`
lines below it are the **actions** for files it matches:

```
# zip
regex/\.(zip|ZIP)$
    Open=%cd %p/uzip://
    View=%view{ascii} unzip -v %f

# ISO9660 CD image
shell/i/.iso
    Open=%cd %p/iso9660://
```

**Matchers** (first match wins): `regex/PATTERN` matches the file name with a
regular expression (`regex/i/…` case-insensitive); `shell/.ext` matches a name
suffix and `shell/name` an exact name (`shell/i/…` case-insensitive). Other
Midnight Commander matcher kinds (`type/…`, `directory/…`) are recognised and
skipped.

**Actions.** `Open` runs on **Enter**, `View` on **F3**, `Edit` on **F4**.
`Open=%cd <path>/<prefix>://` mounts the file with the extfs script `<prefix>`
(see *File associations and extfs*); any other `Open` / `View` / `Edit` value is
a shell command run in the file's directory. A `View` value prefixed with
`%view{ascii}` or `%view{hex}` pipes the command's output into the built-in
viewer instead. `Icon=` and unknown keys are ignored.

**Macros** are the same as the user menu — `%f` / `%p` the current file, `%d` its
directory, `%s` / `%t` the tagged files, `%%` a literal percent — plus `%x` for
the file's extension.

Delete the file to regenerate the default examples.
