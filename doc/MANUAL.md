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
  remembered across sessions: cycle them with `Alt-P` / `Alt-N`, or press `Alt-Shift-H`
  to pick one from the **Shell History** window. `Alt-Enter` drops the name under
  the cursor onto the command line. `Ctrl-F5` hides it and gives the row to the
  panels, which also makes plain typing start a quick search — see *Working
  without the command prompt*.
- **Function-key bar** (bottom row) — shows what F1–F10 do in the current
  context. The labels also work as buttons: click one to run it.

Each panel's bottom border shows the volume's **free / total** disk space, and a
mini status line under the listing shows the full name of the highlighted file.

### The console (behind the panels)

Commands you run from the command line go to a **single persistent shell** —
the very same session `Ctrl-O` drops into — whose screen sits *behind* the
panels, exactly like Norton Commander. Because it's one lasting session, state
carries across commands and between the command line and `Ctrl-O`: a variable
you set, a directory you `cd` into, your shell history and aliases are all
shared. Commands run in the active panel's directory (the shell follows the
panel automatically).

Running a command does **not** interrupt the file manager — there's no "press a
key to continue". The output lands on the console behind the panels; to watch
it, hide a panel with `Ctrl-F1` / `Ctrl-F2`, shrink both with `Ctrl-F4`
(half-height), or press `Ctrl-O` to step into the shell full-screen (and again
to step back). Colours and cursor motion render faithfully, so it reads like the
real terminal it is. (Interactive programs and Ctrl-C act on the shell while
you're in it via `Ctrl-O`.)

**On a remote panel.** When the active panel is on an **SFTP or SCP** connection,
the command line and `Ctrl-O` run a shell on the **remote host** instead — over
the *same* SSH connection the file transfers use (no second login). It is its own
persistent session (one per open server, keyed to that connection), with its own
backdrop; the backdrop follows whichever shell you last used, local or remote.
Commands `cd` into the panel's remote directory first, so they run where you are
browsing. (FTP has no shell, so an FTP panel keeps using the local shell.)

> **Windows note.** The persistent behind-the-panels console is a Unix feature.
> On Windows the command line and `Ctrl-O` instead run **the classic way**: the
> panels are suspended while the shell runs (a command line runs once and waits
> for a key; `Ctrl-O` opens an interactive shell until you type `exit`), then the
> panels return. There is no live console backdrop, and shell state is not carried
> between separate runs — each command runs in the active panel's directory.


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
- A `*` in an **F6** target stands for the file's own name, so `*.bak` renames
  `notes.txt` to `notes.txt.bak` — and renames a whole selected set the same way,
  each file through its own name. For anything more involved than a suffix, use
  the multi-rename tool (**Shift-F6**) and its `[N]` / `[E]` placeholders —
  which also reach a photo's EXIF and an audio file's tags.

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
- Roll the **wheel** over a panel to move a whole page at a time, exactly like
  `PgUp`/`PgDn` — a page that would run past the listing lands on the first/last
  entry. Once you are at that end, where the page key does nothing, a further
  notch acts as `↑`/`↓` instead. The panel under the pointer scrolls; which panel
  is active doesn't change.
- Click the **`◀`** arrow (a panel's top-left corner) or the **`▶`** arrow (its
  top-right corner) to step that panel **back / forward** through its directory
  history.
- Click a **menu-bar title** to open it and an entry to run it.
- Click the bottom **F-key bar** to run that function.
- Click **OK / Cancel** (or **Yes / No**) buttons in dialogs, and the
  **Abort** button on a scan/search progress dialog.
- In any dialog with input fields — **Copy/Move**, **Make directory**,
  **Chmod**, **Chown**, **Checksum**, **Select/Unselect group**, the
  **FTP/SFTP/SCP connection** form, **Settings**, **Find file**, editor
  **Search/Replace**, and more — click a **text field** to focus it and place the
  caret, click a **checkbox** or **radio** to toggle it, click a **dropdown** to
  open and pick from it, click a **tab** to switch to it, and click **OK/Cancel**
  to finish.
- Click an entry in the **user menu** (F2) or the **shell-history** window to run
  / recall it.
- In the **disk explorer**, click a box to select it — or click one of the big
  files it lists to put the cursor on that file — and **double-click** to enter
  that subdirectory.


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
- `Shift-F4` — Edit a new file: asks for a name, then opens the editor on that
  file in the active panel's directory
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
- `Alt-Shift-H` — Open the **Shell History** window just above the command line: move
  with `↑`/`↓` (or `Alt-P`/`Alt-N`) and press `Enter` to copy the chosen command
  into the command line **without running it**; `Esc` closes it
- `Alt-S` / `Ctrl-S` — **Quick search** the active panel: opens an empty search
  box; each letter you type filters, jumping the cursor to the first file whose
  name starts with it (case-insensitive; `Shift` for uppercase works). The box
  stays open even when empty — `Backspace` trims it, and only `Esc` or an arrow
  key dismisses it. `Enter` opens the match. With the command prompt hidden
  (`Ctrl-F5`), simply typing a character starts it
- `Ctrl-O` — Step into the persistent shell full-screen and back (press again to
  return). It is the **same session** the command line runs in — see *The
  console* above
- `Ctrl-P` — Open the **Command palette** (fuzzy search over every action,
  setting, bookmark and open connection — see *The command palette* below)
- `Alt-←` / `Alt-→` (or Midnight-Commander's `Alt-y` / `Alt-u`) — Go **back** /
  **forward** through the directories the active panel has visited. The clickable
  `◀` (top-left) and `▶` (top-right) arrows on each panel's border do the same
  (and are dimmed when there is nowhere to go that way) — see *Directory history*
  below
- `Alt-H` — Open the **Directory History** window: the same history the arrows
  step through, as a pickable list — see *Directory history* below
- `Alt-I` — Point the **other panel** at this panel's directory, so both show the
  same place
- `Alt-O` — Show the directory under the cursor **on the other panel** and step
  down one entry (with the cursor on a file, the other panel gets *this*
  directory instead) — so holding `Alt-O` walks a listing while the other panel
  keeps pace
- `Alt-T` — Cycle the active panel's **view format** (full → brief → details →
  tree → 3D → thumbnails)
- `Ctrl-\` — Open the **directory hotlist** (your bookmarked directories): jump
  to one, add the current directory, or remove one — see *The directory hotlist*
  below
- `Alt-Shift-I` — Set or clear the active panel's **persistent listing filter** (a
  shell glob like `*.rs`, or plain text; a blank entry clears it) — distinct from
  the quick-search cursor jump; see *The listing filter* below
- `Alt-G` — Open the **Git menu** (status, log, commit, fetch/pull/push, checkout,
  reset, init, clone …) — see *The Git menu* below
- `Ctrl-G` — **Stage / unstage** the file(s) under the cursor (git) — see
  *Git-aware panels* below
- `Alt-D` — Open a side-by-side **diff of the file against its committed (`HEAD`)
  version** (git) — see *Git-aware panels* below
- `Ctrl-Ins` — Copy the **selected paths** (or, with nothing marked, the cursor's)
  to the **system clipboard** — see *The system clipboard* below
- `Ctrl-R` — Re-read (refresh) the active panel — usually unnecessary now, see
  *Auto-refreshing panels* below
- `Ctrl-E` — Toggle reverse sort order (choose the sort key from the panel menu)
- `Alt-T` — Cycle the view format (full → brief → details → tree → 3D → thumbnails)
- `Ctrl-X` — Toggle vertical / horizontal split
- `Ctrl-U` — Swap the two panels
- `Ctrl-F1` / `Ctrl-F2` — Hide (and show again) the left / right panel,
  Norton-Commander style. Each panel keeps its own half; hiding one exposes the
  **console** underneath (see below), and both may be hidden at once. The menu
  bar and F-key bar always stay on screen
- `Ctrl-F4` — Toggle **half-height** panels: both panels shrink to the top half
  of the screen, exposing the console beneath them
- `Ctrl-F5` — Show / hide the **command prompt** below the panels. With it hidden
  the panels take over its row, and typing any printable character starts a quick
  search instead of entering text — see *Working without the command prompt* below
- `Alt-F1` / `Alt-F2` — Drive / volume / connection picker for the left / right panel
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
On the **command line only**, `Ctrl-E` and `Alt-F` keep their panel meaning
(reverse sort / File menu) while the line is empty, and switch to editing as soon
as it has text. `Ctrl-W` is always the editing key — the listing-type toggle it
used to share now lives on `Alt-T`.

### Viewer (F3)

- `F1` — Help (opens this manual)
- `F2` — Toggle line wrap (in an **audio view**: switch Spectrogram / Waveform;
  in a **table**: turn the header row on or off)
- `F4` — Cycle text / hex / byte-map mode (and the binary view, for an
  executable or library); in a **document view**, show the file's bytes in hex
- `F5` — Goto (line / percent / byte offset)
- `F6` — (Markdown files) show the document outline — a tree of the headings.
  Use `↑ ↓` / `PgUp PgDn` / `Home End` or the mouse to pick a heading, `Enter`
  (or a click) to jump to it, `Esc` / `F6` to dismiss. Opening this manual with
  `F1` lands on its outline. In a **document view**, the document's outline (its
  headings, slides or pages); for a spreadsheet, the next sheet (`Shift-F6` the
  previous one)
- `F7` — Search
- `F8` — (Markdown files) toggle Raw / Render; (CSV / TSV files) toggle Table /
  Raw; (image files) toggle Image / Raw;
  (model files) toggle Model / Raw; (audio files) toggle Audio / Raw; (byte map)
  toggle Density / Bytes colouring; (binary view) toggle demangled / raw symbol
  names; (certificate and key files) toggle Certs / Raw; (documents and
  spreadsheets) toggle Document / Raw
- In an **audio view**: `Space` play / pause, `s` stop, `← →` seek 5 s,
  `PgUp PgDn` seek 30 s, `Home End` jump to the start / end, `+ -` or `↑ ↓`
  change the volume; click or drag on the picture to seek
- `n` — Repeat the last search
- `f` — Follow the file as it grows (`tail -f`); `f` again stops
- `b` — Git blame: show who last changed each line; `↑ ↓` move a line cursor,
  `Enter` opens that line's commit in the panel, `b` again hides the column
- In a **table**: `← → ↑ ↓` / `PgUp PgDn` move the cell cursor, `Home End` go to
  the first / last field of the record, `Ctrl-Home Ctrl-End` to the first / last
  record, `Tab` / `Shift-Tab` step through the cells, `<` `>` (or `Ctrl-← →`)
  narrow and widen the column; a click picks a cell
- In a **spreadsheet**: `]` / `[` (or `Ctrl-PgDn` / `Ctrl-PgUp`, or `F6` /
  `Shift-F6`) switch to the next / previous sheet; each keeps its own cursor
- In the **binary view**: `Tab` / `Shift-Tab` or `1`–`7` switch lists, `Enter`
  opens the hex view at the highlighted row, `Esc` drops a *Find all* filter
- In the **certificate view**: `Tab` / `Shift-Tab` or `1`–`6` switch tabs,
  `Enter` goes to the highlighted row's line in the raw text, `Esc` drops a
  *Find all* filter
- `↑ ↓` / `PgUp PgDn` / `Home End` — Scroll (in a **model view**: `← → ↑ ↓`
  orbit the camera, `+` / `-` zoom, `Home` re-frames; dragging orbits and the
  wheel zooms)
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
- `Shift-F5` — Insert a file at the cursor
- `Shift-F7` — Search again (repeat the last search)
- `Ctrl-C` / `Ctrl-X` / `Ctrl-V` — Copy / cut the block to the clipboard, paste
- `Ctrl-Z` / `Ctrl-Y` — Undo / redo
- `Ctrl-A` — Mark the whole file
- `Ins` — Toggle insert / overwrite (the status line shows `OVR`)
- `Shift+arrows` (or `Shift+Ctrl-arrows`) — Mark text while moving
- `Ctrl-Home` / `Ctrl-End` — Start / end of the document
- `Ctrl-← / →` — Move by word
- `Ctrl-N` — Start a new, unnamed buffer
- `Ctrl-F` — Write the block (or the whole file) to another file
- `Ctrl-S` — Toggle syntax highlighting
- `Ctrl-L` — Repaint the screen
- `Alt-L` — Go to a line number
- `Alt-B` — Jump to the bracket matching the one at the cursor
- `Alt-P` — Format (re-wrap) the paragraph around the cursor
- `Alt-T` — Sort the marked block's lines
- `Alt-U` — Run a command and paste its output at the cursor
- `Alt-K` / `Alt-J` / `Alt-I` / `Alt-O` — Bookmark: toggle, next, previous, flush
- `F9` — Pulldown menu (see *The editor menu* below)
- `Shift-F9` — Toggle word wrap
- `Ctrl-F9` — Toggle the in-place hex editor
- `Alt-G` — Toggle the spreadsheet grid (see *Spreadsheet grid* below)
- `Alt-T` / `F3` — (audio files) Toggle the tag page and the file's bytes (see *Audio tags* below)
- `Alt-E` / `Alt-Shift-E` — (JSON, TOML, YAML and XML files) Jump to the next /
  previous syntax error
- `Alt-F` — (JSON files) Pretty-print the document
- `Alt-M` — Show and edit the GeoJSON in the file on a world map (see *GeoJSON map* below)
  — including a shapefile opened with F4 (see *Shapefiles* below)
- `Esc` / `F10` — Quit (prompts if modified)

While **Shift** or **Ctrl** is held, the F-key bar relabels the keys those
modifiers reach — **F2 → Save as**, and with Shift also **F5 → InsFil**,
**F7 → Again** and **F9 → Wrap** (Ctrl shows **F9 → Hex**).

The editor remembers where you left the cursor in each of the last **50** local
files (in `editor-positions.toml`); re-opening a file restores the cursor and
scrolls so it sits in the vertical center of the view. Turn *Save file position*
off in the editor options to stop it.

### The editor menu (F9)

**F9** opens an `mcedit`-style pulldown over the editor's status row. Its six
menus are **File**, **Edit**, **Search**, **Command**, **Format** and
**Options**; ← → step between them, ↑ ↓ move within one, `Enter` activates the
highlighted item, and a menu's underlined letter jumps straight to it. `Esc`,
`F9` or `F10` close the menu again, and the mouse works throughout. "File" and
"Format" share the letter `f`, so pressing it repeatedly steps between the two.

Every item is also reachable by its own shortcut (listed above and in the menu
itself). Only actions this editor can carry out are listed — mcedit entries that
would need a tags database, a macro recorder, a spell checker or a window
manager are absent rather than present-but-dead. In **hex mode** the items that
work on the text buffer are greyed out; saving, quitting and switching back to
text mode stay available.

Beyond the F-key actions, the menus offer:

- **File** — Open another file, start a new buffer, insert a file at the cursor,
  write the block to a file, and an About box.
- **Edit** — Mark all / unmark, the clipboard operations, and jumps to the start
  and end of the file.
- **Search** — Search, search again, replace, and the four **line bookmark**
  actions. A bookmarked line's text is drawn in the "marked" colour, and
  `Alt-J` / `Alt-I` step through the bookmarks in order, wrapping around. In a
  JSON file, **Next error** and **Previous error** step through its syntax
  errors.
- **Command** — Go to line, jump to the matching bracket, and the syntax /
  word-wrap / hex-mode / spreadsheet toggles, the **GeoJSON map**, **Decode JWT
  at cursor** (the JSON Web Token under the cursor — in a header, a log line, a
  config file — shown with its header and claims pretty-printed, `iat` / `nbf` /
  `exp` read as dates with how long until or since it expires; the signature is
  not checked), plus a screen repaint. In hex mode, **Binary templates** opens a submenu to choose a
  template, run it again, jump to the variable under the cursor, edit the
  template, start a new one, or stop using one, and **Data inspector** shows or
  hides the inspector.
- **Format** — Insert the date and time, re-wrap the current paragraph to the
  configured line length, sort the marked block's lines (with reverse,
  ignore-case and remove-duplicates options), and paste a shell command's output.
  In a JSON file, **JSON** opens a submenu to pretty-print, minify or sort the
  keys of the document.
  In the **spreadsheet grid**, insert and delete rows and columns, and choose
  whether the first row is the header; these are greyed out elsewhere, and the
  items that work on lines and marked blocks are greyed out in the grid.
- **Options** — **General…** opens the editor options dialog below; **Save
  setup** writes the current options to `config.toml` as the new defaults.

#### Editor options (Options → General)

The options are stored in `config.toml` under `[editor_options]`, so they
persist across runs and apply to every file opened afterwards.

- **Wrap mode** — `None` (long lines scroll sideways), `Dynamic paragraphing`
  (long lines are *shown* across several rows; the file is unchanged) or
  `Type writer wrap` (typing past the wrap column breaks the line for real).
- **Tabulation** — *Backspace through tabs* makes one Backspace inside a line's
  indentation remove a whole indent step; *Fill tabs with spaces* decides
  whether Tab types spaces or a tab character; *Tab spacing* is how wide one
  step is (Tab advances to the next multiple of it).
- **Return does autoindent** — Enter copies the current line's leading
  whitespace to the new line.
- **Confirm before saving** — whether F2 asks first.
- **Save file position** — remember the cursor position per file (see below).
- **Visible trailing spaces** / **Visible tabs** — mark whitespace that would
  otherwise be invisible.
- **Syntax highlighting** — the same toggle as `Ctrl-S`, but remembered.
- **Cursor after inserted block** — where the cursor lands after F5 or a paste.
- **Persistent selection** — whether a plain cursor move keeps the marked block
  (on) or drops it (off, the behaviour of most GUI editors).
- **Group undo** — undo a run of typing in one step instead of per character.
- **Word wrap line length** — the column *Format paragraph* and typewriter wrap
  break at.

### Spreadsheet grid (CSV / TSV files in the editor)

- `← → ↑ ↓` / `PgUp PgDn` — Move the cell cursor; `Home` / `End` — first / last
  field of the record; `Ctrl-Home` / `Ctrl-End` — first / last record
- `Tab` / `Shift-Tab` — Next / previous cell, in reading order
- `Enter` — Edit the cell (its value is kept); a typed character — edit the cell,
  replacing its value; `Backspace` — edit it from empty; `Delete` — clear it
- While editing: `Enter` writes the cell and moves down, `Tab` / `Shift-Tab` /
  `↑ ↓` write it and move, `Alt-Enter` puts a line break in the value, `Esc`
  throws the edit away; `← →` / `Home End` and the usual line-editing keys move
  within the value
- `F3` — First row is the header (on / off)
- `F5` / `F6` — Insert a row above / a column left of the cursor
- `F8` / `Shift-F8` — Delete the cursor's row / column
- `Ctrl-C` / `Ctrl-X` / `Ctrl-V` — Copy / cut / paste the cell's value
- `Ctrl-← / →` — Narrow / widen the column
- `F2`, `F4`, `F7`, `Ctrl-Z` / `Ctrl-Y`, `F9`, `Esc` / `F10` — as in the text
- `Alt-G` — Back to the text, with the cursor on the cell

### Hex editor (Ctrl-F9 in the editor)

- `0`–`9`, `a`–`f` — Overwrite the current byte's nibble (hex column)
- typed character — Overwrite the current byte (ASCII column)
- `Tab` / `Shift-Tab` — Step round the hex and ASCII columns, the data
  inspector when it shows, and the template tree when a template is shown
  (entered as with `F6`)
- `← ↑ ↓ →` / `PgUp PgDn` — Move; `Home` / `End` — start / end of row
- `Ctrl-Home` / `Ctrl-End` — Start / end of file
- `F7` — Search (hex bytes like `48 65` or text)
- `F4` — Replace all (same length, overwrite-only)
- `F2` — Save the changed bytes in place
- `F5` / `Shift-F5` — Choose the binary template / run it again
- `F6` — The template variable under the cursor, in the template tree
- `F3` — The template's output / its variables
- `F8` — Show / hide the data inspector
- `Ctrl-F9` — Back to text mode
- `F9` — Pulldown menu (text-only items greyed out)
- `Esc` / `F10` — Quit (prompts if modified)

In the template tree (see *Binary templates*):

- `↑ ↓` / `PgUp PgDn` / `Home End` — Move; the byte cursor follows to the
  variable
- `→` / `+` — Open; `←` / `-` — Close, or go to the parent; `*` — Open
  everything below
- `Enter` — Open or close a struct or array, or edit a value (`Enter` writes it,
  `Esc` drops it)
- `F6` / `Esc` — Back to the bytes, at the selected variable; `Tab` /
  `Shift-Tab` — the same, on to the hex column / the inspector

In the data inspector (see *Data inspector*):

- `↑ ↓` / `PgUp PgDn` / `Home End` — Choose a type; its bytes are marked
- `← →` — Move the byte cursor; `Ctrl-←` / `Ctrl-→` — by the chosen value's width
- `Enter` — Edit the value (`Enter` writes it, `Esc` drops it)
- `b` — Switch between little- and big-endian
- `Esc` — Back to the bytes; `F6` — to the template tree

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
  recolor; `Enter` / `→` jump to the color picker. Under most colors sits an
  indented **Gradient** row for that element's ramp (see below).
- **Color picker** (truecolor) — `↑ ↓` pick the R / G / B channel; `← →` adjust
  it by 1, `Shift-←` / `Shift-→` by 20, `PgUp` / `PgDn` by 16, `Home` / `End`
  set it to 0 / 255; `Enter` returns to the list. On a 16-color terminal the
  picker is a swatch grid moved through with the arrows.
- **Type a hex code** — from either the color list or the picker, type a
  six-digit hex code (e.g. `1a2b3c`) to set the selected color directly;
  `Backspace` edits it and `Esc` cancels the entry.
- **Gradient rows** — on an indented **Gradient** row, `Space` switches the
  ramp on or off, `Ctrl-D` cycles its direction (horizontal → vertical →
  diagonal → radial) and `Ctrl-A` its animation. The color picker and hex entry
  then set the ramp's **second** endpoint — the first is the element's own color
  on the row above. The row shows both endpoints, the direction (`↔ ↕ ↘ ◎`) and
  a `*` while it animates; the picker's title names the direction in full.
- **Buttons** — `← →` move between **Save**, **Save as…** and **Cancel**;
  `Enter` / `Space` activates.
- `F2` / `Ctrl-S` — Save and close. `Esc` / `F10` — Close (prompts to save,
  discard, or cancel if there are unsaved changes).
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

Each panel can show its listing five ways; cycle them with **Alt-T** or pick
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

  Inside a **git work tree**, a **git activity calendar** follows: a year of the
  item's history laid out like a contribution graph — a column per week, the
  current one on the right, a row per weekday from Monday — with each day
  shaded by how many commits touched the file, the directory, or the tagged
  items together, relative to their busiest day. The count for the whole year
  is written on its rule. It is counted with `git log` in the background once
  the cursor has rested on an item for a moment, so running down a listing
  starts nothing, and an item visited again comes straight from memory (a
  commit or pull through the Git menu counts afresh). A narrow panel shows the
  latest weeks that fit, a wide one squares the cells, and a panel too short to
  spare the rows leaves it out. Days are counted on the same UTC clock as every
  other time shown. *Details view: git activity* (Settings → Panels, or the
  command palette) turns it off, for a repository big enough that each `git log`
  is felt.

  Beneath the metadata, a **preview** of the item is shown (loaded in the
  background so large or remote items stay responsive):
  - a **text file** → a syntax-highlighted view of its first lines;
  - an **image** → a thumbnail — a true-pixel image where the terminal supports
    graphics (Kitty / Sixel / iTerm2), or half-block cell art otherwise —
    **centred** in the panel. An embedded EXIF thumbnail is used when present (so
    a full-resolution photo isn't decoded just to shrink it), and a short **EXIF
    summary** (camera, lens, date, exposure) is shown above it when the image
    carries one;
  - an **audio file** (local) → its format, length and tags, then its
    spectrogram or waveform with the progress and transport rows — the same
    controls as the viewer's *Audio view*, in a narrower form. **Click Play**
    and it plays while you carry on browsing: the file list keeps the focus.
    Click or drag on the picture to seek, and click − / + for the volume. With
    the Details panel itself active (`Tab`), `Space` plays and pauses, `← →` and
    `PgUp`/`PgDn` seek, `Home`/`End` jump to the ends and `+`/`-` or `↑`/`↓` set
    the volume (`Space` and `← →` go to the command line instead once something
    is typed there). Moving the cursor to another item stops it. A preview never
    starts playing by itself, whatever *Auto-play audio in the viewer* is set to. The file is
    decoded only once the cursor has rested on it for a moment, so running down
    a music folder decodes nothing on the way past;
  - an **archive** (`.zip`, `.tar.*`, `.7z`, …) → its top-level file list;
  - a **directory** → a shallow tree of its contents.

  This is useful for inspecting a file's metadata, glancing at its contents, or
  measuring how much space a folder or a set of tagged items uses, while you
  browse with the other panel.
- **Tree** — the directory structure is visualized as a tree, arrow keys navigate,
  pressing enter changes the opposite panel's directory and opens up the directory
  structure underneath.
- **3D** — a **tree of boxes joined by lines**, showing what is inside the
  directory the *other* panel is in, each box sized by what that directory holds.
  See *3D view* below.
- **Thumbnails** — the listing as a **grid of pictures**: images and 3D models
  (`.stl`, `.obj`) as thumbnails, everything else by its type. See *Thumbnails*
  below.
- **Activity log** — a live list of what is being created, written, removed and
  renamed anywhere under the *other* panel's directory. Chosen from the **Left** /
  **Right** menu (it is not in the `Alt-T` cycle). See *Activity log* below.

**Sorting** is configurable from the **Left** / **Right** menu; **Ctrl-E** toggles
reverse order. The keys are: Unsorted, Name, Extension, Size,
Modify / Access / Change time, or Inode — with reverse, case-sensitive and
executables-first toggles.

Filenames carry an `ls -F`-style **type marker** so kinds read by symbol, not
just color: `/` directory, `*` executable, `@` symlink, `!` broken symlink, and
a leading space for plain files (keeps names aligned). File **names are colored**
by type as well: archives, documents, images and audio/video each get a hue.

**Nerd Font symbols.** Turning on *Options → Settings → Appearance → Nerd Font symbols*
replaces those markers with a per-type icon — a folder, a chain link (broken for
a dangling one), and a glyph picked from the file itself: its language for source
files (Rust, Python, Go, …), the category for documents, images, audio, video,
archives, fonts, keys, databases and binaries, and a few whole names that carry
no extension (`Makefile`, `Dockerfile`, `LICENSE`, `.gitignore`, `Cargo.toml`, …).
Anything unrecognized gets a blank page, or a gear if it is executable. The glyph
is followed by a space, so names line up exactly as they do without it. This
needs a [Nerd Font](https://www.nerdfonts.com/) selected in your terminal —
preferably a **Mono** variant, whose icons are one cell wide — which is why it is
off by default; without one the listing shows replacement boxes.


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

**From the file itself.** A mask can also pull values out of the files being
renamed. These are read in the background when the dialog opens — the footer
says so while it is happening — and a file that has no such value contributes
nothing, so a mixed batch is fine. Values are cleaned up for use in a name
(path separators and control characters removed, runs of whitespace collapsed).

From a photo's EXIF:

- `[EXIF:YMD]` / `[EXIF:hms]` — the date and time the photo was taken
- `[EXIF:Y]`, `[EXIF:M]`, `[EXIF:D]`, `[EXIF:h]`, `[EXIF:m]`, `[EXIF:s]` — the
  same moment in separate parts, so you can write `[EXIF:Y]-[EXIF:M]/…`
- `[EXIF:Make]`, `[EXIF:Model]`, `[EXIF:Lens]` — the camera and lens
- `[EXIF:Exposure]`, `[EXIF:FNumber]`, `[EXIF:ISO]`, `[EXIF:FocalLength]`
- `[EXIF:Orientation]`, `[EXIF:Width]`, `[EXIF:Height]`
- `[EXIF:GPSLat]`, `[EXIF:GPSLon]` — signed decimal degrees

From an audio file's tags (MP3, Ogg, FLAC, M4A, WAV and the rest):

- `[TAG:Title]`, `[TAG:Artist]`, `[TAG:Album]`, `[TAG:AlbumArtist]`
- `[TAG:Track]`, `[TAG:Disc]` — single digits are zero-padded so a batch sorts
- `[TAG:Year]`, `[TAG:Genre]`, `[TAG:Comment]`, `[TAG:Composer]`

So `[EXIF:Y]-[EXIF:M]-[EXIF:D]_[C].[E]` numbers a shoot by date, and
`[TAG:Track] - [TAG:Artist] - [TAG:Title].[E]` names an album properly.

Press **F1** in the dialog for the full list; ↑↓ scroll it, Esc closes it.

Only local files are read. A file on a remote or inside an archive would have to
be fetched whole to reach its metadata, so those simply have none.

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

To build the same kind of listing from any command instead of a search, see
[Panelize a command's output](#panelize-a-commands-output).

**Options** include recursive search, case sensitivity, skip-hidden, and shell-
wildcard vs. regular-expression name matching. On a **remote** panel the search
matches **file names only** (content search is local).

**Content search.** Fill in **Content** to keep only the files that contain some
text. Tick **Content is a regular expression** to treat it as a regex instead of
literal text — the same regex flavour the viewer's and editor's F7 search uses;
**Case sensitive** applies to it as well as to the file name. Files are read in
**streaming windows** rather than loaded whole, so searching a tree full of large
files costs a fixed amount of memory, and files that look **binary** (a NUL byte
near the start) are skipped the way `grep` skips them. Pressing **F3** on a result
that matched on content opens the viewer **at the matching line**.


## Panelize a command's output

*Command menu → Panelize command output…*

Runs a shell command and puts the files its output names into
the active panel, as the same kind of flat listing [Find file](#find-file)
produces.

**Useful for** reaching a *set* of files that no directory holds — everything
`rg -l` matched, everything `git ls-files -m` changed, every file a package
owns — and then working on it with the ordinary keys.

**Usage:** Open the dialog and type a command. It runs through your shell, in
the active panel's directory, and each line of its output is taken as the path
of a file to list. So the command has to print **paths**, one per line —
`ls -la` prints a *listing* (permissions, owner, size, name) and names nothing
usable; plain `ls` does. When no line names a file that exists, the dialog says
so rather than opening an empty panel. The matches replace the panel listing; a `..` entry at the
top returns to normal browsing. **F3** views, **Insert** / `+` tags, and
**F5** / **F8** copy and delete, exactly as in a normal listing.

Some commands worth keeping:

| Command | Lists |
| --- | --- |
| `rg -l TODO` | every file containing *TODO* |
| `git ls-files -m` | every modified file in the work tree |
| `find . -mtime -1` | everything changed in the last day |
| `dpkg -L nginx` | every file the package owns |
| `pacman -Qlq foo` | the same, on Arch |

**Paths.** A relative path is resolved against the panel's directory, because
that is where the command ran; an absolute one is used as is. Nothing is
resolved through symlinks, so `find . -type l` lists the links themselves rather
than their targets, and a **broken symlink is still listed** — it is a file you
may well want to find and delete. Lines naming nothing reachable are dropped
silently; only when *nothing* is left does the command report that it produced
no usable paths.

**Names with newlines in them.** If the output contains a NUL byte anywhere, it
is split on NUL instead of on newlines — so `find -print0`, `rg -l --null` and
`git ls-files -z` all work with nothing to configure, and those are the forms
that survive a file name containing a line break. Plain `git ls-files` quotes
such names in C syntax and is **not** unquoted here; use `-z`.

**Limits.** The command must run in a **local** directory: the shell cannot be
pointed at a cwd inside an archive or on a remote, which is the same limit the
command line has. Unlike the command line, it runs your shell
**non-interactively**, so shell **aliases do not expand** — an interactive
shell takes the terminal away from the program that is drawing the panels, so
it cannot be used here. Functions and variables from `~/.zshenv`, `config.fish`
and the like still apply. Output is capped at 100 000 files. The command is remembered
for the next time the dialog opens, but is kept **separate from the shell
history** — it was never run at the prompt, so it does not come back on
**Alt-P**.


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

The comparison only *looks* at the top level of each directory and only tags
what it finds. To act on the differences — including inside subdirectories —
press **Synchronize…** in the same dialog (see below).


## Synchronize directories (mirror)

*Command menu → Synchronize directories…*, or the **Synchronize…** button in the
Compare-directories dialog.

Makes one directory tree match another, **recursively**. The active panel is
always the **source** and the other panel the **destination** — the same
direction as F5/F6 — and both dialogs spell the two paths out so there is no
doubt which way round it runs.

**Useful for** keeping a backup, a USB stick, or a server copy up to date:
*"mirror this folder to my SFTP server"* is this dialog.

**Remote and archive panels.** The sync runs over the same filesystem layer as
copy/move, so either side can be a remote connection — but what a backend can
report and store differs, and that changes what sync can do:

| Panel | Sync support |
| --- | --- |
| **Local** | Full — compares size + time, and stamps copies with the source's time. |
| **SFTP** | Full, exactly as local (SFTP reports *and* can set file times). |
| **FTP / SCP** | These report no file times, so files are compared **by size only** and each run re-copies nothing it doesn't have to. A same-size edit is not noticed. **Two-way is refused** — "newer wins" can't be decided without times. |
| **Archive** | May be the **source**, never the destination: an archive is rebuilt as a whole rather than written file by file, so syncing *into* one is refused up front. |

**Modes.**

- **One-way: copy new and changed files** — anything missing from or different on
  the destination is copied over. Nothing is ever removed. The safe default.
- **One-way mirror: also delete extraneous files** — as above, plus **anything
  the source doesn't have is deleted from the destination**, so it ends up an
  exact copy. This throws data away; the preview is flagged in red and opens on
  Cancel.
- **Two-way: newer file wins** — a file on only one side is copied to the other;
  a file on both sides is replaced by whichever copy is newer. Nothing is ever
  deleted. If the two have the same time but different contents, the **source**
  (active panel) wins rather than the choice being arbitrary.

**How files are compared.** By **size and modification time** — a file counts as
changed when either differs. Times within **two seconds** are treated as equal,
since filesystems disagree about resolution (FAT rounds to 2 s, some remotes
report whole seconds) and without that slack an untouched file would be copied on
every run. A backend that reports no timestamp at all falls back to size alone.

**The preview.** Nothing is touched until you approve it. Comparing the trees runs
in the background (it can take a moment over a slow link), then the plan is shown
in full:

- Each line is one step: `→ file` copies to the destination, `← file` copies back
  (two-way only), `✗ delete file` removes one, and `mkdir dir/` creates a
  directory. Deletions are red.
- A summary gives the totals — how many files to copy, how many bytes, how many
  to delete.
- Scroll with `↑`/`↓`/`PgUp`/`PgDn`/`Home`/`End` or the wheel; `Tab` moves between
  **Execute** and **Cancel**; `Esc` cancels. If the trees already match, it says
  so and offers only Cancel.

**Execution.** Execute hands the plan to the ordinary file-transfer engine, so a
sync behaves like any copy: a progress window with a speed chart, **Abort**, and
**To background** — which is what makes the "mirror to my server while I carry on
working" case practical. It shows up in *Background operations* like any other
transfer.

**Notes.**

- A copied file is stamped with its **source's** modification time, so a second
  run has nothing to do rather than re-copying the tree. (Local and SFTP
  destinations can do this; see the table above for the rest.)
- The order is deliberate: directories are created first, copies next, and
  extraneous deletions **last** — so a failed transfer never costs you data that
  was only about to be replaced.
- An extraneous *directory* is removed in one recursive step rather than listed
  file by file.
- If one side has a file where the other has a directory, only the deleting
  mirror mode resolves it (by removing and replacing). The other modes leave that
  path alone rather than guess.
- Search-result (panelized) listings can't be synchronized — they aren't
  directories.


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

**Binary files.** When either file is binary (it has a NUL byte among its
first 8 KiB) — or, for local files, too large to load as text — the two are
compared **byte by byte** instead, in a read-only view: offsets on the left,
then each file's hex and ASCII side by side (only the hex on a narrower
terminal, one file above the other on a narrow one), one cursor on both.
Every byte that differs is coloured, as is the offset of a row holding one; a
file shorter than the other shows blank cells past its end. Local files are
read from disk a page at a time, so files of any size can be compared; binary
files from an archive or a remote connection are read into memory and must be
under 64 MiB.

A scan in the background finds the **runs** of differences (differences less
than 16 bytes apart count as one run); the status line shows how many it has
found, which one the cursor is in, how far it has got, and the two sizes when
they differ. When a binary template fits the first file (see *Binary
templates*), it runs over that file in the background too, and the status line
names the field the cursor is in — `ZIP.bt: record.frCompression` — so a
difference reads as the field it changes. Bytes are compared at the same offset: a byte inserted in one file
moves everything after it, and the rest of the file shows as different.

- `↑ ↓ ← →` / `PgUp PgDn` — Move; `Home` / `End` — start / end of the row;
  `Ctrl-Home` / `Ctrl-End` — start / end of the files
- `Ctrl-↓` / `n` — Next difference; `Ctrl-↑` / `N` — previous. Asked for
  before the scan gets there, the step waits for it.
- `F5` / `g` — Go to an offset (`0x1F0`, `1F0h` or decimal)
- the mouse wheel scrolls; a click puts the cursor on a byte
- `Esc` / `F10` / `q` — Close


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


## Send a file over the LAN

*File menu → Send over LAN…* (also in the command palette).

Shares the highlighted file with another device on the same network — a phone,
a tablet, or a laptop — without any cloud service, account, or cable. Rat
Commander starts a small one-shot HTTP server and shows the download URL as a
**QR code**; scan it with the other device's camera and the file downloads.

**Operation.**

- Put the cursor on a file (or **tag** several files / a directory) and choose
  **Send over LAN**.
- A single file is served as-is. A **multi-file or directory** selection is
  **zipped** first, with a progress bar; the zip is named after the directory.
- The dialog shows the **QR code** — a true-pixel image on terminals with a
  graphics protocol (Kitty / Sixel / iTerm2), or **half-block cell art**
  otherwise — plus the URL, the file name and size, and a live count of
  completed downloads. The QR is always drawn black-on-white so a phone camera
  reads it regardless of your colour theme.
- Scan the code (or type the URL) on a device on the **same network**. The URL
  uses a **LAN IP address** (a private one is preferred when the machine has
  several) and a **free port** chosen automatically.
- Press **Esc**, **Enter**, or click to **close** the dialog. Closing stops the
  server immediately and deletes any temporary zip — the link stops working, so
  keep the dialog open until the transfer finishes.

**Notes.**

- The server binds all interfaces on the LAN so any device on the subnet can
  reach it; it only ever serves this one file and forgets everything when closed.
- Only **local** files can be sent (a remote or in-archive panel has nothing on
  local disk to serve). Firewalls that block inbound connections on the chosen
  port will stop the download.


## Receive files over the LAN

*File menu → Receive over LAN…* (also in the command palette).

The other way round: take files **from** a phone, tablet or laptop on the same
network into the directory the active panel shows — photos off a phone without
a cable, a cloud service or an app. Rat Commander serves a small upload page
and shows its address as a **QR code**; scan it, choose files, and they arrive.

**Operation.**

- Go to the directory the files should land in, and choose **Receive over LAN**.
- Scan the **QR code** (or type the URL) on the other device. The page it opens
  has one button, **Choose files** — on a phone that also offers the camera and
  the photo library — and sends what you pick one after another, each with its
  own progress bar and, when done, the name it was saved as.
- The dialog shows the URL, where the files are going, and the file on its way
  with a progress bar and its speed; after that, how many files have arrived,
  their total size and the last one's name.
- A file with a name already taken is **never overwritten**: it is saved as
  `photo (1).jpg`, `photo (2).jpg`, and so on.
- Press **Esc**, **Enter**, or click to **close** the dialog. That stops the
  server at once; an upload still running is abandoned, and nothing of it is
  kept.

**Notes.**

- Every URL carries a **random token**, and the server answers nothing else —
  someone on the network who hasn't seen the code can't find the page, let alone
  send to it. Like Send, it is plain HTTP on your LAN: fine for a home or office
  network, not a way to pass files across the internet.
- A file is written under a hidden `.rc-upload-….part` name and only moved into
  place once it has arrived whole, so a half-received file never appears under
  its real name, and one cut off by a dropped connection, a phone that went to
  sleep, or closing the dialog leaves nothing behind. An upload that would not fit on the
  disk is refused before it starts.
- Names are reduced to a plain file name, so nothing can be written outside the
  directory. Only **local** directories can receive.


## The viewer (F3)

A read-only file viewer with text and hex modes, search,
syntax highlighting, a Markdown render mode, a spreadsheet table for CSV files, fullscreen image and 3D model
views, and a look inside executables and libraries.

**Useful for** quickly reading a file — including very large ones — without
loading it into an editor.

**Operation and options.**

- **Image view** — opening a supported image (`.png`, `.jpg`/`.jpeg`, `.gif`,
  `.bmp`, `.webp`, and vector `.svg`/`.svgz`) shows it **fullscreen**, centred: a true-pixel image on
  terminals with a graphics protocol (Kitty / Sixel / iTerm2), or **half-block
  cell art** otherwise. The header shows the original pixel dimensions. Press
  **F8** to toggle between the image and the **raw** bytes (as text/hex), and
  **F4** switches that raw view between text and hex. If a file can't be decoded
  as an image, the viewer just opens it as raw text/hex as usual. An SVG is
  drawn on white, as a browser shows it, and a small one is scaled up to stay
  sharp; only images embedded in the file itself are drawn, never ones it
  names by path.
- **Document view** — opening a document shows what it **reads as** rather than
  its bytes: **Word** (`.docx`), **PowerPoint** (`.pptx`), **OpenDocument** text
  and presentations (`.odt`, `.odp`), **EPUB** books and **PDF** files open as
  text with their headings, lists and tables, and **spreadsheets** (`.xlsx`,
  `.xlsm`, `.xlsb`, `.xls`, `.ods`) open in the table view, one sheet at a time.
  - Headings are coloured and make up the **F6 outline**, so a long report, a
    book's chapters, a deck's slides (*Slide 3: Title*) or a PDF's pages (*Page
    12*) are a keystroke apart. Text wraps at word boundaries, and **F7**
    searches it.
  - Tables are drawn with box lines, and long cells wrap inside their column.
  - A spreadsheet shows its cells' values (a formula shows what it last
    computed), with dates written as dates. The header names the sheet;
    `]` / `[` (or `F6` / `Shift-F6`, `Ctrl-PgDn` / `Ctrl-PgUp`) move between
    sheets, and each keeps its own place.
  - **F8** switches to the file's raw bytes and back, and **F4** shows them in
    hex.
  - A document that cannot be read (damaged, encrypted, or one that takes the
    reader more than 30 seconds) opens on its bytes, with the reason in a
    message. A PDF is read for its text only, so a scanned page says it has
    none.
  - Everything is read by the program itself, with no office suite or PDF tool
    installed, and documents from archives and remote hosts open the same way.
- **Model view** — opening a 3D model (`.stl`, binary or ASCII, and `.obj`)
  shows the mesh **fullscreen** as a shaded solid you can turn: `← → ↑ ↓`
  **orbit**, `+` / `-` **zoom**, `Home` re-frames it, and dragging with the mouse
  orbits while the wheel zooms. The header names the format and the triangle
  count. As with images, it is a true-pixel render on a terminal with a graphics
  protocol, **half-block cell art** on a truecolor one, and an **ASCII luminance
  ramp** otherwise; **F8** toggles between the model and the raw bytes.

  Both formats are read natively — no converter and no external viewer — and the
  facet normals stored in the file are ignored in favour of ones computed from
  each triangle's winding, because exporters write them wrongly often enough that
  trusting them produces randomly unlit faces. A file that does not parse (or one
  that is too large) simply opens as raw text/hex, so a `.stl` that is not really
  one behaves exactly as it always did.

  Model files also get their own colour in the panel listings and their own
  solid — a cut gem — in the 3D landscape view.
- **Audio view** — opening an audio file (`.wav`, `.flac`, `.mp3`, `.ogg`/`.oga`
  Vorbis, `.m4a`/`.aac` AAC or ALAC, `.aiff`, `.caf`, `.mka`) draws the **whole
  file** as a **spectrogram** — time across, frequency up on a log scale,
  loudness as colour — or as a **waveform**, its peak envelope with the RMS level
  inside. **F2** switches between the two for the file on screen; *Audio view*
  (Settings → Panels) picks which one files open on. The picture fills in while
  the file is decoded in the background, so even an hour-long recording shows up
  at once and completes in seconds. It uses the usual three tiers: true pixels on
  a graphics terminal, half-block art on a truecolor one, an ASCII ramp
  otherwise.

  Beneath the picture sit a **progress row** marking the play position, a time
  scale, and the **transport row**: start, back 5 s, play / pause, stop, forward
  5 s, the time played and the length, and the **volume** as a bar with − and +
  beside it. All of it answers the mouse — **click anywhere on the picture or the
  progress row to jump there**, or drag along it and let go where you want to be
  — and the keys: `Space` plays and pauses, `s` stops, `← →` seek five seconds
  and `PgUp`/`PgDn` thirty, `Home`/`End` jump to either end, and `+`/`-` (or
  `↑`/`↓`) set the volume. The header names the format, the picture, the play
  state and time, and the artist and title when the file is tagged. Nothing
  plays until you ask — unless *Auto-play audio in the viewer* (Settings →
  Panels) is on, in which case F3 starts the file playing as it opens — and
  closing the viewer stops it. **F8** toggles to the
  raw bytes; Opus files, and anything that does not decode, open as raw bytes
  straight away.

  Only one file plays at a time, across the whole program: starting one stops
  whatever the viewer or a Details view was playing, and the volume is shared
  between them. Handing the terminal to another program — `Ctrl-O`, a command,
  an external editor — pauses playback. Playback needs the **`audio`** build
  feature (on by default); a build without it — the 32-bit Raspberry Pi package
  is one — or a machine with no sound device still draws the picture and says
  *No audio output* where the buttons are.
- **Text / Hex / Map** — **F4** cycles the three (and the binary view, for a
  file that is one). Hex mode shows an offset / hex / ASCII dump.
- **Byte map** — the third **F4** mode draws the **whole file as one picture**.
  Each cell is a span of the file, coloured either by **density** (its Shannon
  entropy, as a fraction of the most a sample that size could score) or, with
  **F8**, by **byte class** — mostly-zero padding, printable ASCII, high bytes,
  or mixed. Compressed and encrypted regions come out bright and flat, padding
  and sparse holes come out dark, and the boundaries between a container's parts
  appear as visible bands.

  Move the cursor with `← → ↑ ↓` (and `PgUp`/`PgDn`, `Home`/`End`); the header
  reports the byte offset and entropy under it. Press **Enter** and the **hex
  view opens at that offset** — which is what makes this a way of finding
  something rather than only a picture of it.

  The file is **sampled rather than read whole**: each cell reads at most 4 KiB
  from the start of its span, so building the map costs the same bounded work on
  a 4 MB file as on a 40 GB disk image. The trade is that something small hiding
  in the middle of a large span will not register.
- **Binary view** — **F3** on an **executable or a library** opens what it is
  made of instead of a screen of bytes: **ELF** (Linux and the BSDs), **PE**
  (Windows `.exe`, `.dll`, `.sys`, .NET assemblies) and **Mach-O** (macOS,
  universal binaries included). All three are read natively on every platform,
  so a Windows DLL can be looked into from Linux and a Mach-O from Windows. A
  file is recognised by its contents rather than its name, so a program with no
  extension opens this way too, and one that merely starts like a binary but
  does not parse opens as text, as it always did. A find-file content hit still
  opens on its matching line; **F4** reaches the binary view from there, and from
  the binary view steps on to text, hex and the byte map.

  Seven lists, one on screen at a time. **Tab** / **Shift-Tab** step through
  them, **1**–**7** jump straight to one, and a click on a title opens it:

  | List | What it holds |
  | --- | --- |
  | Info | Format, architecture, type (executable, position-independent executable, shared library, object file, core dump), entry point, interpreter, platform and minimum OS, subsystem, whether it is a .NET assembly, link time, the name it is loaded by and the paths it searches for libraries, whether it has a symbol table and debug info, its build ID (GNU build ID, Mach-O UUID or PDB signature), and its **hardening**: PIE, NX, RELRO, stack canary and FORTIFY for ELF; ASLR, DEP and CFG for PE; PIE and code signature for Mach-O |
  | Sections | Address, file offset, size and kind of every section |
  | Libraries | The shared libraries it loads, with delay-loaded, weak and re-exported ones marked, and a Mach-O library's version |
  | Imports | Every symbol it takes from elsewhere, with the library it comes from (and the symbol version, on ELF) |
  | Exports | The symbols it offers, and where a forwarded one really lives |
  | Functions | Every function, with its address and size |
  | Strings | The readable text inside it, with the section each string is in |

  `↑ ↓` / `PgUp PgDn` / `Home End` (or a click, or the wheel) move the
  highlight, and `← →` scroll a long name or string sideways. **Enter** opens
  the **hex view at the highlighted row's bytes** — a section, a function, a
  string — the same way out the byte map offers. **F8** shows Rust and C++
  symbol names **demangled** (the default) or as the linker spells them.

  **F7** searches the list on screen, and `n` moves on to the next match; a
  symbol is found by either spelling of its name, or by its address. The search
  dialog's **Find all** narrows **every** list to its matching rows at once — so
  one term shows the imports, functions and strings to do with it — and the
  header names the filter until **Esc** drops it. **F5** picks a row by number or
  by percentage, or, given a byte offset, opens the hex view there.

  **Functions in a stripped file.** Most programs ship without a symbol table,
  but every one keeps the tables its exceptions and backtraces unwind through —
  `.eh_frame` on ELF, `.pdata` on 64-bit Windows, `LC_FUNCTION_STARTS` on Mach-O
  — and those list nearly every function's start and length. The functions list
  merges them with whatever symbols and exports the file has; a function nothing
  names shows as `sub_` and its address. 32-bit Windows images have no such
  table, so only their exports are listed.

  **Strings without the noise.** Text is found as UTF-8 (any script, not just
  ASCII) and as UTF-16, the way Windows stores it. Unlike a plain `strings`, runs
  with no letter or digit, or made of one repeated character, are dropped; a run
  is split where a letter of one script runs straight into a letter of another,
  which words never do and random bytes constantly do; and inside machine code
  only sentence-like ASCII counts, so thousands of function prologues that
  happen to be printable do not bury the real strings. A .NET assembly's code
  section is IL and metadata rather than machine code, and is read as data.

  The analysis runs **in the background**. An ordinary program opens straight
  into its lists; a debug build of several hundred megabytes shows *Analyzing…*
  for a second or two while the rest of the program carries on. Only headers and
  tables are read from the file, the strings pass streams it, and every list
  stops at 250,000 rows (its count then shown with a `+`), so a pathological file
  costs bounded memory.
- **Certificates and keys** — **F3** on a certificate or key file shows what it
  holds rather than base64. A file is taken for one by its name (`.pem`,
  `.crt`, `.cer`, `.der`, `.csr`, `.p10`, `.key`, `.pub`, `ca-bundle…`,
  `id_…`, `…-cert.pub`, `authorized_keys`, `known_hosts`) or by beginning with a
  PEM block or an SSH key, so a README that quotes a certificate stays text.
  PEM files may hold any number of blocks with text around them; DER files are
  read by their name. Files over 4 MiB are not inspected.

  Up to six tabs, only those with something in them. **Tab** / **Shift-Tab**
  step through them, **1**–**6** jump straight to one, and a click on a title
  opens it:

  | Tab | What it holds |
  | --- | --- |
  | Summary | One row per certificate, request and key: who a certificate is for and when it expires, what a key is |
  | Certificates | Each certificate field by field: subject, issuer, serial number, validity, public key type and size, signature algorithm, subject alternative names, basic constraints, key usage and extended key usage, key identifiers, CRL distribution points, OCSP and CA-issuer addresses, SHA-256 and SHA-1 fingerprints, and the SPKI SHA-256 pin |
  | Requests | Each certificate request: its subject, key, the names and extensions it asks for, and whether its signature was made with its own key |
  | Certificates (SSH) | Each OpenSSH certificate: user or host, key ID, serial number, the principals it is valid for (flagged when it lists none, which means any), validity, its key and the signing CA's fingerprints, whether the CA's signature holds, critical options and extensions |
  | Keys | Each private or public key: its algorithm and size, how it is stored (PKCS#1, PKCS#8, SEC1, SubjectPublicKeyInfo, OpenSSH), how an encrypted one is encrypted, its SPKI SHA-256 pin or SSH fingerprint, and the certificate in the file it belongs to |
  | Entries | Each line of an `authorized_keys` file (its key, fingerprint, comment and options) or a `known_hosts` file (its hosts — or that they are hashed — any `@cert-authority` or `@revoked` marker, and its key), with lines that can't be read marked |
  | Chain | Whether each certificate is followed by its issuer, and for each one who issued it and whether that issuer's key verifies its signature |

  Values that need attention are coloured: a certificate that has **expired**
  or is not valid yet, and a signature that **does not verify**, in red; one
  **expiring within 30 days**, an RSA key under 2048 bits and an MD5 or SHA-1
  signature, in the warning colour; a verified signature and a certificate with
  time to go, in green.

  A private key is matched to its certificate by its public key, which RSA and
  SEC1 keys store beside the private one — nothing is decrypted, and **no key
  material is shown**. An OpenSSH private key keeps its public half
  unencrypted, so its fingerprint shows even when it has a passphrase. An encrypted key shows only how it is encrypted. The
  chain is checked **within the file only**: whether the system trusts the root,
  whether the certificate fits a host name, and whether it has been revoked are
  not looked at, and the Chain tab says so.

  **Enter** goes to the line of the raw text the highlighted row came from, and
  **F8** switches between the certificates and the raw text. **F7** searches
  the rows of the tab on screen, `n` moves on to the next match, and **Find
  all** narrows every tab to the matching rows (keeping the heading of the
  certificate or key each belongs to) until **Esc** drops it.
- **Line wrap** — **F2** toggles soft wrapping.
- **Search** — **F7** opens the **same search dialog the editor uses** (see
  *Search and replace* under the editor): Normal / Regular expression / Hex /
  Wildcard modes, with *Case sensitive*, *Backwards* and *Whole words*, plus the
  **Find all** button, which tints every line holding the term. **`n`** repeats
  the last search, and so does re-running the same one from the dialog — each
  repeat moves to the **next** occurrence and wraps at the end. Changing the term
  or any option starts again from the top. Search streams the file in windows, so
  it works on huge files without loading them.
- **Goto** — **F5** jumps to a line number, a percentage through the file, or a
  decimal/hex byte offset (in hex mode the line number is a 16-byte row).
- **Follow mode** — **`f`** keeps the view on the end of a file that is still
  being written, the way `tail -f` does: jump to the last page, and move with it
  as lines are appended. The header reads **[Follow]**.

  Scroll up to read something and following **pauses** — the view stays put, and
  the header counts what has arrived since (**[Paused +12]**). Scroll back down
  to the last page, or press **End**, and it picks up again. **`f`** a second
  time stops following altogether.

  A file that is **truncated** (`> app.log`, or logrotate's `copytruncate`) is
  read again from the start. On Linux and macOS a log that is **rotated** away —
  renamed, with a new file created in its place — is noticed too, and the viewer
  moves on to the new file, as `tail -F` would. Follow mode works on local files
  only; a remote file is viewed from a temporary copy, which never grows.
- **Git blame** — **`b`** on a file inside a git work tree adds a column beside
  the text saying which commit last changed each line. It runs `git blame` in
  the background, so a long history never holds the viewer up; the header reads
  **[Blame…]** until it is ready.

  Each line gets a bar shaded by the **age** of its commit — the file's newest
  change brightest, each older one a step dimmer. The steps go by the order of
  the commits rather than their dates, so every change in the file stays
  tellable apart even when most of its history happened in one busy week. The
  author and date are written once for each run of lines from the same commit
  (and on the cursor line), so a block reads as one change rather than a column
  of repeated names. Lines that are not committed yet say so. On a narrow screen
  the column shrinks to the date.

  While the column is up, `↑ ↓` / `PgUp PgDn` / `Home End` (or a click) move a
  **line cursor**, and the header shows that line's commit: its id, author, date
  and subject. **Enter** closes the viewer and walks the active panel into the
  repository's history — to the directory holding the file *as it was in that
  commit*, with the file under the cursor (see *Browsing git history*). From
  there **F3** shows that version, and *Compare files* diffs it against today's.
  A file renamed since is found under its old name. **`b`** again hides the
  column. Blame accompanies the raw text only: hex, the byte map and a rendered
  Markdown view put it away until you come back.
- **Log levels** — in a file with no syntax of its own that is either named like
  a log (`*.log`, `*.log.1`) or being followed, a line that names its severity
  near the start is drawn in that severity's colour: errors (`ERROR`, `FATAL`,
  `err`, …) in the theme's error colour, warnings in its accent colour, and debug
  and trace output dimmed. Syslog, logfmt (`level=warn`), bracketed (`[ERROR]`)
  and most application loggers are all recognised; only the start of the line is
  looked at, so a message that merely mentions an error is left alone.
- **Syntax highlighting** colors recognized source files, using a bundled theme
  matched to the active light/dark UI. It covers syntect's default languages
  plus bundled extras (TOML, INI, Dockerfile, HCL/Terraform, GraphQL, Protobuf,
  CMake, TypeScript/TSX, Kotlin, Swift, SCSS/Sass, Elixir, Zig, Nix and more).
- **Table view** — `.csv`, `.tsv` and `.tab` files open as a **spreadsheet**: a
  grid of cells with row numbers down the side, the columns separated by rules,
  numbers right-aligned, and a **cell bar** along the top naming the cell under
  the cursor (`B12`), its column's title, and its whole value. When the first
  record reads as column titles — every cell filled, none a number, none twice —
  it becomes a **header row** that stays in place while the rows scroll beneath
  it; **F2** turns that on or off. Without a header the columns are lettered A,
  B, C… as in a spreadsheet.

  The delimiter is worked out from the file: commas, semicolons (the European
  convention, where the comma is the decimal separator), tabs or pipes —
  whichever splits the first records into the same number of fields most
  consistently. `.tsv` and `.tab` files are always tab-separated. Quoted fields
  may hold the delimiter, doubled quotes and **line breaks**; a line break shows
  as `↵` in its cell, and the record after it is still the next row. Quotes are
  read leniently — one in the middle of a field is just a character — and a file
  whose quotes never close is read as if it had none, rather than as one giant
  field.

  Move the **cell cursor** with the arrows, `PgUp`/`PgDn`, `Home`/`End` (first
  and last field of the record) and `Ctrl-Home`/`Ctrl-End` (first and last
  record); `Tab` and `Shift-Tab` walk the cells in reading order. Columns are
  sized to their contents, up to 40 cells, and `<` / `>` (or `Ctrl-←`/`Ctrl-→`)
  make the one under the cursor narrower or wider; the cell bar always shows a
  cut value whole. A click picks a cell, and the wheel scrolls.

  **F7** searches the file as usual and puts the cursor on the **cell** holding
  the match; **Find all** tints every record with a hit. **F5** goes to a row
  number, a percentage, or the record holding a byte offset. **F8** shows the
  raw text on the record the cursor was on, and F8 again returns to the table
  there; **F4** steps on to hex and the byte map. Like the text view, the table
  is paged from disk: only the start of each record is indexed, as far as you
  have moved, so a multi-gigabyte export opens at once (the header's row count
  carries a `+` until the end has been reached).
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
and replace, undo/redo, syntax highlighting, a pulldown menu on **F9**, and an
in-place hex editor.

**Useful for** quick edits without leaving the file manager.

**Starting a new file.** **Shift-F4** in the panels asks for a file name and
opens the editor on it, in the active panel's directory — the file itself is
created by the first save (**F2**). A name that already exists there simply
opens that file, and the name may point into a subdirectory of the panel's
directory (`notes/todo.txt`). It works wherever the panel is pointed: on disk,
inside a writable archive, or on a remote server.

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
- **Ctrl-C** / **Ctrl-X** / **Ctrl-V** — copy or cut the block to the
  clipboard, paste it.
- **Ctrl-A** — mark the whole file; **Edit → Unmark** drops the mark.

**Search and replace.** **F7** searches; **F4** opens search & replace. Both use
the same dialog, and the viewer (F3) uses it too:

- **Mode** — **Normal** (literal text), **Regular expression**, **Hex** (byte
  strings like `48 65 6c`), or **Wildcard** (`*` and `?`). In regex mode `^` and
  `$` anchor to **each line**, so `^fn ` finds every line starting with `fn `;
  `.` does not cross a line break.
- **Options** — *Case sensitive*, *Backwards*, *Whole words*, and, when
  replacing, *In selection*.
- **Buttons** — **OK** finds the next match (pressing Enter from any field does
  the same), **Cancel** closes, and **Find all** — between them — highlights
  **every line holding the term** at once.

**Find all** is for reading rather than jumping: the matching lines stay tinted
(in the theme's inactive-cursor shade) while you scroll, edit and search onward,
so you can see where the hits are without stepping through them. The highlight
survives ordinary searches and is replaced only by the **next Find all** — or
dropped when the editor closes. It honours the mode and options, so you can
highlight, say, every whole-word `foo` case-sensitively. The status line reports
how many lines were marked, and the cursor lands on the first of them.

**Saving.** **F2** writes the file in place (after a confirmation, unless
*Confirm before saving* is turned off). **Save as** (**Shift-F2** or
**Ctrl-F2**) opens a browser — navigate directories and type a file name,
prefilled with the current one — to write the buffer somewhere else; the editor
then continues editing the new file. If a normal save fails (a read-only
location, a permission error, …), the Save-as browser opens automatically with
the reason shown, so you can redirect the write without losing your work.

**Word wrap.** **Shift-F9** toggles virtual word wrap: long
lines are shown across several screen rows without changing the file, and each
*continued* row ends in a **`>`** marker so soft wraps are distinguishable from
real line breaks. Cursor movement, scrolling and the mouse all follow the
visible (wrapped) rows; `WRAP` shows on the status line while it is on.

**Help.** **F1** brings up a list of the editor's keyboard shortcuts and what
they do; any key closes it. While **Shift** or **Ctrl** is held, the F-key bar
relabels the keys those modifiers reach — **F2 → Save as**, and with Shift also
**F5 → InsFil**, **F7 → Again** and **F9 → Wrap**, with Ctrl **F9 → Hex** (on
terminals that report held modifier keys via the enhanced keyboard protocol).

**The menu.** **F9** opens a pulldown menu bar over the status row — see *The
editor menu (F9)* in the key reference above for what each of its six menus
offers, and *Editor options* for the settings dialog behind Options → General.

**Bookmarks.** **Alt-K** marks the line the cursor is on (its text turns the
"marked" colour); **Alt-J** and **Alt-I** step forward and back through the
marked lines, wrapping around, and **Alt-O** clears them all. Bookmarks live for
as long as the file is open.

**Reformatting text.** **Alt-P** re-wraps the paragraph around the cursor (blank
lines delimit it) to the configured *Word wrap line length*, keeping the
paragraph's own indentation; one undo puts the whole reflow back. **Alt-T**
sorts the marked block's lines — or the whole file when nothing is marked — with
reverse / ignore-case / remove-duplicates options. **Alt-U** runs a shell
command and pastes its output at the cursor (non-interactively, so aliases do
not expand — an interactive shell would take the terminal away from the
editor).

**Other.** **Ctrl-Z** / **Ctrl-Y** undo and redo (see the *Group undo* option).
**Ins** switches between insert and overwrite typing. **Alt-L** jumps to a line
number and **Alt-B** to the bracket matching the one at the cursor. The status
bar shows the byte under the cursor, the line and column, and the totals.
Syntax highlighting updates incrementally as you type (**Ctrl-S** toggles it).

**Spreadsheet grid (Alt-G).** A `.csv`, `.tsv` or `.tab` file opens as a
**grid of cells** — the same table the viewer shows, with the same delimiter
detection, header row and cell bar — and it is edited there: move to a cell and
**type** to replace its value, or press **Enter** to change the value it has.
The value is edited in the **cell bar** along the top, where a long one has
room, and written back when you press Enter (which moves down, as a spreadsheet
does), Tab or an arrow key; Esc throws the edit away. **F5** and **F6** insert a
row above and a column to the left of the cursor, **F8** and **Shift-F8** delete
the row and the column, and **F3** says whether the first row holds the column
titles. There is always an **empty row below the table and an empty column
beside it**: type into one and the table grows, and a cell typed into past the
end of a short record fills in the delimiters before it.

The grid is a way of *editing the text*, not a copy of it. Every change is an
ordinary edit of the characters the cell covers — quoting the value when it
holds the delimiter, a quote or a line break, and keeping a field quoted that
already was — so a cell, a new row or a whole new column is **one undo step**,
**F2** saves exactly what the grid shows, the file's own line endings (LF or
CR LF) are kept, and search and replace (**F7**, **F4**) work as always and put
the cursor on the cell holding the match. **Alt-G** switches to the text with
the cursor on the cell that was selected, and back — for any file, so a table
without a table's name can be edited as one too.

**Syntax checks.** A `.json` file — and `.geojson`, `.topojson`, JSON Lines
(`.jsonl`, `.ndjson`) and JSONC (`.jsonc`, and the `tsconfig.json`-style
configuration files that allow comments) — is **checked as you type**. Once you
pause for a quarter of a second the whole file is read again in the background,
and **every** syntax error is shown, not only the first: each line with an error
gets a red `✗` in a two-column **gutter** on the left, the characters it is
about are drawn in the theme's error colour and underlined, and the status line
counts them (`✗ 3`). With the cursor on an error's line the status line says
what is wrong — *Missing ',' after this value*, *Trailing comma before ']'*,
*Keys must be strings in double quotes*, *Expected '}' to close the object from
line 12*. **Alt-E** jumps to the next error and **Alt-Shift-E** to the previous
one, wrapping round, with the message on the bottom line.

The checker reads past each mistake the way a person would, so one slip is one
error rather than a cascade: a value with its comma missing is still the next
value, a key without its colon is still the key, and a closing bracket of the
wrong kind closes the container it was meant for. What it points out goes
beyond missing punctuation: comments and trailing commas (in files that do not
allow them), single-quoted or unquoted strings and keys, `True`, `None`, `NaN`
and other words JSON does not have, numbers with leading zeros, a leading `+` or
a hexadecimal prefix, invalid escapes and raw tabs inside strings, strings that
never close (the check picks up again on the next line), brackets left open at
the end of the file — reported where they were opened — and anything after the
end of the document. JSONC files may have comments and trailing commas; JSON
Lines files may hold one document after another. A file with more than a
thousand errors is not checked past the thousandth. JSON5 is a different
language and is not checked.

**JSON tools.** In a JSON file, **Alt-F** (or Format → JSON → *Pretty-print*)
lays the whole document out one value to a line, indented by the editor's tab
setting (spaces, or a tab when *Fill tabs with spaces* is off); *Minify* writes
it on one line with no spaces; *Sort keys* sorts every object's keys, at every
depth (by what the key says, so `"\u0061"` sorts as `a`), and lays it out as
pretty-printing does. Nothing else about the document changes: keys stay in
their order (except when sorting), numbers and strings are kept exactly as
written — `1.0e+2` is not turned into `100`, and escapes stay escapes — and in
JSONC files the comments stay with the member they are about (a comment on a
member's line stays at the end of that line, and moves with it when sorting);
minifying drops them, and says so. A JSON Lines file stays one document to a
line. The tools refuse a document that has syntax errors, since what it means
would be a guess. The rewrite is one step for **Ctrl-Z**, and the cursor stays
on the token it was on.

**TOML, YAML and XML** files are checked the same way — the same gutter,
underlines, status line and **Alt-E** — each by its own rules:

- **TOML** (`.toml`, `Cargo.lock`, `poetry.lock`, `uv.lock`): every error the
  parser finds, which reads on past each one — a value that isn't one, a table
  header left open, a key given twice, a table defined twice.
- **YAML** (`.yaml`, `.yml`), read as YAML 1.2: the first syntax error (a YAML
  parser can't tell what follows a broken indentation or an unclosed quote, so
  it stops there), and before it **every key given twice in one mapping** —
  which YAML forbids but many tools silently accept, keeping one of the two.
- **XML** (`.xml`, `.svg`, `.xsd`, `.xsl`, `.csproj` and the other MSBuild files,
  `.xaml`, `.kml`, `.gpx`, `.plist`, …): end tags that don't match — reported,
  and read past, so the errors after one still show — elements never closed,
  attributes given twice or malformed, a second root element or text outside
  the root, and a fatal error such as an unterminated attribute, after which
  the rest of the file can't be read. Only well-formedness is checked, not a
  schema or DTD.

**JSON Schema.** Once a JSON, YAML or TOML file's syntax is right, it is also
checked against its **JSON Schema**, and what the schema doesn't allow is
marked the same way — with a `!` in the gutter and an underline in the warning
colour rather than the error colour, a `! n` count on the status line, and
**Alt-E** stepping through these too. An unknown or misspelt key is marked
where the key is, a missing required one at the object that lacks it, and a
value of the wrong type or outside the allowed set at the value. The schema is,
in this order:

1. The one the file names: `"$schema": "…"` at the top of a JSON document, a
   `# yaml-language-server: $schema=…` comment in YAML, or `#:schema …` in TOML
   (comments are looked for in the first 20 lines). It may be the URL of a
   bundled schema, a `file://` URL, or a path relative to the file; a file that
   names any other URL is not validated.
2. The one your mapping gives the file: `schemas/schemas.toml` in the
   configuration directory (`~/.config/rat-commander/schemas/schemas.toml` on
   Linux), with a `[[map]]` table per rule, shown below. The first rule with a
   matching glob wins.
3. The bundled one for a well-known file: **Compose** (`docker-compose.yml`,
   `compose.yaml` and their `compose.*.yaml` variants),
   **GitHub Actions** workflows (`.github/workflows/*.yml`) and `action.yml`,
   **Dependabot** (`.github/dependabot.yml`), **GitLab CI** (`.gitlab-ci.yml`),
   `Cargo.toml`, `package.json` and `tsconfig.json` / `jsconfig.json`.

```toml
[[map]]
files = ["**/deploy/*.yaml"]    # globs matched against the absolute path;
                                # * stops at a /, ** does not
schema = "deploy.schema.json"   # beside schemas.toml, a bundled URL, or "none"
```

Nothing is fetched from the network: a schema's references to other bundled
schemas and to local files are followed, and a reference to anything else is
taken as allowing anything. Files over 4 MiB are not validated, and at most 200
schema errors are shown. The bundled schemas are those published by SchemaStore
and the Compose Specification (Apache-2.0) and GitLab (MIT); their sources and
licenses are listed in `assets/schemas/README.md`.

**Shapefiles (F3, F4).** A `.shp` is binary and comes with a `.shx` index, a
`.dbf` of attributes and usually a `.prj`. Opening one shows it as the
**GeoJSON it becomes** — one feature per shape, with its attributes as
properties — so everything that already works on GeoJSON works on it:

- **F3** shows that GeoJSON, with syntax highlighting and search.
- **F4** opens it in the editor, and **Alt-M** draws it on the world map, where
  it can be panned, clicked, and edited like any other GeoJSON — `e` to edit,
  `1`/`2`/`3` to draw, `Del` to remove, `Ctrl-Z` to undo.
- **F2** writes it back into the `.shp`, `.shx` and `.dbf` — not to a new file.

Some details worth knowing:

- **Coordinate systems.** A shapefile is often in metres on a projected grid
  rather than in degrees. The `.prj` is read, and **Web Mercator** and **UTM on
  WGS84** are converted to degrees on the way in and back to the file's own
  coordinates on the way out, so the round trip is exact. A projection that
  cannot be converted is refused with a message naming it, rather than opening
  as an empty map.
- **Attributes.** The `.dbf` schema — the field names, types and widths — is
  kept and written back as it was. A property you add on the map that the schema
  has no column for gets one, with its type inferred; a value too long for an
  existing column is cut. Either way you are told after the save.
- **One shape type per file.** The format allows a file only one kind of
  geometry, so drawing a polygon into a file of points cannot be saved; those
  features are left out and counted in the message.
- **Saving is all-or-nothing.** The three files are written beside the originals
  and moved into place only once all three have been written, so a failed save
  never leaves a half-replaced set behind. The `.prj` is never touched.

**Audio tags (F4).** Opening an MP3, Ogg, FLAC, M4A, WAV or other audio file
with **F4** opens a **page of its tags** — nothing else is loaded, since an
audio file is not text and is not usually something you want to patch bytes in.

The well-known fields come first — Title, Artist, Album, Album artist, Track,
Disc, Year, Genre, Comment, Composer — followed by **any other tag the file
carries**, under *Other tags in this file*. Both are editable.

- **↑↓** pick a row; **Enter** (or **F4**, or simply typing) edits it; **Enter**
  commits and **Esc** abandons the edit. **F8** or **Del** clears a row.
- **F5** or **Ins** adds a tag the file does not have yet. A picker lists every
  key the file's tag format can hold — type to narrow it, **Enter** to add —
  and the new row is ready to type into straight away. Only keys that format
  actually supports are offered: an MP3's ID3v2 has no BPM frame of its own,
  for instance, so a value written under one would simply be gone by the next
  read.
- **F2** writes the tags back into the file. A row left empty is *removed* from
  the tag rather than written blank, which is what players expect — so a tag
  added and left blank is not written at all.
- Embedded cover art is counted and kept; so is anything binary the file holds.
- A tag whose key the file uses **more than once** (two performers, say) is
  shown greyed and left alone: editing it by key would collapse every value
  into the one being edited.
- **Year** is stored by some formats inside a full recording date. Where it is,
  changing the year rewrites that date — keeping its month and day — rather
  than adding a second year that would disagree with it.
- **F3**, **Alt-T** or **Ctrl-F9** shows the file's raw bytes (the ordinary
  in-place hex editor) and switches back again, so nothing is hidden from you.
  The byte editor is only opened if you ask for it.

The status row names the kind of tag being written — ID3v2 for an MP3, Vorbis
comments for an Ogg, and so on — so it is clear what is going into the file.

Writing a tag re-lays-out the container, so if you have also edited bytes in the
hex view those are written first and the hex view is reopened on the rewritten
file.

**GeoJSON map (Alt-M).** Draws the GeoJSON in the file over a **map of the
world** — a `.geojson` file, or GeoJSON anywhere inside a larger JSON document,
such as the `geometry` in an API response. Every piece of GeoJSON the file holds
is found: a FeatureCollection counts as one, a Feature is not counted again for
its own geometry, and anything else — a bare geometry under some other key, a
list of features outside a collection — is listed with the path it was found at
(`$.data.regions[2].shape`). With more than one, a list down the left picks
which to look at; the others stay on the map, dimmed. The dialog opens on the
object and feature the editor's cursor is in, framed.

The map is built into the program: land, lakes, country borders, rivers and
cities from **Natural Earth** at 1:10m, stored as vector outlines at five levels
of detail, so a coastline is a clean line at the whole-world view and when
zoomed in on a single region alike, with the names of the biggest cities that
fit. GeoJSON areas are filled translucently and outlined, lines are drawn over
them and points marked; the feature you pick is drawn in a colour of its own and
its name and properties appear along the bottom. The top row names what is
shown and the longitude and latitude under the pointer, and says when positions
had to be left out — GeoJSON in a projected coordinate system has no longitudes
to draw — or the file has syntax errors (whatever reads around them is still
drawn).

Drag the map to pan and turn the wheel to zoom about the pointer, or use the
arrow keys and `+` / `-`; `Home` frames the selection again and `w` shows the
whole world. A click picks the feature under the pointer, and `n` / `p` step
through the features one by one. **Go to** (or `Enter`) closes the map with the
editor's cursor on the picked feature (or the object), and **Close** (or `Esc`)
leaves the cursor where it was. `Tab` moves between the list and the map.

On a terminal with a graphics protocol the map is true pixels, anti-aliased;
anywhere else it is drawn in **braille** characters — two dots across and four
down per cell, the land and sea as each cell's background and the lines as the
dots — which a 16-colour terminal can show too. Its colours come from the
active theme. The file is read in the background, so even a large GeoJSON file
brings the dialog up at once.

*Editing GeoJSON on the map.* **Edit features** (or `e`) turns the map into an
editor of the GeoJSON, with a row of tools above the bottom line; `e` again, or
**Stop editing**, turns it off. Every change is made to the editor's text the
moment it is made, as one undo step of its own, and nothing else in the file is
touched: a moved position rewrites only its geometry's `coordinates`, a new
feature is added to the end of its collection's `features`, a removed one takes
its comma with it. What is written follows the layout around it — all on one
line or indented (with spaces or tabs, and as deep), with or without spaces
after commas, a position or a number to a line — and the numbers of positions
that were not moved stay exactly as they were written, altitude included. New
positions are rounded to what the zoom can point at, never finer than seven
decimal places. Close the map and save the file as usual; `Ctrl-Z` in the
editor takes the changes back too.

- **Positions.** The picked feature shows a square handle on each of its
  positions and a dot in the middle of each segment. Drag a handle to move the
  position, drag (or click) a dot to add one there; a click selects a position,
  `[` / `]` step through them, `Shift`+arrows move the selected one a cell,
  `Insert` adds one after it, and `Del` (or a right click on a handle) removes
  it. A line keeps at least two positions and a polygon three: a hole or one
  part of a Multi… geometry left smaller than that goes as a whole. With no
  position selected, `Del` removes the picked feature. A feature with too many
  positions to show at the zoom asks to be zoomed in on first.
- **Drawing.** `1`, `2` and `3` (or **Point**, **Line**, **Polygon**) draw a new
  feature: a click places each position — `Space` places one at the crosshair in
  the middle of the map, for drawing from the keyboard with the arrow keys
  panning — and `Enter`, a click back on the last position, or for a polygon a
  click on the first, finishes it. `Backspace` takes back the last position and
  `Esc` cancels. The tool stays on for the next feature until `Esc` or its key
  again. The new feature has empty properties and is picked when done.
- **Where new features go.** Into the collection chosen in the list, or the
  one the picked feature is in, or the file's first FeatureCollection; the top
  row shows which (`→ $.parks`). A file with none gets one: an empty file becomes
  a FeatureCollection, a file that is a single Feature or geometry becomes a
  FeatureCollection holding it and the new feature, a file of features one to a
  line (GeoJSON Lines) gets another line, and any other JSON gets a new
  FeatureCollection at the editor's cursor, which has to be where a value can
  go — after a `:`, a `[` or a comma — and not inside GeoJSON already there; a
  `null` at the cursor is replaced. `c` (**New collection**) makes an empty one
  the same way, to draw into.
- **Names.** `r` (**Rename**) types a name for the picked feature, written to
  the naming property it already has (`name`, `title`, `label`…) or as a new
  `name` in its properties.
- **Undo.** `Ctrl-Z` and `Ctrl-Y` step back and forth through the changes made
  on the map — in the map and in the editor's text together.

`Esc` steps back out: from a feature being drawn, from the tool, from a
selected position, then out of editing, and then closes the map.

**Hex editor (Ctrl-F9).** Toggles an in-place offset / hex / ASCII editor. Only the
visible window is read and only changed bytes are written back, so arbitrarily
large files can be hex-edited (and a file too big to load as text opens straight
into hex mode). Editing is overwrite-only (length-preserving). **Tab** switches
between the hex and ASCII columns; **F7** searches for hex bytes (`48 65 6c`) or
text, **F4** replaces all (same length), **F2** saves the changed bytes. **F9**
still opens the menu, with the text-buffer items greyed out.

### Data inspector

**F8** in hex mode (or Command → Data inspector) shows the bytes at the cursor
read as each common type, in a panel beside the bytes (or below them, when the
terminal isn't wide enough) that follows the cursor as it moves:

- **binary** — the byte's bits
- **int8** … **uint64** — signed and unsigned integers of 1, 2, 4 and 8 bytes
- **float16**, **float32**, **float64** — half, single and double precision
- **ULEB128**, **SLEB128** — variable-length integers, as DWARF, WebAssembly
  and Android's DEX use them
- **UTF-8**, **UTF-16** — the character starting at the cursor and its code
  point, or *invalid*
- **time_t** (32-bit), **time64_t**, **FILETIME**, **OLETIME**, **DOSDATE**,
  **DOSTIME** — dates and times, in UTC
- **GUID** — 16 bytes, the first three groups stored little-endian (as Windows
  does) or, big-endian, in the order they are written

A row shows *—* when too few bytes are left in the file to read one. The title
row says which **byte order** the numbers are read in; **b** (or a click on the
title) switches between little- and big-endian, and both it and whether the
inspector shows are remembered.

**Tab** reaches the inspector after the ASCII column (and before the template
tree). There **↑ ↓** choose a type, and its bytes are marked in the hex view;
**← →** move the byte cursor, and **Ctrl-← →** move it by the width of the
chosen value. **Enter** edits the value in place: numbers in the same notations
as the template tree (`-5`, `0x1F`, `1Fh`, `0b101`), dates as they are shown,
a character as itself, in quotes or as `U+00E9`, a GUID as its 32 hex digits.
The bytes go into the hex editor's unsaved changes, so **F2** saves them like
any other edit. Editing never changes the file's length, so a value must fit
the bytes it replaces: a smaller LEB128 number is padded to the length of the
one there, and a character must take as many bytes as the one it replaces.

### Binary templates

In hex mode the file is read with an **010 Editor Binary Template** — a small
C-like program that describes a file format — and the result is shown as a
tree of the file's structures and fields beside the bytes (or below them, when
the terminal isn't wide enough). Each row has the variable's **name**, its
**value**, where it **starts**, its **size**, its **type** and a **comment**,
narrower panels dropping the later columns. The template's colours tint the
bytes it covers, and the bytes of the variable selected in the tree are
highlighted.

**Picking the template.** When hex mode opens, the template that fits the file
runs by itself, in the background — its name and progress show on the status
line. A template fits when one of its **file masks** matches the file's name
and, if it has any, one of its **ID bytes** patterns matches the start of the
file; a template without file masks fits on its ID bytes alone. A plain text
file is not taken for a binary format that merely shares its extension, and an
extension several formats use (`.img`, `.dat`) only picks a template by itself
when one of them also recognises the file's first bytes. **F5**
opens the template picker: the templates that fit come first, then every other
one by category; type to filter, **Enter** uses the highlighted template, **F4**
opens it for editing, and **(No template)** stops using one. **Shift-F5** runs
the template again.

**Moving around.** **F6** selects the variable under the byte cursor in the
tree — opening whatever it is inside — and gives the tree the keys; **F6** or
**Esc** give them back to the bytes, with the byte cursor on the selected
variable (it stays put if it is already inside it). **Tab** steps round the hex
column, the ASCII column, the data inspector when it shows and the tree
(**Shift-Tab** the other way), going into and out of the tree the same way. Moving through the tree moves the byte cursor
to each variable. **→** opens a struct or array, **←** closes it or steps to its
parent, and **\*** opens everything below the selected row. Large arrays list
their elements a thousand at a time, with a row to list more.
**F3** switches the panel to the template's **output** — what it printed, its
warnings, and why it stopped if it did — and back.

**Editing values.** **Enter** on a value edits it in place. Numbers can be typed
as decimal, `0x1F`, `1Fh` or `0b101`, an enum by its constant's name, a
character as `'A'`, a string in quotes (with `\n`, `\t`, `\x41` escapes), and
dates in the format they are shown in. The value is written with the variable's
byte order and width — a bitfield keeps the bits around it — into the hex
editor's unsaved changes, so **F2** saves it like any other edit. A variable
with its own `read` function is edited through its `write` function, and is
read-only without one. After an edit the template runs again once typing pauses
(a template that took more than a second waits for **Shift-F5** instead).

**The templates.** The program comes with **307 templates** from SweetScape's
[template repository](https://www.sweetscape.com/010editor/repository/templates/)
— archives, images, audio and video, executables, fonts, disk images, databases
and many more — which it writes to **`templates/`** in the config directory on
first start. They are yours to change: an edited template is never overwritten,
one you delete is not brought back, and a later release only refreshes the ones
you left as they were. Any other `.bt` file in that directory (or a subdirectory
of it) is a template too, chosen by its header comments:

```text
//      File: MyFormat.bt
//  Category: User
//   Purpose: What it parses
// File Mask: *.myf
//  ID Bytes: 4D 59 46 [+4] 01   // bytes at the start; [+N] skips N
```

**New template…** (Command → Binary templates) starts one for the file being
viewed — its file mask and first bytes already in the header — and opens it in
the editor; **Edit template…** opens the template in use, at the line a run
stopped at. The hex editor waits underneath and comes back, running the edited
template, when that editor is closed. `#include` looks next to the including
file, then in the templates directory, then among the built-in templates.

**What runs.** Structs and unions (with arguments, recursive, on-demand with
`size=`), typedefs, enums, padded and unpadded bitfields in either direction,
duplicate arrays, strings, local variables and structs, functions with reference
parameters, the `read`, `write`, `comment`, `name`, `format`, `fgcolor`,
`bgcolor`, `style`, `hidden`, `open`, `optimize`, `pos` and `localpos`
attributes, and the reading, string, math, date, checksum, search and colour
functions. A template never changes the file while it runs: functions that
would write to it, open other files or run programs stop the template with an
error, keeping what it had built; prompts take their default answer, and
disassembly is shown as plain bytes. An array of structs whose size can vary is
read element by element when it has up to 1024 of them, and otherwise, as in
010 Editor, assumed to repeat the size of its first element (`optimize=false` /
`optimize=true` decide it for good). A run stops after two million variables or
two minutes.


## Archives — browsed like directories

Lets you walk into `.zip`, `.tar`, `.tar.gz`, `.tar.bz2`,
`.tar.xz`, `.tar.zst`, `.7z` and `.rar` archives as if they were folders — and
into `.deb` and `.rpm` packages, which are archives in their own right.

**Useful for** inspecting, extracting from, or adding to an archive without
unpacking it first, and for answering "what is actually *in* this package".

**Operation.** Press **Enter** on an archive file to browse it. Copy files
**out** (F5 to a normal panel) or **in** (F5 from a normal panel into the archive
panel); **F8** deletes from the archive. To build a new archive, tag a selection
and use *File menu → Compress…*, choosing the format by the name you type
(`.zip`, `.7z`, `.tar.gz`, `.tar.bz2`, `.tar.xz`, `.tar.zst`).

**Zstandard.** `.tar.zst` and `.tzst` are read and written like any other tar,
so an Arch package (`.pkg.tar.zst`, which is just a `.tar.zst`) opens with
Enter. A plain `.zst` — a single compressed file such as `syslog.1.zst` — opens
as a one-member listing holding the decompressed file, so F5 copies it out
already unpacked.

**Packages.** A `.deb` is shown as one tree: the files it installs at the root,
its control files under **`/DEBIAN`** (where `dpkg-deb -R` puts them), and
`debian-binary` beside them. The two halves of the package are compressed
independently, which is why the layout is flattened — a path inside Rat
Commander names one container, so a tarball inside the package could not be
stepped into. An `.rpm` shows its payload directly. Packages built by rpm 4.14
and later keep every file name, mode and size in the package header rather than
in the payload; both that shape and the older one are read.

**Read-only formats.** RAR archives, plain `.zst`, `.deb` and `.rpm` can be
browsed and extracted but not created or modified: no tool can create RAR; a
plain `.zst` holds a single stream with nowhere to put a second file; rebuilding
a `.deb` would mean regenerating `md5sums` and keeping the control fields
consistent; and an RPM's header signs its payload, so any rewrite invalidates
it. *Compress…* refuses those names, and a copy **into** such an archive is
refused when it starts. (RAR support is an optional build feature, on by
default.)


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


## Other things browsed like directories

Beyond archives, Rat Commander opens several kinds of file **as a directory**,
natively — no helper script, no external tool. Press `Enter` on one, and `..` at
the top steps back out to the file, exactly as it does for an archive.

All of these are **read-only**: copying *into* one is refused before any bytes
move, rather than failing part-way through.

- **Disc images (`.iso`)** — ISO 9660, including **Joliet** (so long, mixed-case
  names come through as they were written, not as `LONGNAME.TXT;1`) and
  **Rock Ridge** (real POSIX names, the executable bit, and symlinks). This
  replaces the `iso9660` extfs script and the `isoinfo` it needs; the rule for it
  is still in `rc.ext` for the rare image the built-in reader declines — a
  UDF-only one, say — which then falls through to it as before.
- **SQLite databases (`.db`, `.sqlite`, `.sqlite3`)** — tables and views as
  directories, rows as files showing `column = value`, plus a `_schema.sql` at
  the top holding the statements that would rebuild it. Rows are named by their
  `rowid`, or by the primary key for a `WITHOUT ROWID` table, or by position for
  a view. A table with more than a thousand rows is **paged** into directories
  rather than listed whole. A blob is described (`<blob, 200 bytes> a3f1…`)
  instead of being dumped into a text view. The database is opened read-only and
  *immutable*, so browsing one that another program is writing neither blocks it
  nor changes it.
- **JSON and TOML documents** — objects and tables as directories, arrays as
  numbered entries (zero-padded, so sorting by name sorts by index), and each
  scalar as a file holding its bare value. A setting buried six levels down is
  something you `cd` to and `F3`, and the `Find file` search will look through it
  like any other tree.

Each of these confirms what a file actually is before claiming it — by its
header, or by parsing it — so a `.db` that is not a database, or a `.json` that
does not parse, opens the way it always did instead of half-listing.

**Where this sits.** `Enter` resolves in this order: a real directory, then a
built-in archive, then one of the above, then an `rc.ext` rule, and finally the
image flasher, the default application, or simply running the file.


## Remote filesystems (SFTP / FTP / FTPS / SCP)

Mounts a remote server into a panel, so you browse and transfer
files over **SFTP** or **SCP** (SSH) or **FTP / FTPS** exactly like local files.

**Useful for** managing files on a server without a separate client — copy/move/
delete works transparently between local, remote and archive panels.

**Remote shell.** On an **SFTP/SCP** panel, the command line and **Ctrl-O** run on
the **remote host** over the same SSH connection — see *The console → On a remote
panel* above.

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

**FTPS** is FTP secured with TLS: the connection is switched to TLS (`AUTH TLS`)
before logging in, so the password and every listing and transfer are
encrypted, and the data connections resume the same TLS session, as servers
such as vsftpd require. It has the same form, PASV option and history as FTP
(its own history). The server's certificate is checked like this:

- A certificate that one of the **system's trusted authorities** vouches for,
  for the host name you connected to and in date, is accepted without a word.
- Any other — **self-signed**, from a private CA, expired, for another name —
  stops the connection and asks: *Untrusted certificate* shows why it isn't
  trusted, its subject, issuer, expiry and **SHA-256**. **Trust** pins that
  certificate for the server (`host:port`) in `ftps_known_hosts` in the
  configuration directory and connects; from then on that server connects
  without asking, as long as it presents the same certificate.
- If a pinned server presents a **different** certificate, the prompt turns into
  a red *Certificate changed* warning, with **Cancel** focused: it may simply
  have been renewed, or someone may be intercepting the connection. Trusting it
  replaces the pin.

Only explicit FTPS is supported; *implicit* FTPS (a TLS connection from the
first byte, usually on port 990) is not.

**SSH authentication** follows the same order `ssh` itself uses, stopping at the
first method the server accepts:

1. **The ssh-agent**, if one is running (`SSH_AUTH_SOCK`; on Windows the OpenSSH
   agent's named pipe). Every key it holds is offered.
2. **Key files** — the one named in the dialog's **Key file** field, or, when that
   is blank, `~/.ssh/id_ed25519`, `~/.ssh/id_ecdsa` and `~/.ssh/id_rsa` in that
   order. A `~/` prefix is expanded, and the path is remembered per server.
3. **The password** from the form.

This means a server with `PasswordAuthentication no` — the default on most cloud
images — connects normally. If a chosen key is **encrypted**, a passphrase prompt
appears *before* the connection is attempted; the passphrase is used for that one
attempt and never stored. When nothing works, the error names each method that
was tried, so you can tell "wrong key" from "wrong password".

SSH host keys are checked against `~/.ssh/known_hosts`: a matching key connects,
an **unknown** host is trusted and **recorded** on first use, and a **changed**
key is rejected as a possible machine-in-the-middle.

**`~/.ssh/config`.** The hosts your SSH config names are offered in the SFTP and
SCP connect forms' Host dropdown, after the recent servers — each shown as
`alias   user@hostname:port via jump   (ssh config)` — and in the command palette
as *SSH host* entries. Picking one fills in the **alias** as the host, and any
host typed into the form is looked up in the config too, so a connection is set
up the way `ssh alias` would set it up:

- **HostName** is where it connects (with `%h` tokens expanded); **User** and
  **Port** apply unless the form gives a user or a port other than the default,
  as a command-line option would override them for `ssh`.
- **IdentityFile** keys are tried, in order, instead of the default keys (unless
  the form names a key file); `~` and the `%h %p %r %u %d %n` tokens are
  expanded, and files that don't exist are skipped. **IdentitiesOnly yes** leaves
  the agent's keys out.
- **ProxyJump** routes the connection through one or more jump hosts, each looked
  up in the config for its own HostName, User, Port, IdentityFile and
  HostKeyAlias; every hop's host key is checked against `known_hosts` and every
  hop authenticates in turn (the password is only offered to the host itself).
  The jump connections stay open for as long as the panel's connection does.
- **HostKeyAlias** is the name a host's key is looked up and recorded under.
- **Include** files are read in place (relative to `~/.ssh`, wildcards allowed);
  the first value found for a setting wins, as in `ssh`.

Not supported: **ProxyCommand** (a host that needs one gets an error rather than
a connection that bypasses the proxy — use `ProxyJump` where you can), **Match**
blocks (skipped), and a different passphrase per key (one passphrase prompt
applies to every encrypted key on the route).

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
your shell session still alive. When the active panel is on an **SFTP/SCP**
server, that shell runs on the **remote host** over the same SSH connection (see
*The console → On a remote panel*).

On **Windows** this works differently (see the *Windows note* under *The
console* above): `Ctrl-O` opens a fresh interactive shell that you leave by
typing `exit`, and command-line commands run one at a time with the panels
suspended — there is no persistent behind-the-panels session.

### Which shell runs

On **Unix**, the command line and `Ctrl-O` run **`$SHELL`** (falling back to
`/bin/sh`). Commands typed at the command line run it *interactively*, so your
aliases and shell functions from `~/.bashrc` / `~/.zshrc` work there just as they
do at your normal prompt.

**Windows** has no `$SHELL`, and `%COMSPEC%` says `cmd.exe` however you started
`rc`. So Rat Commander looks **up the process tree** for the shell it was
launched from and uses that one — start `rc` from **PowerShell** or **pwsh** and
that is what `Ctrl-O` gives you; start it from Git-Bash and you get bash. Only
when no shell ancestor is found does it fall back to `%COMSPEC%`.

To pin a shell rather than inherit one, set **`shell`** in `config.toml`:

```toml
shell = "pwsh"
# or a full path, which is what to use when several versions are installed:
# shell = 'C:\Program Files\PowerShell\7\pwsh.exe'
# shell = "/usr/bin/fish"
```

The same setting is the **Shell** field in *Options → Settings… → Programs*.
Give a **program only**, not a command line — the arguments that run a command
(`-c`, `/C`, `-Command`) are added for you, chosen from the shell's name. A change
made in Settings applies to the next shell Rat Commander starts; the console shell
already running behind `Ctrl-O` and the command line keeps its program until you
`exit` it (an edit to `config.toml` itself needs a restart).

Commands Rat Commander composes itself — launching an external editor or viewer,
and the `%view` filters in `rc.ext` — keep running under plain `sh` on Unix,
since those are written in `sh` syntax; on Windows they use the shell above.

### Working without the command prompt

**Ctrl-F5** hides the command line altogether (the same switch as **Command
prompt** in *Options → Settings… → Programs*, and a toggle in the command
palette). The
panels take over its row, and the choice is remembered across sessions.

With it hidden, typing a printable character no longer enters text — it starts a
**quick search** on the active panel, seeded with that character, exactly as if
you had pressed `Alt-S` first. `Backspace` trims the query, `Esc` cancels, `Enter`
opens the match, and any other key leaves the search and does its usual job. The
selection keys keep their meaning: `+` and `-` still open the select / unselect
dialogs and `*` still inverts the selection.

The command-line-only keys (`Alt-Enter`, `Alt-P` / `Alt-N`, `Alt-Shift-H`) do
nothing while it is hidden. Everything that doesn't need the prompt still works:
**Ctrl-O** drops to the subshell, **F2** user-menu entries run their commands, and
`Enter` on a directory descends into it.


### Changing directory on exit

A program cannot change its parent shell's working directory, so `rc` does the
next best thing: **`rc --print-last-dir <FILE>`** (short form **`-P <FILE>`**)
writes the directory the active panel was showing when it quit into `<FILE>`,
and a shell function reads it and does the `cd` itself. This is the same trick
Midnight Commander's `mc -P` wrapper uses.

The packages install ready-made wrappers, which define **`rcd`**:

```sh
source /usr/share/rat-commander/rc.sh          # bash / zsh
source /usr/share/rat-commander/rc.fish        # fish
```

From a source checkout they are in `packaging/shell/`. Use `rcd` wherever you
would have typed `rc`; it passes every argument straight through, so
`rcd /edit notes.txt` still opens the editor, and it returns `rc`'s own exit
status.

The flag may appear anywhere on the command line and is removed before the rest
is interpreted, so `rc -P /tmp/dir /edit notes.txt` works.

Three cases can't hand back the panel's own path, and each falls back to
something a shell can actually enter:

| Active panel | What gets written |
|---|---|
| A local directory | that directory |
| Inside an archive | the directory *holding* the archive |
| A remote (SFTP/FTP/SCP) panel | that panel's last local directory |

If even the fallback no longer exists, the directory `rc` itself was started in
is written instead. The file is only written when the flag is given, so `rc`
started without it behaves exactly as before.

### When a file operation is refused

If the filesystem refuses a step because of **permissions**, the operation does
not fail outright — it stops on that one file and asks:

| Answer | What happens |
|---|---|
| **As root** | Retry this one step with elevated privileges |
| **Root all** | Retry this and every later refusal as root, without asking again |
| **Skip** | Leave this file alone and carry on with the rest |
| **Skip all** | Skip this and every later refusal |
| **Abort** | Give up on the whole operation |

The default is **Skip**, not the privileged answer. Everything the operation
*can* do still gets done, so copying a directory that contains one unreadable
file now copies everything else instead of stopping at it.

Choosing a root option asks for your **sudo password** once, in the same masked
prompt the disk manager uses. The password is used only to unlock `sudo`'s own
credential cache and is then discarded — it is never stored, and never passed to
the running operation. Each escalated step is performed by a small helper mode of
`rc` itself, invoked through `sudo`, which does exactly one primitive (copy one
file, delete one file, remove or create one directory) and exits; the directory
walking stays unprivileged. Files it creates for you are handed back to your own
user, so an escalated copy doesn't leave root-owned files behind.

Escalation is offered only where it could actually help — real files on local
disk. On a **remote panel** or inside an **archive**, `sudo` has no bearing on
what the server or the container permits, so only Skip and Abort are offered.

### Deleting and the trash

**F8** deletes the selection. When the trash is enabled (it is by default) and
everything selected is a real file on local disk, F8 **moves it to the trash**
instead of unlinking it, and the prompt says so. **Shift-F8** — or **Ctrl-F8**,
for terminals that don't report Shift-F8 — deletes **permanently**, and its
prompt is drawn in the red "danger" style so the two are never confused.

The trash is the freedesktop one your desktop environment already uses
(`~/.local/share/Trash`), written directly rather than by calling out to `gio`.
Files trashed by `rc` show up in GNOME Files, Dolphin, `gio trash --list` and
anything else that follows the specification, and can be restored from there.
Files on **another mount** (a USB stick, a separate `/home`) cannot be moved
across filesystems by a rename, so they go to a trash directory on their own
volume — `$topdir/.Trash/$uid` when the administrator has provided one, else
`.Trash-$uid` — exactly as the specification requires. A cross-filesystem trash
is a real copy, so it shows a progress window and can be aborted.

Trashing never applies to **remote panels** or the **inside of an archive**:
there is nowhere to move the file to, so F8 deletes outright there and says so.

The trash is a plain directory, so restoring by hand needs no special UI — the
command palette's **Go to Trash** entry points the panel at it, and **F6** moves
anything back out. To turn the trash off entirely, untick **Use trash bin** in
*Options → Settings… → Confirmations* (or toggle it in the palette), or set
`use_trash = false` in `config.toml`; F8 then deletes permanently as it always
did.

Trashing is a Linux/BSD feature. macOS's `~/.Trash` uses a different, undocumented
format that Finder would not be able to restore from, and Windows needs the
shell's recycle-bin API, so on both F8 keeps deleting permanently.

### Directory tabs

Each panel can hold several directories at once and switch between them, without
giving up the two-panel layout.

| Key | Action |
|---|---|
| **Ctrl-N** | Open a new tab, on the current directory |
| **Alt-K** | Close the current tab |
| **Alt-J** | List the panel's tabs and pick one |
| **Ctrl-PageDown** / **Ctrl-PageUp** | Next / previous tab |
| **Ctrl-Tab** / **Ctrl-Shift-Tab** | Next / previous tab, where the terminal allows it |
| Click a tab | Switch to it |

The command palette (**Ctrl-P**) has *New tab*, *Close tab* and *Next tab* too.

A tab strip appears along the top of a panel **only once it has more than one
tab**, so a single-tab panel looks exactly as it always did and gives up no room.
Each tab remembers its own directory, view format, sort order, listing filter,
marked files and cursor position — switching away and back puts you exactly where
you were, and the cursor is restored by *file name*, so it still finds its place
if the directory changed while you were elsewhere.

Closing the last remaining tab does nothing: a panel always shows a directory,
and quitting is **F10**'s job.

Tabs on **local** directories are saved and restored between runs, the same way
each panel's last directory is. A tab on a remote server or inside an archive is
not saved — that would need credentials the program deliberately doesn't keep.

The one-remote-panel rule applies to tabs as well: if one panel is already on a
remote connection, switching the other panel to a tab that sits on a remote
connection is refused, and says so.

Why these keys: **Ctrl-T** is Midnight Commander's "tag file", **Ctrl-W** is the
command line's delete-word-backwards, and **Alt-1**…**Alt-9** are the Esc-N
function-key aliases — all long-standing bindings that tabs do not take over.

**If Ctrl-Tab does nothing, that is your terminal, not `rc`.** Two separate
things get in the way, and neither is something a program can work around:

* Terminals without the **Kitty keyboard protocol** cannot express Ctrl-Tab at
  all — they send the same byte for Tab and Ctrl-Tab, so the two are literally
  indistinguishable by the time they arrive.
* Several terminals that *can* send it — **Konsole**, **GNOME Terminal**,
  **Tilix**, **Terminator** — bind Ctrl-Tab to switching their *own* tabs and
  never pass it on. Unbinding it in the terminal's own keyboard settings hands
  it back to `rc`.

**Ctrl-PageDown / Ctrl-PageUp** are the reliable equivalent: no terminal claims
them, and they are the same chord browsers and editors use. **Alt-J**'s picker
always works, including on a terminal that reports no modified keys at all.

## The user menu (F2)

**F2** opens a configurable **user menu** of shell commands. It is created with
sensible defaults on first run and uses the Midnight Commander `menu` file
format (see *Configuration* below). Each entry runs commands against the current
file, the current directory, the other panel or the tagged files via macros, and
can be shown or defaulted **conditionally** — only when files are tagged, for a
matching file type or name, and so on. Useful for one-key access to your own
scripts and recurring tasks.


## The command palette (Ctrl-P)

*Command menu → Command palette…*, or **Ctrl-P**.

A single fuzzy-search box over everything the program can do, so you can reach a
command by name instead of hunting through menus.

**Useful for** running any action, changing a setting, or jumping somewhere in a
couple of keystrokes — without remembering which menu it lives in.

**Operation.** Type to filter; the list narrows as you go, ranking the tightest
matches first and highlighting the letters that matched. `↑`/`↓` (or the mouse
wheel) move the selection, `PgUp`/`PgDn` jump a page, **Enter** (or a click) runs
the highlighted entry, and **Esc** (or **Ctrl-P** again) closes it. The query is
a *fuzzy* match — `cf` finds *Compare files*, `dspl` finds *Disk explorer* — and
typing a family name (`bookmark`, `connection`, …) narrows to that family.

**What it lists**, shown with a colour-coded tag on the right:

- **Command** — every menu action (view, copy, compare, the explorers, the
  connection forms, the panel view/sort options for the active panel, …), run
  exactly as if picked from the menu.
- **Setting** (tagged *Options*) — switch the **theme**, **language** or
  **graphics** mode directly, or flip a toggle (truecolor, animations, the status
  widget, RTL reshaping, the internal viewer/editor, and each confirmation) in
  place. A `✓` marks the active theme/language/mode; toggles show `✓`/`✗`. The
  change is applied and saved immediately.
- **Bookmark** — jump the active panel to a saved directory, or **add / remove**
  the current directory. Bookmarks are local directories, persisted in
  `config.toml` (the `bookmarks` list); the **directory hotlist** (Ctrl-\, below)
  manages the same list.
- **Connection** — switch the active panel to any **open** remote session, or
  **reconnect** to a **saved** server (opens the connection form prefilled from
  history, ready for the password).


## Git-aware panels

When a panel is showing a directory inside a **Git working tree**, Rat Commander
reads its VCS status in the background (so even a large repository stays
responsive) and surfaces it in the listing and on the border.

**Per-file status.** Each entry's leading marker is replaced by a coloured status
glyph when git has something to say about it:

- **`>`** — **modified** in the working tree (unstaged changes).
- **`+`** — **staged** in the index (ready to commit).
- **`?`** — **untracked** (not yet added to git).
- **`!`** — **conflict** (an unmerged path).

A subdirectory that *contains* changes is flagged with the strongest state found
beneath it, so you can see where changes live without descending. The file name
is tinted the same colour as its glyph.

**Branch on the border.** The bottom-left of the panel border shows the current
branch, e.g. `⎇ main ↑2 ↓1` — the `↑`/`↓` counts are how many commits the branch
is **ahead** / **behind** its upstream (shown only when non-zero).

**One-key actions** (they act on the active panel's cursor file, or the tagged
files if any):

- **`Ctrl-G`** — **stage / unstage**. A staged file is unstaged; anything else is
  staged (`git add` / `git restore --staged`). The glyphs refresh immediately.
- **`Alt-D`** — open a **side-by-side diff against `HEAD`** — the committed
  version on the left, your working copy on the right — in the same
  [Compare files](#compare-files-side-by-side-diff) view. Use `Ctrl-←` to bring a
  hunk over from `HEAD` (discarding a change) and **F2** to write the working file
  back. An untracked file diffs against an empty left side. (The `HEAD` side is
  read-only, so it can't be written back over anything.) From here you can also
  stage a single hunk — see below.

### Staging hunks

In the `Alt-D` diff, three keys act on the **hunk under the cursor**:

- **`s`** — **stage** it (`git apply --cached`). The rest of the file stays
  unstaged, so a change you want to commit on its own can be split out without
  leaving Rat Commander.
- **`x`** — **discard** it from the working file. This throws away an edit that
  was never committed and cannot be undone, so it asks first.
- **`u`** — **unstage the whole file** (`git restore --staged`).

The status line shows how many hunks the file has and which one you are in, and
the view refreshes itself after every Git action, so staging one hunk
immediately leaves the remaining ones on screen.

Two things are worth knowing about why it behaves this way:

- **The hunks are not the same as the `⇄` blocks.** The two panes show `HEAD`
  and your working file, while the hunks run from the **index** to the working
  file — the same thing `git add -p` would offer you. They coincide only while
  nothing is staged; when something is, the status line says so.
- **Unstaging is whole-file, not per-hunk.** Picking a staged hunk would mean
  putting a cursor on an *index* line, and no pane here shows one. Unstage the
  file and stage again, or use the Git menu.

Binary files have no hunks to stage, and neither does a file with nothing
unstaged; both say so rather than doing nothing.

Everything else lives in the [Git menu](#the-git-menu) below.

Git integration shells out to the `git` command; if `git` isn't installed, or the
directory isn't a repository, panels simply show no VCS info. Remote and archive
panels never show git status.


## The Git menu

**`Alt-G`**, or *File menu → Git* (it opens as a submenu; `→`/`Enter` opens it,
`←`/`Esc` steps back out). Every entry is also in the command palette — type
`git` to see them all.

The point is to cover the everyday git round trip without dropping to a shell,
and to *guide* rather than assume you remember the flags. Long-running commands
(fetch/pull/push/clone) run in the background, so the UI never freezes.

**Seeing what's going on.**

- **Status** (`S`) — `git status`, shown in a scrollable output box.
- **Log** (`L`) — the last 200 commits as a decorated graph, one line each.
- **Diff vs HEAD** (`D`, or **`Alt-D`**) — the side-by-side diff described above.

**Staging and committing** (these act on the tagged files, or the cursor file):

- **Add (stage)** (`A`) — `git add`.
- **Stage/unstage** (`G`, or **`Ctrl-G`**) — the toggle.
- **Unstage** (`U`) — `git restore --staged`.
- **Stash save…** (`V`) — `git stash push`: put the working tree aside and go
  back to a clean `HEAD`. The message is optional (git writes its own "WIP on
  *branch*" otherwise); tick **Include untracked files** to take along files git
  is not tracking yet, and **Keep the index staged** to leave what you have
  already staged in place.
- **Stashes…** (`E`) — the stash list, with the verbs on it: **Enter** shows a
  stash's diff, **`a`** applies it and keeps it, **`p`** pops it (applies and
  removes), **`d`** drops it — which discards it for good, so it asks first.
- **Remove…** (`M`) — `git rm`: drops the files from the index **and deletes them
  on disk**, so it asks first.
- **Restore (discard)…** (`T`) — `git restore`: throws away uncommitted edits.
  Git cannot undo this, so it asks first.
- **Commit…** (`C`) — a message field plus *stage all tracked changes* (`-a`) and
  *amend the last commit*. An empty message cancels rather than letting git
  reject it.

**Exchanging with the remote.**

- **Fetch…** (`F`) — with *all remotes* and *prune* options (*all* is pre-ticked
  when the repo has more than one remote).
- **Pull…** (`P`) — with a *rebase instead of merge* option.
- **Push…** (`H`) — pick the **remote** from a dropdown, and optionally *set
  upstream*, *force with lease*, or *force*. **Force with lease** is the safer of
  the two (it refuses to clobber work that reached the remote after your last
  fetch) and wins if you tick both.
- **Sync** (`Y`) — pull then push in one step. A failed pull skips the push.

**Moving around and undoing.**

- **Checkout…** (`K`) — the **branch dropdown** lists your local branches (the
  current one first) followed by the remote-tracking ones; picking `origin/x`
  checks out `x` as a tracking branch rather than detaching HEAD. Type a name in
  the second field instead to **create** that branch.
- **Reset…** (`R`) — pick the **mode** (*soft* keeps the index and working tree,
  *mixed* keeps the working tree, *hard* discards everything) and the target
  commit (default `HEAD`).

**Starting a repository.**

- **Init repository…** (`I`) — `git init` in the panel's directory (confirmed).
  Offered only when the directory isn't already in a work tree.
- **Clone…** (`N`) — a URL, and optionally the directory name to clone into
  (blank lets git derive it from the URL). Clones into the panel's directory.

**Output.** Anything git prints — a push summary, a merge conflict, an error —
appears verbatim in a large scrollable box: `↑`/`↓`/`PgUp`/`PgDn`/`Home`/`End`
and the mouse wheel scroll, `←`/`→` pan sideways (long `log --graph` lines are
not wrapped), and **Close** / `Esc` / `Enter` dismiss it. A command that
succeeds silently (like `add`) shows no box at all; a failure is titled as such
and coloured. Afterwards the panels and the status glyphs are re-read
automatically.

All of these need the active panel to be on a **local** directory — a remote or
in-archive panel has no work tree to act on.


## Browsing git history

*Git menu (`Alt-G`) → Browse a revision…, or the command palette*

Mounts the repository's **history into the panel**, so any past commit browses
like an ordinary directory.

**Useful for** getting a file back the way it was, reading a module as it stood
before a rewrite, or diffing today's working copy against a release from two
years ago — without a checkout, a stash, or a second clone.

**The mount's root is the list of commits**, newest first, each one a directory
named for when it was made:

```
2026-09-11_15-42-47_a9ef3a7_Made-3d-topography-more-static
^ date      ^ time   ^ commit  ^ its subject
```

The timestamp leads so that the panel's ordinary **sort by name is sort by
time**. Press `Enter` on one and you are inside that commit's tree; `..` steps
back to the commit list, and `..` again leaves history for the working tree you
started in. The panel's border shows which revision you are in, in place of the
branch it shows for a working directory.

Inside a revision **everything works the way it does anywhere else**, because
history is just another filesystem to Rat Commander: `F3` views a file as it was,
`F5` copies it out into the present, *Compare files* diffs it against the working
copy, and *Find file* searches it. Sizes, the executable bit and symlinks are all
as they were recorded.

**History is read-only.** Copying, moving, renaming or deleting *into* a revision
is refused up front rather than failing part-way through — the past is not
somewhere you can write. Submodules show as empty directories, since their
contents live in a repository of their own.

The commit list is capped (the most recent 500) so that opening history on a very
large repository is instant rather than a five-second pause.


### The time machine — watching a repository grow

*In a 3D-view panel: `t`*

The 3D view already draws a directory tree as a landscape. The time machine drags
that landscape **through the repository's history**: scrub back through the
commits and the shape re-forms as you go — directories swelling as they fill up,
whole branches of the tree growing out of their parent the moment they were first
committed, and fading away again as you scrub back past their creation.

Put a panel into **3D view** (`Alt-T`, or the Left/Right menu) with the *other*
panel on a git work tree, then press **`t`**.

| Key | |
| --- | --- |
| `t` | Turn the time machine on or off |
| `[` / `]` | One commit older / newer |
| `{` / `}` | Ten commits |
| `Shift-Home` / `Shift-End` | The first / last commit |
| Click the track | Seek to that point; the `◀`/`▶` ends step one commit |

A track under the scene shows where you are (`209/259`), and the panel's title
names the commit you are looking at. Turn it off and the panel returns to the
live filesystem.

**Sizes come from git**, which is why turning the time machine on usually makes
everything smaller: a work tree's `target/` or `node_modules/` was never
committed, so it is not there. Within history the sizes are consistent, and they
are the file sizes git recorded rather than blocks on disk.

**Scrubbing is cheap.** A commit's tree never changes, so one already visited is
redrawn from memory; holding a step key down coalesces into a single read once
the movement settles, and the commit you are heading for is fetched while you
look at the one you are on. Dragging across the whole track costs a handful of
reads, not one per commit.


## Directory history (back / forward)

Each panel remembers the directories it has visited, like a web browser's history.

- **Go back** with `Alt-←` (or Midnight-Commander's `Alt-y`) to return to the
  directory you were in before the current one; **go forward** with `Alt-→` (or
  `Alt-u`) to retrace a step you went back from.
- Two small arrows sit on each panel's top border: **`◀`** (back) at the
  **top-left** corner and **`▶`** (forward) at the **top-right** corner.
  **Click** them with the mouse to navigate. Each is drawn brightly when there is
  somewhere to go that way and **dimmed** when there is not.
- Any move that changes a panel's directory — pressing Enter on a folder, `..`,
  `cd`, a bookmark jump, a tree selection, switching drives or servers — is
  recorded. Making a new move after going back discards the forward trail (again,
  like a browser). Each panel keeps its own independent history.

**The history window (`Alt-H`)**, also on *Command menu → Directory history…*,
shows the whole trail as a list rather than making you step through it:

- Entries are in visit order — everything you can go **back** to, then the
  current directory (marked **`▶`** and highlighted), then anything you can go
  **forward** to.
- Move with `↑`/`↓`/`PgUp`/`PgDn`/`Home`/`End` or the wheel, **Enter** (or a
  click) to jump straight there, `Esc` to close. Picking the directory you are
  already in just closes the window.
- Jumping is an ordinary move, so it is recorded on the history itself.

(The **shell** history window — recent *commands* — is `Alt-Shift-H`; the two
share the letter.)


## Sending a directory to the other panel

Two shortcuts move the *other* panel without leaving this one:

- **`Alt-I`** — point the other panel at **this panel's directory**, so both show
  the same place. Handy right before a copy, a compare, or a sync.
- **`Alt-O`** — show the directory **under the cursor** on the other panel, and
  step the cursor down one entry. Holding `Alt-O` therefore walks down a list of
  folders while the other panel previews each in turn. With the cursor on a
  **file** (or in an empty listing) the other panel gets *this* directory
  instead, which is the useful reading of "show me where I am".

Both are also on the *Command menu*, and both record the move on the other
panel's history like any navigation.


## Auto-refreshing panels

A panel **re-reads itself** when something else changes the directory it is
showing — a command in the subshell, a build running in another window, a file
arriving over the network. `Ctrl-R` still forces a refresh, but you rarely need it.

**Useful for** watching a directory fill up without hammering `Ctrl-R`.

The refresh keeps its place: the cursor stays on the **same file by name**, and
marks survive (a mark on a file that has since gone is simply dropped). Changes
are **coalesced** — one `cp` or `git checkout` produces a burst of filesystem
events but only one re-listing, about a third of a second after things go quiet.

**What is not watched.** Only plain **local** directories. A **remote** (SFTP /
FTP / SCP) panel and the inside of an **archive** have nothing the operating
system can watch, and a panel showing **find-file results** is a list rather than
a directory — re-reading it would throw the results away. A listing only
re-reads for changes in the directory itself, not deeper down; the whole tree is
watched only while a 3D view or an Activity log on the other panel is drawing it.

**Turning it off.** Untick *Auto-refresh panels* in *Options → Settings… →
Panels* or toggle it in the command palette (`Ctrl-P`), or set
`auto_refresh = false` in `config.toml`. Worth doing on a
sluggish network mount or a directory with very many files, where the re-listing
costs more than it saves. On Linux each watched directory uses one inotify watch;
if your `fs.inotify.max_user_watches` is exhausted, watching quietly fails and the
panel simply behaves as it did before.


## The system clipboard

**Ctrl-Ins** (the Norton Commander convention), *File → Copy path to clipboard*,
or the command palette.

Puts a path — or a marked set of them, one per line — on the **system** clipboard,
so it can be pasted into a browser, a chat window or another terminal. The palette
also offers *Copy file name to clipboard* (the bare name, without directories) and
*Copy selected paths to clipboard*. In the **editor**, **Ctrl-C** and **Ctrl-X**
now put the marked block on the system clipboard as well as the editor's own.

**Useful for** getting a long path out of the file manager and into something
else without retyping it.

**How it works — and why it works over SSH.** Rather than talking to a clipboard
daemon, `rc` asks the *terminal* to set its clipboard, using the **OSC 52** escape
sequence. That means it needs no X or Wayland session on the machine `rc` is
running on: copying from a panel on a remote server lands the text on the
clipboard of the laptop in front of you. What is copied is what **F5** would act
on, so marks and find-file results behave exactly as they do for a file operation.

**Caveats worth knowing.**

- Your **terminal must support OSC 52**, and some ship with it off. There is no
  reply to the sequence, so `rc` cannot tell a terminal that took the text from
  one that ignored it — a copy that appears to do nothing is almost always this.
- Inside **tmux**, passthrough must be enabled: `set -g allow-passthrough on`
  (tmux 3.3+) in `tmux.conf`. `rc` wraps the sequence for tmux and for GNU screen
  automatically, but neither will forward it if configured not to.
- Very large copies are **refused rather than truncated** (the limit is 64 KiB),
  because a half-written sequence would silently replace your clipboard with a
  fragment. You get a message saying so.
- **Pasting** from the system clipboard is not offered: the same sequence can read
  the clipboard back, but terminals disable that by default — a remote host could
  otherwise help itself to whatever you last copied — so **Ctrl-V** in the editor
  keeps pasting from the editor's own clipboard. Use your terminal's own paste
  (usually **Shift-Insert** or **Ctrl-Shift-V**) to bring outside text in.


### Selecting with the mouse, without the trailing spaces

Dragging your terminal's *own* selection across the internal viewer or editor —
Shift-drag, in the terminals that reserve plain dragging for the program — used
to copy every line padded out to the window width, because a full-screen program
paints blanks where a line stops.

`rc` ends a partly-written line with an *erase to end of line* instead, exactly
as Midnight Commander's ncurses does. The cells past the text are then **unset**
rather than holding spaces, and terminals leave unset cells out of a selection:
you get `let x = 1;`, not `let x = 1;` followed by sixty spaces. Spaces *inside*
a line — indentation, aligned columns — are untouched.

Nothing changes on screen, because the erase paints in the current background
colour. Two cases keep their padding:

- a theme whose background behind the text is a **horizontal or diagonal
  gradient**, since there is no single colour to erase to (a vertical one is
  fine — each row is one colour);
- the rare terminal that erases to the *default* background instead of the
  current one, where the right-hand side of the editor would lose its colour.
  Turn the behaviour off there: *Strip trailing spaces on copy* in *Options →
  Settings… → Terminal* or the command palette (**Ctrl-P**), or
  `strip_trailing_spaces = false` in `config.toml`.


## The directory hotlist (Ctrl-\)

*Command menu → Directory hotlist…*, or **Ctrl-\**.

A list of **bookmarked directories** you can jump to instantly — the same
bookmarks the command palette shows and stores in `config.toml`.

- **Enter** (or a click) jumps the active panel to the highlighted directory.
- **`a`** or **Insert** adds the active panel's current directory to the list.
- **`d`** or **Delete** removes the highlighted entry.
- `↑`/`↓`, `PgUp`/`PgDn`, `Home`/`End` move the selection; **Esc** closes.

Edits (adds/removes) are saved to `config.toml` when the hotlist closes. Only
local directories can be bookmarked.


## The listing filter (Alt-I)

*Command menu → Panel filter…*, or **Alt-I**.

A **persistent** filter that hides files whose names don't match, until you clear
it — distinct from *quick search* (Alt-S / Ctrl-S), which only jumps the cursor
and never hides anything.

- Type a **shell glob** (`*.rs`, `img_??.png`) or **plain text** (matched
  anywhere in the name, case-insensitively — e.g. `report` shows every name
  containing "report"). Matching is case-insensitive.
- The active filter is shown in the panel's **title** as `[pattern]`, so it is
  obvious when entries are hidden. `..` is always kept so you can still navigate.
- The prompt is pre-filled with the current filter; submit an **empty** value to
  clear it. The filter is per panel and stays in effect as you browse and refresh
  (it is re-applied on each directory load). Marks on files the filter hides are
  preserved and reappear when the filter is cleared.


## 3D view

*`Alt-T` until the 3D mode is active, or Left / Right menu → 3D view*

A panel view format that draws a directory and its neighbourhood as a **tree of
boxes joined by lines**, seen in perspective.

**Useful for** taking in the shape of a directory tree — how deep it goes, how it
branches, and where the weight sits — without leaving the panel you are working
in.

**This view shows the contents of the *other* panel.** Like the Details and Tree formats, the 3D view
does not show its own directory: it shows wherever the **opposite** panel is.

**Boxes reflect their size on disk**.

**Operation.** The **arrows** move the selection between neighbouring boxes as
they appear on screen, and **Enter** points the *other* panel at the selected
directory — which, since the view follows that panel, is also what sends the
camera there. **Backspace** walks that panel back up. **Alt-arrows** orbit the camera,
**`+`/`-`** zoom, and **Home** returns to the default overview and framing. With the
**mouse**, click a box to select it, **drag with either button held** to turn the
scene, and use the **wheel** to zoom. Two levels of
contents are drawn without being asked for, so you can see what is inside a
subdirectory before deciding to go there.

**`t` turns on the time machine**, which scrubs the whole scene back through the
repository's history — see *Browsing git history → The time machine*.

**Directories light up as they are written into.** While the view is up, a
directory something writes into glows warm and then fades back over about a
second, so a burst of filesystem activity is visible as it happens: point one
panel at a build output directory, put the other in 3D, and you can watch a
compile fill it in. Activity **anywhere below** a box surfaces on that box — the
glow climbs to the nearest directory the scene actually draws — so work several
levels down is not silently lost.

This needs a **recursive** watch on the tree being shown, which costs one watch
descriptor per directory. On a very large or network-mounted tree that is worth
avoiding, so it can be switched off with **3D view: show filesystem activity**
(`Ctrl-P`, or Settings → Panels). If the system refuses the recursive watch — the
per-user inotify limit is the usual reason — the view quietly does without the
glow and the panel keeps its ordinary auto-refresh.

**Local directories only.** The sizes come from walking the real filesystem, so
the view has nothing to show while the other panel is on an archive, FTP or SFTP
directory, and `Alt-T` skips over the format on a remote panel.

### Styles (Settings → Panels → 3D style)

- **Cubes** (the default) — the tree described above: shaded boxes hanging in the
  panel's background, children fanned out on rings below their parent.

- **Spare no expense** — an homage to **fsn**, the 3D file system navigator that
  shipped with SGI's IRIX (and briefly starred in *Jurassic Park*). 

## Thumbnails

*`Alt-T` until the grid appears, or Left / Right menu → Thumbnails view*

A panel format showing the directory as a **grid of pictures**, each over its
name: **images** (`.png`, `.jpg`, `.gif`, `.bmp`, `.webp`, `.svg`) as thumbnails, **3D
models** (`.stl`, `.obj`) rendered the way the model viewer first shows them, and
everything else — directories included — by its type (a Nerd Font icon when
those are on, otherwise `DIR` or the extension).

**Useful for** finding a photo by what is in it rather than by
`IMG_20260912_141503.jpg`, or picking out a part in a folder of printable models.

**Operation.** It is an ordinary listing laid out differently: `↑ ↓` move a whole
row, `← →` one picture, `PgUp PgDn` a screenful, and `Enter`, `F3`, `F5`, marking
with `Insert`, the mouse and everything else work as in the other formats. The
cursor is the coloured plate behind a picture and its name.

**How the pictures appear.** They load in the background, a few at a time, for
the screenful you are looking at and the one after it, so scrolling on finds the
next page ready; a picture still loading shows `…`. A photo's embedded EXIF
thumbnail is used when it has one, so a large JPEG isn't decoded just to be
shrunk. On a terminal with a graphics protocol (Kitty / Sixel / iTerm2) they are
true pixels; elsewhere **half-block cell art**, or an ASCII ramp without 24-bit
colour. Pictures are kept for coming back to, within a memory budget, and are
let go when the panel leaves the format.

**Size.** *Options → Settings… → Panels → Thumbnail size*: **Small**, **Medium** or
**Large** cells.

**Limits.** On a remote (SFTP / FTP / SCP) panel or inside an archive, images
over 8 MB are shown by type rather than downloaded for a thumbnail (30 MB
locally), and models get thumbnails only from the local disk (up to 16 MB and
150,000 triangles).


## Activity log

*Left / Right menu → Activity log* (also in the command palette)

A panel format that lists, **live**, what changes anywhere under the directory
the **other** panel is in: files created, written, removed and renamed, however
deep. Newest at the top.

**Useful for** answering "what is this installer writing?", "which files does
this build touch?", or "is anything still being written to this disk?" — point
the other panel at the directory in question and watch.

**Reading it.** Each row is how long ago it happened, a mark, and the path
relative to the watched directory — the file name bright, the directories dim:

| Mark | Meaning |
| --- | --- |
| `+` | created (a directory ends in `/`) |
| `~` | modified |
| `✓` | written: a file opened for writing was closed, so it is complete |
| `-` | removed |
| `→` | renamed (`old → new`) |
| `↑` / `↓` | moved out of / into the watched tree |

Bursts are **folded**: a file written to four hundred times in a row reads as one
row with `×400` on the end, and a new file that is still being written keeps
saying it is new. The status line under the list shows the events per second
now and a sparkline of the last minute.

**Operation.**

| Key | |
| --- | --- |
| `↑ ↓` / `PgUp PgDn` / `Home End` (or the mouse) | Move through the rows |
| `Enter` (or a double-click) | Show the file in the other panel — its directory, with the cursor on it — and make that panel active. For a file that is gone, its directory. |
| `Insert` | Pause: new events are held back so the rows stand still to be read (the status line counts them); `Insert` again takes them all in |
| `Delete` | Clear the log |
| `Alt-Shift-I` | Filter the rows, like a listing filter: `*.rs`, or plain text matched anywhere in the path |

The log follows the other panel: point that panel somewhere else and it starts
again there. It works whether or not *Auto-refresh panels* is on.

**Limits.** Only **local** directories can be watched. It watches the whole tree
under the directory, which on Linux costs an inotify watch per subdirectory; on a
tree too big for the limit (`fs.inotify.max_user_watches`) the log says so and
falls back to the directory itself rather than failing silently. The operating
system reports *what* changed, not *which program* changed it.


## Disk explorer

*Command menu → Disk explorer…*

Draws a full-screen **treemap** of the current directory: each
box's area is proportional to a subdirectory's total on-disk size, labeled with
the name and a human-readable size.

**Useful for** finding what is using your disk space.

**It fills in while it scans.** The treemap is drawn from the first frame and is
navigable straight away — there is no progress bar to wait behind. The title
carries the running total and the number of directories read so far, and the boxes
grow as the crawl proceeds. The scan is **shared and cached**: descending into a
subdirectory that was already measured as part of its parent is instant, and so is
coming back up. Kernel filesystems (`/proc`, `/sys`, `/dev`, `/run`) are never
walked, and symlinks are never followed or counted.

**Operation.** Boxes that are large enough also show their **biggest files**
inside, each with its size, so you can spot space hogs without diving in. On a
terminal with graphics support (see *Terminal graphics*), the **whole treemap is
drawn as pixel "pillow" boxes**: each directory is a softly cushion-shaded box
**in its own hue**, subdivided into recessed, semi-transparent **sub-boxes** for
its largest files (sized by their share, with names labeled where they fit), so
every box reads as a little map of its own contents and much finer detail is
visible than with characters. It falls back to character-cell boxes on a plain terminal.

**The cursor reaches the files, not just the boxes.** **`←`**/**`→`** move
between boxes; **`↓`** steps *into* the selected box's list of biggest files and
walks down it, and **`↑`** walks back up and out onto the box again — so the
files a box advertises can be picked out directly, instead of vanishing the
moment you dive into the directory. The top bar names whatever the cursor is on:
the box's name, size and share of the total, or the selected file's path and
size. **`Del`** (or **`F8`**) deletes the selected file after a confirmation, and
the box shrinks and drops the row immediately — no rescan of the subtree.

**Enter** still dives into a subdirectory (whether or not a file is selected),
**Backspace** goes up, **`g`** (or **Ctrl-Enter**) exits and points the active
panel at the selected directory, **Esc** closes. With the **mouse**, click a box
to select it — or click one of its file rows/sub-boxes to select that file — and
**double-click** to dive in. Symlinks are never followed or counted.


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

**The cursor sleeps until you need it.** The explorer opens in **monitor mode**
with no row cursor, so the constantly re-sorting list can be watched from the
top — the busiest processes stay where they belong instead of a highlight
chasing one PID up and down. The first **`↑`**/**`↓`** (or `PgUp`/`PgDn`/`Home`/
`End`, or a tree key) reveals the cursor on the top row, and from then on it
sticks to its process across re-sorts. **`Esc`** then backs out one step at a
time: the first press puts the cursor away and returns to the top of the list,
the second closes the explorer. (**`F10`** and **`q`** close it either way.)

**`k`** kills the selected process, **`K`** force-kills it; both ask to confirm —
they need the cursor, so they do nothing in monitor mode.

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


## Screensaver

*Options → Settings… → Appearance → Screensaver* (off by default), or *Start screensaver* in
the command palette to see one now.

After a set time — 1 to 30 minutes — with no key pressed and the mouse left
alone, the screen gives way to an animation, the way Norton Commander's did. Any
key or mouse movement brings everything back exactly as it was.

- **Starfield** — Norton Commander's own: flying forward through stars that
  stream out of the middle of the screen, brightening as they pass.
- **Matrix** — streams of glyphs raining down, each with a bright head and a
  green trail fading behind it.
- **Clock** — the time, in large digits, drifting around the screen and changing
  colour each time it bounces off an edge. It is the one place the program shows
  **local** time rather than UTC.
- **Pipes** — pipes growing and turning across the screen until it fills up, then
  starting over.
- **Random** — a different one each time.

**Notes.**

- The key that wakes the screen does **nothing else**: an Esc, Enter or `q` that
  would abort a copy running behind a progress dialog is spent on waking up.
- If something needs an answer while the screensaver is up — a copy stopping to
  ask about a file that already exists, say — the screensaver makes way for the
  question, so it is on screen when you come back.
- Time spent in the subshell (`Ctrl-O`), a command's output or an external editor
  doesn't count as idle.
- It draws **text only**, advancing ten times a second, so it costs next to
  nothing — but over a slow remote connection even that is traffic nobody is
  watching, which is why it starts out turned off.


## Mounted volumes

The **Drive / connection picker** (**Alt-F1** / **Alt-F2**, or the panel menu's
**Drive…** entry) offers a button per **mounted volume** beside **Local**: disks
and NAS shares (NFS, CIFS, SSHFS, …) mounted under `/mnt` or `/media`, and
automounted disks, USB sticks and optical media under `/run/media` (on macOS,
`/Volumes`). Each button carries the mount point's last component — usually the
volume label — and picking one takes the panel straight to that directory.

The list is read fresh every time the picker opens, so a stick plugged in a
moment ago is already there. Long names are shortened with a `~`, and the buttons
wrap onto further rows so even a dozen mounts fit a narrow panel. Windows has no
such buttons — drive letters cover the same ground there (see below).


## Windows: drive letters

On Windows the **Drive / connection picker** (**Alt-F1** / **Alt-F2**, or the
panel menu's **Drive…** entry) shows the available **drive letters** on its first
row, with the current drive highlighted. Use the arrow keys or press a
drive-letter key to switch the panel to that drive. The **Local** button, any
open remote connections and the SFTP / FTP / SCP buttons appear below.

The command line and `Ctrl-O` shell also behave differently on Windows — see the
**Windows note** under [The console](#the-console-behind-the-panels): commands run
with the panels suspended (there is no persistent behind-the-panels console), and
`Ctrl-O` opens an interactive shell you leave by typing `exit`. That shell is the
one you launched `rc` from (PowerShell, `pwsh`, Git-Bash, …), or whatever
`shell` in `config.toml` names — see
[Which shell runs](#which-shell-runs).


## Configuration

Configuration files live in your platform config directory
(`~/.config/rat-commander/` on Linux):

- **`config.toml`** — written by the Settings dialog, which can change every
  setting described in this paragraph. Holds the active theme and
  language, the truecolor / animation / status-widget / `nerd_font` toggles, the external
  editor and viewer commands, the confirmation flags, the remembered remote
  servers (without passwords), and your directory **`bookmarks`** (used by the
  command palette, Ctrl-P). It also holds `command_history_max` (default
  `100`) — the maximum number of command-line entries kept in the persistent
  history; set it to `0` to disable history, `auto_refresh` (default `true`) —
  whether a panel re-reads itself when its directory changes on disk (see
  *Auto-refreshing panels*), `audio_display` (`"spectrogram"` by default, or
  `"waveform"`) — how audio files are drawn (see *Audio view*), `audio_autoplay`
  (default `false`) — whether the viewer starts an audio file playing when it
  opens it,
  `strip_trailing_spaces` (default `true`) — whether
  a line is ended with an erase rather than padded with blanks, so terminal
  selections copy no trailing whitespace (see *Selecting with the mouse, without
  the trailing spaces*), and `shell` (empty by default) — the
  shell program the command line and `Ctrl-O` run, overriding the detection
  described under *Which shell runs*. Finally it remembers the **session
  layout** — each panel's last directory, listing filter, visibility and
  half-height state, the split direction, and which side was active — and
  restores it on the next launch. The one exception is the initially-active
  panel, which opens at the current directory (where `rc` was launched) so you
  land where you were working; the other panel restores its saved directory (or
  the working directory if that directory is gone). An **`[editor_options]`**
  table holds the internal editor's settings, written by its Options → General
  dialog and by Options → Save setup (see *The editor*).
- **`history`** — the persistent command-line history, one command per line
  (recalled with `Alt-P` / `Alt-N` / `Alt-H`), trimmed to `command_history_max`.
- **`editor-positions.toml`** — the editor's cursor-position memory for the last
  50 files edited (see *Editor*).
- **`themes.toml`** — your editable themes (see *Themes*).
- **`lang/`** — the localization files, one TOML per language (see *Language*).
- **`templates/`** — the binary templates for the hex editor, the built-in ones
  and your own, and `.rc-manifest.toml`, which records what was deployed so
  upgrades refresh only the templates you haven't edited (see *Binary
  templates*).
- **`menu`** — the F2 user menu (see below).
- **`rc.ext`** — file associations for Open/View/Edit actions and extfs mounts
  (see *The rc.ext file format*).

### Settings (Options → Settings…)

The Settings dialog is split into **tabs**, shown along its top. Switch between
them with **Ctrl-PgUp** / **Ctrl-PgDn** from anywhere in the dialog, by clicking
a tab, or by moving the focus onto the tab row (**Shift-Tab** from a tab's first
field) and using **←/→** (**Home**/**End** jump to the first/last tab); **↓** or
**Tab** goes back down into the fields. **OK** saves the settings on every tab at
once, and **Esc** discards them all. The dialog reopens on the tab you last left
it on, until you quit.

Below the tab's settings, a few lines **describe whatever has the focus**: what
the setting does, what it costs, and when it is worth turning off. The text
follows the focus as you move, so tabbing through a tab reads you its options.

- **Appearance** — the **Theme**, **Animations**, **Nerd Font symbols**
  (per-file-type icons in the listing — see *Panels*; needs a Nerd Font in your
  terminal), the **System status widget**, and when the **Screensaver** starts
  and which **Screensaver style** it plays (see *Screensaver*).
- **Panels** — the number of **Brief view columns**, the **Thumbnail size** of
  the thumbnail grid (see *Thumbnails*), the **3D style**, the **Audio view**
  audio files open on (Spectrogram or Waveform — see *Audio view*), **Auto-play
  audio in the viewer** (F3 starts an audio file playing as it opens it; the
  Details view still waits for Play), and three switches for
  work done behind the listing: **Auto-refresh panels** (see *Auto-refreshing
  panels*), **3D view: show filesystem activity** and **Details view: git
  activity**.
- **Programs** — the **External editor** and **External viewer** commands (used
  instead of the built-in ones) and whether to **use the internal viewer/editor**
  anyway; the **Command prompt** (the shell line below the panels — also
  `Ctrl-F5`; see *Working without the command prompt*); the **Shell** program
  (blank detects it — see *Which shell runs*); and the **Command history size**,
  how many command lines are remembered (`0` turns the history off). Lowering it
  drops the oldest entries straight away. When the external editor field is left
  blank, `rc` falls back to the **`$VISUAL`** then **`$EDITOR`** environment
  variable; the external viewer likewise falls back to **`$PAGER`**. Only if none
  of those is set does the built-in tool run.
- **Confirmations** — which actions ask first, and **Use trash bin** (see
  *Confirmations* and *Deleting and the trash*).
- **Language** — the UI **Language** and **Reshape RTL text** (see *Language*).
- **Terminal** — the **Graphics** mode (see *Terminal graphics* below),
  **Truecolor (gradients)**, and **Strip trailing spaces on copy** (see
  *Selecting with the mouse, without the trailing spaces*).

Most of the switches are also in the command palette (`Ctrl-P`), which flips
one without opening the dialog.

The **Theme**, **Language** and **Graphics** fields are dropdowns: press
**Enter** to open the scrollable list, **↑/↓** (or the mouse wheel) to move
through it, **Enter** to pick, **Esc** to close. They **preview live** as you
move the highlight — the UI re-colors / re-translates / re-draws immediately — so
**Enter** keeps the highlighted one and **Esc** (closing the dialog) reverts to
what you started with. The **Nerd Font symbols** box previews live in the same
way: the listings behind the dialog switch markers as you tick it, and go back on
**Esc**. In every dialog the **OK** and **Cancel** buttons are part
of the keyboard focus ring: **Tab** / **↑↓** move onto them and **Enter** or
**Space** activates the highlighted one (**Enter** still submits from a field and
**Esc** always cancels).

### Terminal graphics

Where the terminal supports a graphics protocol, the **progress bars**, the
**process-explorer graphs** (CPU, per-core, memory, disk and network), the
file-transfer **speed graph**, the **disk-explorer treemap**, the editor's
**GeoJSON map** and the **dialog
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

The **3D style** setting chooses which of the two looks the panel's **3D view**
draws — **Cubes** or **Spare no expense**. See *3D view → Styles* above.

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

### Confirmations (Options → Settings… → Confirmations)

The **Confirmations** tab of Settings (the command palette's *Confirmations*
entry opens the dialog straight on it). Toggle which actions ask first:
**delete** (on), **overwrite** (on), **execute / open with default app** (off),
**unmount** (on) and **exit** (on). The same tab holds **Use trash bin** (on),
which decides whether F8 moves files to the trash (see *Deleting and the
trash*).

### Themes (Options → Edit themes…)

Rat Commander ships many themes — Dracula, Nord, Gruvbox, Solarized, Tokyo
Night, Catppuccin, One Dark and more — plus a classic Midnight Commander look,
Monochrome, Amber/Green CRT, and some playful ones.

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
`menu_selection_fg`, the per-type file colors (`dir_fg` for directories, `file_fg`
for regular files, plus `exec_fg` / `symlink_fg` / `archive_fg` / `doc_fg` /
`image_fg` / `media_fg` / `marked_fg`), the gradient endpoints, and so on. You can
also edit the file directly — open it with **F4** in a panel, and saving
live-reloads it. Delete the file to regenerate the presets. An older `themes.toml`
is upgraded in place on start: newly-added fields (such as `file_fg`) are filled
with sensible per-theme values, presets you have not touched pick up the
gradients they now ship with, and presets added since the file was written are
appended, so an existing install still gets new themes. A preset you have
recolored — and any theme of your own — is left exactly as it is. A preset you
*delete* stays deleted: the `known_presets` list at the top of the file records
what you have already been offered, so take a name off it to be offered that
preset again.

#### Per-element gradients

On a truecolor terminal any of these elements can fade between two colors
instead of painting one flat color:

| | |
|---|---|
| Panels | `panel_bg`, `panel_border`, `panel_border_active` |
| Cursor | `cursor_bg`, `cursor_inactive_bg` |
| Bars | `menubar_bg`, `fkey_label_bg` |
| Dialogs | `dialog_bg`, `dialog_border_fg`, `dialog_selection_bg` |
| Menus | `menu_bg`, `menu_selection_bg` |
| Controls | `input_bg`, `button_bg`, `button_focused_bg` |

The presets already use them. To change one, or to add a gradient to a theme of
your own, use the theme editor's indented **Gradient** rows, or write the table
yourself at the end of a `[[theme]]` block:

```toml
[theme.gradients.panel_bg]
to = "#001a80"          # the second endpoint
from = "#000040"        # optional; defaults to panel_bg itself
direction = "vertical"  # horizontal | vertical | diagonal | radial
animated = false        # off by default
```

Each element ramps across **its own** bounds, so every dialog, button and cursor
bar carries a whole gradient rather than a slice of one screen-wide one.
`animated` makes a ramp drift back and forth; it is off by default (a moving
background is distracting, a moving cursor or bar is not) and also follows the
global **Animation** setting, so switching animations off stills everything.

Removing a gradient (`Space` in the editor, or deleting its table) puts the
element back to its flat color.

Two caveats. Elements a theme paints in exactly the same color cannot be told
apart on screen and therefore share a gradient — the stock themes give the
cursor, the menu bar and the F-key bar one and the same teal, so give them
distinct colors to ramp them separately. And gradients need truecolor: on a
16/256-color terminal every element keeps its flat color.

The separate `gradient_from` / `gradient_to` pair is the theme's **accent**
ramp — the one the progress bars, graphs and the disk treemap fade through, and
the default look of the bars and the cursor until those elements are given a
gradient of their own.

### The F2 user-menu format

The `menu` file uses the Midnight Commander format. A line starting in column 0
is a menu entry whose first character is its hotkey; the indented lines below it
are the shell commands to run. Lines starting with `#` are comments.

```
# a comment
= t d
3      Compress the current subdirectory (tar.gz)
        Pwd=`basename %d`
        cd .. && tar cf - "$Pwd" | gzip -f9 > "$Pwd.tar.gz"
```

**Condition lines** may precede an entry: `+ <cond>` shows the entry only when the
condition is true, `= <cond>` marks the default (highlighted) entry, and `=+` /
`+=` do both — evaluated against the panels when you press F2. Sub-conditions,
combined **left-to-right** (no precedence): `f` / `F <pat>` the current /
other-panel file matches a pattern, `d` / `D <pat>` the current / other directory,
`t` / `T <types>` the file type, `x <path>` an executable exists, `!` negates,
`&` / `|` combine. Type chars: `n` not-dir, `r` regular, `d` dir, `l` link,
`c` / `b` char / block device, `f` fifo, `s` socket, `x` executable, `t` tagged.
A first line `shell_patterns=0` switches `f` / `d` patterns from shell globs to
regular expressions. For example, `+ ! t t` shows an entry only when nothing is
tagged; `+ t t` only when something is.

**Macros** expand before the command runs: `%f` / `%p` the current file, `%d` the
current directory, `%s` the tagged-or-current files, `%t` the tagged files; the
uppercase `%F` / `%P` / `%D` / `%S` / `%T` are the same for the *other* panel;
`%u` is the tagged files (untagged afterwards); `%x` the extension; `%%` a literal
percent. Value macros are **shell-quoted** by default, so write them **unquoted** —
`cat %f`, not `cat "%f"`; `%0f` turns quoting off and `%1f` forces it.
`%{Prompt text}` pops up a dialog and inserts what you type; `%view{…}` is accepted
(the command then runs in the foreground shell). Menu commands run in a **suspended
foreground shell**, so their output is visible and interactive `read` prompts work.

A `menu` written for an older version that wrapped macros in quotes (`"%f"`) is
**migrated automatically** on first load — the quotes are dropped and a one-time
`menu.bak` backup of the original is kept beside it.

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
