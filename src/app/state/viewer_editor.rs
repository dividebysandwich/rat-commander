//! Viewer/editor lifecycle, fetch-to-temp, and directory/file comparison.

use super::*;

impl AppState {
    /// If we remember where the cursor was last left in this (local) file, put it
    /// there and center it. Only local text files are keyed (a remote session's
    /// path isn't stable across runs); hex buffers restore nothing.
    pub(in crate::app::state) fn restore_editor_position(ed: &mut EditorState) {
        if ed.path.scheme != "file" || ed.is_hex() || ed.is_unnamed() || !ed.save_file_position() {
            return;
        }
        if let Some((line, col)) = crate::config::load_editor_position(&ed.path.display()) {
            ed.restore_position(line, col);
        }
    }

    /// Remember the current editor cursor position for the (local) file being
    /// edited, so re-opening it restores the cursor. Called as the editor closes.
    pub(in crate::app::state) fn record_editor_position(&self) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        if ed.path.scheme != "file" || ed.is_hex() || ed.is_unnamed() || !ed.save_file_position() {
            return;
        }
        let (line, col) = ed.cursor_line_col();
        crate::config::save_editor_position(&ed.path.display(), line, col);
    }

    /// Set a freshly built editor up from the saved options: wrap mode, tab and
    /// typing behaviour, and whether it gets a syntax highlighter. Every editor
    /// the app opens goes through this.
    pub(in crate::app::state) fn prepare_editor(&self, ed: &mut EditorState) {
        ed.set_options(self.config.editor_options.clone(), self.dark_ui());
    }

    /// Apply an [`EditorSignal`] (from a key or a mouse gesture): save, close,
    /// or raise the relevant modal dialog.
    pub(in crate::app::state) async fn apply_editor_signal(&mut self, signal: EditorSignal) {
        match signal {
            EditorSignal::Stay => {}
            EditorSignal::Close => self.close_editor().await,
            EditorSignal::Save { close_after } => {
                if self.editor.as_ref().is_some_and(|e| e.is_unnamed()) {
                    // No filename yet → go straight to "Save as" (nothing to
                    // confirm, since there's no named file to overwrite).
                    self.open_save_as(None);
                } else if close_after {
                    self.save_editor(true).await;
                } else if self.editor.as_ref().is_some_and(|e| e.confirm_before_saving()) {
                    let name = self.editor.as_ref().map(|e| e.name.clone()).unwrap_or_default();
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::save_editor(&name)));
                } else {
                    self.save_editor(false).await;
                }
            }
            EditorSignal::SaveAs => self.open_save_as(None),
            EditorSignal::ConfirmQuit => {
                let name = self.editor.as_ref().map(|e| e.name.clone()).unwrap_or_default();
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::editor_quit(&name)));
            }
            EditorSignal::OpenSearch => {
                self.dialog = Some(Dialog::SearchReplace(self.search_dialog(false)));
            }
            EditorSignal::OpenReplace => {
                self.dialog = Some(Dialog::SearchReplace(self.search_dialog(true)));
            }
            EditorSignal::NewFile => {
                // Starting a new buffer replaces this one, so unsaved work has to
                // be dealt with first — the same rule File → Open follows.
                if self.editor.as_ref().is_some_and(|e| e.dirty) {
                    self.show_error("Save or discard this file before starting a new one");
                } else {
                    self.record_editor_position();
                    self.open_new_editor();
                }
            }
            EditorSignal::Browse(kind) => self.open_editor_browser(kind),
            EditorSignal::OpenGotoLine => {
                let line = self.editor.as_ref().map(|e| e.cursor_line_col().0 + 1).unwrap_or(1);
                self.dialog = Some(Dialog::Input(InputDialog::new(
                    "Go to line",
                    "Line number",
                    line.to_string(),
                    InputPurpose::EditorGotoLine,
                )));
            }
            EditorSignal::OpenSortBlock => {
                self.dialog = Some(Dialog::Form(FormDialog::editor_sort()));
            }
            EditorSignal::OpenPasteOutput => {
                self.dialog = Some(Dialog::Input(InputDialog::new(
                    "Paste output of",
                    "Command",
                    "",
                    InputPurpose::EditorPasteOutput,
                )));
            }
            EditorSignal::OpenOptions => {
                let opts = self
                    .editor
                    .as_ref()
                    .map(|e| e.options().clone())
                    .unwrap_or_else(|| self.config.editor_options.clone());
                self.dialog = Some(Dialog::Form(FormDialog::editor_options(&opts)));
            }
            EditorSignal::SaveSetup => self.save_editor_setup(),
            EditorSignal::About => {
                self.dialog = Some(Dialog::Message(MessageDialog::info(
                    "About",
                    format!(
                        "Rat Commander {}\n\nInternal editor\n{}",
                        env!("CARGO_PKG_VERSION"),
                        env!("CARGO_PKG_REPOSITORY"),
                    ),
                )));
            }
            // The screen is repainted from scratch on the next frame.
            EditorSignal::RefreshScreen => self.force_clear = true,
            EditorSignal::OpenGeoMap => self.open_geo_map(),
            EditorSignal::ShowJwt(text) => {
                self.dialog = Some(Dialog::GitOutput(GitOutputDialog::plain("JWT", &text)));
            }
            EditorSignal::OpenTemplatePicker => self.open_template_picker(),
            EditorSignal::OpenTagKeyPicker => self.open_tag_key_picker(),
            EditorSignal::EditTemplate { path, line } => self.open_template_editor(path, line),
            EditorSignal::NewTemplate => {
                let stem = self
                    .editor
                    .as_ref()
                    .map(|e| {
                        let p = std::path::Path::new(&e.name);
                        p.extension()
                            .map(|x| x.to_string_lossy().to_uppercase())
                            .unwrap_or_else(|| e.name.clone())
                    })
                    .unwrap_or_default();
                self.dialog = Some(Dialog::Input(InputDialog::new(
                    "New template",
                    "Template name",
                    stem,
                    InputPurpose::EditorNewTemplate,
                )));
            }
        }
    }

    /// Close the editor: back to the one it was opened over (a hex editor
    /// whose template was being edited), or to the panels.
    pub(in crate::app::state) async fn close_editor(&mut self) {
        self.record_editor_position();
        // The hex inspector stays as it was left for the next file opened.
        if let Some(o) = self.editor.as_ref().map(|e| e.options()) {
            self.config.editor_options.hex_inspector = o.hex_inspector;
            self.config.editor_options.hex_inspector_big_endian = o.hex_inspector_big_endian;
        }
        let closed = self.editor.take().map(|e| e.path.path.clone());
        if let Some(mut under) = self.editor_stack.pop() {
            if let Some(path) = closed {
                under.resume_after_edit(&path);
            }
            self.editor = Some(under);
            return;
        }
        self.reload_all().await;
    }

    /// The hex editor's template picker: every template, the ones that fit the
    /// file first.
    fn open_template_picker(&mut self) {
        let Some(ed) = self.editor.as_mut() else { return };
        let head = ed.file_head();
        let name = ed.name.clone();
        let current = ed.active_template().map(|i| i.file_name);
        let templates = crate::bt::library::discover();
        let fitting = crate::bt::library::rank(&templates, &name, &head);
        self.dialog = Some(Dialog::TemplatePicker(Box::new(TemplatePickerDialog::new(
            &templates, &fitting, current,
        ))));
    }

    /// Open binary template `path` in a text editor over the current (hex)
    /// editor, which comes back when this one closes — at `line` when given
    /// (where the last run stopped).
    pub(in crate::app::state) fn open_template_editor(
        &mut self,
        path: std::path::PathBuf,
        line: Option<usize>,
    ) {
        match std::fs::read(&path) {
            Ok(data) => {
                let vpath = VfsPath::local(&path);
                let name = vpath.file_name();
                let mut ed = EditorState::new(name, vpath, &String::from_utf8_lossy(&data));
                self.prepare_editor(&mut ed);
                match line {
                    Some(l) => ed.restore_position(l.saturating_sub(1), 0),
                    None => Self::restore_editor_position(&mut ed),
                }
                if let Some(under) = self.editor.take() {
                    self.editor_stack.push(under);
                }
                self.editor = Some(ed);
            }
            Err(e) => self.show_error(format!("Cannot open the template: {e}")),
        }
    }

    /// Start a new binary template named `name` for the hex editor's file —
    /// its header filled in from the file — use it, and open it for editing.
    pub(in crate::app::state) fn create_template(&mut self, name: String) {
        let Some(dir) = crate::bt::library::user_dir() else {
            return self.show_error("No config directory available");
        };
        let Some(ed) = self.editor.as_mut() else { return };
        let head = ed.file_head();
        let file_name = ed.name.clone();
        match crate::bt::library::create_user_template(&dir, &name, &file_name, &head) {
            Ok(path) => {
                crate::bt::library::invalidate();
                let data = std::fs::read(&path).unwrap_or_default();
                let fname =
                    path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                let mut info = crate::bt::header::parse_header(&fname, &data);
                info.path = Some(path.clone());
                info.origin = crate::bt::header::Origin::User;
                ed.set_template(Some(info));
                self.open_template_editor(path, None);
            }
            Err(e) => self.show_error(format!("Cannot create the template: {e}")),
        }
    }

    /// Open the editor's file browser for one of the File menu's actions,
    /// starting in the edited file's own directory.
    fn open_editor_browser(&mut self, kind: crate::editor::BrowseKind) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        let cwd = || std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let dir = if ed.path.scheme == "file" {
            ed.path.path.parent().map(|p| p.to_path_buf()).unwrap_or_else(cwd)
        } else {
            cwd()
        };
        // Only "copy to file" writes, and it needs a name to start from.
        let name = match kind {
            crate::editor::BrowseKind::CopyTo => ed.name.clone(),
            _ => String::new(),
        };
        self.dialog = Some(Dialog::SaveAs(SaveAsDialog::browse(kind, dir, name)));
    }

    /// Carry out a path picked in the editor's file browser.
    pub(in crate::app::state) async fn editor_browsed(
        &mut self,
        kind: crate::editor::BrowseKind,
        path: std::path::PathBuf,
    ) {
        use crate::editor::BrowseKind;
        match kind {
            BrowseKind::Open => {
                // A modified buffer would be lost — make the user save or discard
                // it first (the same rule File → New follows).
                if self.editor.as_ref().is_some_and(|e| e.dirty) {
                    return self.show_error("Save or discard this file before opening another");
                }
                match tokio::fs::read(&path).await {
                    Ok(data) => {
                        let name = crate::vfs::VfsPath::local(&path).file_name();
                        let text = String::from_utf8_lossy(&data).into_owned();
                        if let Some(ed) = self.editor.as_mut() {
                            ed.load_text(name, crate::vfs::VfsPath::local(&path), &text);
                            Self::restore_editor_position(ed);
                        }
                    }
                    Err(e) => self.show_error(format!("Cannot open file: {e}")),
                }
            }
            BrowseKind::Insert => match tokio::fs::read(&path).await {
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if let Some(ed) = self.editor.as_mut() {
                        ed.insert_at_cursor(&text);
                    }
                }
                Err(e) => self.show_error(format!("Cannot read file: {e}")),
            },
            BrowseKind::CopyTo => {
                let Some(text) = self.editor.as_ref().map(|e| e.block_or_all()) else {
                    return;
                };
                match tokio::fs::write(&path, text.as_bytes()).await {
                    Ok(()) => self.reload_all().await,
                    Err(e) => self.show_error(format!("Cannot write file: {e}")),
                }
            }
        }
    }

    /// Run `cmd` through the user's shell and insert its output at the editor's
    /// cursor (Format → Paste output of…).
    pub(in crate::app::state) async fn editor_paste_output(&mut self, cmd: String) {
        // Not an interactive shell: it would take the terminal's foreground
        // process group and stop the program while the editor is still drawn.
        // See `shell::capture_argv`.
        let out = crate::shell::capture_command(&cmd).output().await;
        match out {
            Ok(o) => {
                // Failures still have something to say — paste stderr so the user
                // sees why nothing came back, rather than a silent no-op.
                let bytes =
                    if o.stdout.is_empty() && !o.status.success() { &o.stderr } else { &o.stdout };
                let text = String::from_utf8_lossy(bytes).into_owned();
                if text.is_empty() {
                    return self.show_error(format!("{cmd}: no output"));
                }
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert_at_cursor(&text);
                }
            }
            Err(e) => self.show_error(format!("Cannot run {cmd}: {e}")),
        }
    }

    /// Apply new editor options: to the open editor at once, and to the config
    /// so the next file opens with them too.
    pub(in crate::app::state) fn apply_editor_options(
        &mut self,
        mut opts: crate::config::EditorOptions,
    ) {
        // The dialog doesn't show the hex inspector's settings: keep them.
        let current = self.editor.as_ref().map_or(&self.config.editor_options, |e| e.options());
        opts.hex_inspector = current.hex_inspector;
        opts.hex_inspector_big_endian = current.hex_inspector_big_endian;
        self.config.editor_options = opts.clone();
        let dark = self.dark_ui();
        if let Some(ed) = self.editor.as_mut() {
            ed.set_options(opts, dark);
        }
        if let Err(e) = self.config.save() {
            self.show_error(format!("Could not save settings: {e}"));
        }
    }

    /// Options → Save setup: write the editor's current options to the config.
    fn save_editor_setup(&mut self) {
        if let Some(ed) = self.editor.as_ref() {
            self.config.editor_options = ed.options().clone();
        }
        match self.config.save() {
            Ok(()) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.set_status("Setup saved");
                }
            }
            Err(e) => self.show_error(format!("Could not save settings: {e}")),
        }
    }

    /// Apply a [`ViewerSignal`] (from a key or a mouse gesture).
    pub(in crate::app::state) async fn apply_viewer_signal(&mut self, sig: ViewerSignal) {
        match sig {
            ViewerSignal::Stay => {}
            ViewerSignal::Close => {
                self.viewer = None;
                // A blame still running has no one left to show it to.
                if let Some(task) = self.blame_task.take() {
                    task.abort();
                }
            }
            ViewerSignal::OpenGoto => {
                self.dialog = Some(Dialog::Goto(GotoDialog::new()));
            }
            // The viewer uses the editor's search dialog, so both offer the same
            // modes and options (Replace is meaningless on a read-only view).
            ViewerSignal::OpenSearch => {
                self.dialog = Some(Dialog::SearchReplace(self.search_dialog(false)));
            }
            // F1 opens the manual, replacing the current viewer — the "Help"
            // label on the viewer's F-key bar now does what it says.
            ViewerSignal::OpenHelp => self.open_help(),
            ViewerSignal::StartBlame => self.start_blame(),
            ViewerSignal::OpenBlameCommit => self.open_blame_commit().await,
        }
    }

    /// F5 on the tag page: choose a tag to add.
    ///
    /// Only the keys this file's tag format can actually hold are offered, and
    /// only the ones it does not hold already — so the list is what can be
    /// added rather than everything that exists.
    fn open_tag_key_picker(&mut self) {
        let Some(ed) = self.editor.as_ref() else { return };
        let keys = ed.addable_tag_keys();
        let dialog = crate::ui::dialog::TagKeyDialog::new(keys);
        if dialog.is_empty() {
            self.show_error("This file already has every tag its format can hold");
            return;
        }
        self.dialog = Some(Dialog::TagKey(Box::new(dialog)));
    }

    /// `b` in the viewer: blame the file in the background. The viewer shows
    /// that it is waiting, and takes the result only while it still is.
    /// Alt-M in the editor: open the GeoJSON map, and read the text for it in
    /// the background — the world map too, the first time — so a large file
    /// shows the dialog at once and the map when it is ready.
    fn open_geo_map(&mut self) {
        let Some(ed) = self.editor.as_ref() else { return };
        let (rope, cursor, name) = (ed.text_snapshot(), ed.cursor_byte(), ed.name.clone());
        self.geo_gen = self.geo_gen.wrapping_add(1);
        let generation = self.geo_gen;
        let dialog = GeoMapDialog::loading(name, generation, cursor, rope.clone());
        self.dialog = Some(Dialog::GeoMap(Box::new(dialog)));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let doc = tokio::task::spawn_blocking(move || {
                let _ = crate::geo::world::world();
                crate::geo::geojson::extract(&rope.to_string())
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(AppEvent::GeoJsonRead { generation, doc: Box::new(doc) }).await;
        });
    }

    /// The GeoJSON for the map dialog has been read: show it — or, when there
    /// is none, the empty map to draw the first feature on — unless the dialog
    /// it was for has gone.
    pub(in crate::app::state) fn apply_geojson(
        &mut self,
        generation: u64,
        doc: crate::geo::geojson::GeoDoc,
    ) {
        let Some(Dialog::GeoMap(d)) = self.dialog.as_mut() else { return };
        if d.awaits(generation) {
            d.set_doc(doc);
        }
    }

    /// Make the edits the GeoJSON map has made in the editor's text.
    pub(in crate::app::state) fn apply_geo_edits(&mut self) {
        let Some(Dialog::GeoMap(d)) = self.dialog.as_mut() else { return };
        let edits = d.take_edits();
        if let Some(ed) = self.editor.as_mut() {
            edits.into_iter().for_each(|e| ed.apply_map_edit(e));
        }
    }

    fn start_blame(&mut self) {
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(path) = v.local_path().map(Path::to_path_buf) else { return };
        if let Some(task) = self.blame_task.take() {
            task.abort();
        }
        self.blame_gen = self.blame_gen.wrapping_add(1);
        let generation = self.blame_gen;
        v.begin_blame(generation);
        let tx = self.tx.clone();
        self.blame_task = Some(tokio::spawn(async move {
            let result = crate::git::blame::blame(&path).await.map(Box::new);
            let _ = tx.send(AppEvent::BlameLoaded { generation, result }).await;
        }));
    }

    pub(in crate::app::state) fn apply_blame(
        &mut self,
        generation: u64,
        result: Result<Box<crate::git::blame::Blame>, String>,
    ) {
        let Some(v) = self.viewer.as_mut().filter(|v| v.awaits_blame(generation)) else {
            return;
        };
        self.blame_task = None;
        match result {
            Ok(blame) => v.set_blame(*blame),
            Err(e) => {
                v.cancel_blame();
                self.show_error(e);
            }
        }
    }

    /// Enter on a blamed line: close the viewer and walk the active panel into
    /// the repository's history, to the directory holding the file as it was in
    /// that line's commit, with the file under the cursor — ready for F3, or for
    /// Compare files against today's copy.
    async fn open_blame_commit(&mut self) {
        let Some((toplevel, oid, path)) = self.viewer.as_ref().and_then(|v| v.blame_target())
        else {
            return;
        };
        let rev = match crate::vfs::git::resolve_rev(&toplevel, &oid).await {
            Ok(rev) => rev,
            Err(e) => return self.show_error(format!("Cannot open revision: {e}")),
        };
        let rel = Path::new(&path);
        let name = rel.file_name().map(|n| n.to_string_lossy().into_owned());
        let mut inner = PathBuf::from("/").join(rev.component());
        if let Some(parent) = rel.parent() {
            inner.push(parent);
        }
        let target = VfsPath::git(crate::vfs::git::container_for(&toplevel), inner);
        let backend = match self.registry.resolve(&target) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        self.viewer = None;
        let side = self.active;
        self.panels[side].try_enter(target, backend, name.as_deref()).await;
    }

    /// F3: view the file under the cursor (internal viewer or external pager).
    pub(in crate::app::state) async fn open_view(&mut self) -> Flow {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind.is_dir() {
            return Flow::Continue;
        }
        let name = e.name.clone();
        let size = e.size;
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();
        if self.refuse_lossy(std::slice::from_ref(&path)) {
            return Flow::Continue;
        }

        // An rc.ext `View` rule takes precedence (MC behaviour): pipe a command's
        // output into the viewer, or run it in the foreground.
        if let Some(flow) = self.try_ext_view(&name) {
            return flow;
        }

        if !self.config.wants_internal_viewer() {
            return Flow::RunExternal {
                program: self.config.external_viewer().unwrap_or_default(),
                path: path.path,
            };
        }

        // A shapefile is binary, so what the viewer shows is the GeoJSON it
        // becomes — the same text F4 would edit, and the same thing F3 shows
        // for a .geojson. The map itself is an editor view (Alt-M), so the
        // footer says so rather than leaving it to be guessed at.
        if path.scheme == "file" && crate::geo::shapefile::is_shapefile_name(&name) {
            match crate::geo::shapefile::open(std::path::Path::new(&path.path)) {
                Ok(opened) => {
                    let mut v = ViewerState::new(name, opened.text.into_bytes());
                    v.set_local_path(path.path.clone());
                    v.enable_syntax(self.dark_ui());
                    self.viewer = Some(v);
                    if let Some(w) = opened.warning {
                        self.show_error(w);
                    }
                    return Flow::Continue;
                }
                Err(e) => {
                    self.show_error(e);
                    return Flow::Continue;
                }
            }
        }

        if path.scheme == "file" {
            // Local: page straight from disk — never load the whole file. The
            // line-index scan runs off-thread so it doesn't block the reactor.
            let local = path.path.clone();
            let dark = self.dark_ui();
            let scanned =
                tokio::task::spawn_blocking(move || crate::viewer::scan_file(&local)).await;
            match scanned {
                Ok(Ok((file, len, line_starts, scanned))) => {
                    let mut v =
                        ViewerState::from_scanned(name, file, len, line_starts, scanned, None);
                    v.set_local_path(path.path.clone());
                    v.enable_syntax(dark);
                    v.set_search_seed(self.search_memory.viewer_query.clone());
                    // A find-file content hit opens the viewer at its matching
                    // line, so F3 on a result lands where the search matched.
                    let hit = self.find_hit_lines.get(&path.display()).copied();
                    if let Some(line) = hit {
                        v.goto_hit_line(line as usize);
                    }
                    // An executable or a library opens in Binary mode — except
                    // when F3 came from a content search, whose hit is in the
                    // text. F4 reaches Binary mode from there.
                    if hit.is_none() {
                        open_binary(&mut v, &path.path).await;
                        open_certs(&mut v, &path.path).await;
                        // A document (Word, PDF, a spreadsheet…) opens on what
                        // it reads as; F8 switches to the bytes.
                        self.attach_doc(&mut v, &path.path).await;
                    }
                    // A supported image opens showing the decoded image fullscreen
                    // (it falls back to the raw text/hex view if it can't decode).
                    if crate::util::img::is_image_name(&v.name)
                        && let Some(iv) = load_view_image(&path.path).await
                    {
                        v.set_image(iv);
                    }
                    // A model file opens showing the mesh, orbitable, on the
                    // same terms: it falls back to the raw view when it doesn't
                    // parse, and F8 switches to the bytes.
                    if crate::mesh::is_model_name(&v.name)
                        && let Some(mv) = load_view_model(&path.path).await
                    {
                        v.set_model(mv);
                    }
                    // An audio file opens on its spectrogram (or waveform),
                    // ready to play (or playing, with auto-play on); F8
                    // switches to the bytes.
                    if let Some(av) =
                        load_view_audio(&path.path, &v.name, &self.config, self.audio_out.clone())
                            .await
                    {
                        v.set_audio(av);
                    }
                    self.viewer = Some(v);
                }
                Ok(Err(e)) => self.show_error(format!("Cannot open file: {e}")),
                Err(_) => self.show_error("Viewer failed to open file"),
            }
        } else {
            // Remote/archive: stream to a temp file with a cancellable progress
            // bar; the viewer then pages from that temp copy.
            self.start_fetch(FetchKind::View, name, path, backend, size);
        }
        Flow::Continue
    }

    /// F4: edit the file under the cursor with the internal editor (or a
    /// configured external editor).
    pub(in crate::app::state) async fn open_edit(&mut self) -> Flow {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind.is_dir() {
            return Flow::Continue;
        }
        let name = e.name.clone();
        let size = e.size;
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();
        if self.refuse_lossy(std::slice::from_ref(&path)) {
            return Flow::Continue;
        }

        // An rc.ext `Edit` rule takes precedence (MC behaviour): run its command
        // in the foreground instead of the built-in editor.
        if let Some(flow) = self.try_ext_edit() {
            return flow;
        }

        if !self.config.wants_internal_editor() {
            return Flow::RunExternal {
                program: self.config.external_editor().unwrap_or_default(),
                path: path.path,
            };
        }

        self.edit_existing_file(name, path, backend, size).await;
        Flow::Continue
    }

    /// Load an existing file into the internal editor: in-place hex when it is
    /// too big to hold as text, straight off the disk when it is local, and via
    /// a cancellable fetch-to-temp when it lives on a remote or in an archive.
    async fn edit_existing_file(
        &mut self,
        name: String,
        path: VfsPath,
        backend: std::sync::Arc<dyn Vfs>,
        size: u64,
    ) {
        let local = path.scheme == "file";
        // A shapefile is a set of binary files. It opens as the GeoJSON it
        // becomes, so the map editor (Alt-M) works on it exactly as it does on
        // a .geojson, and a save turns it back into the set.
        if local && crate::geo::shapefile::is_shapefile_name(&name) {
            match EditorState::new_shapefile(name.clone(), path.clone()) {
                Ok((mut ed, warning)) => {
                    self.prepare_editor(&mut ed);
                    self.editor = Some(ed);
                    if let Some(w) = warning {
                        self.show_error(w);
                    }
                    return;
                }
                Err(e) => {
                    self.show_error(e);
                    return;
                }
            }
        }
        // An audio file is binary — opening it as text would show nonsense, and
        // as bytes would show a haystack — so it opens on its tags, with the
        // bytes one Alt-T away. A file whose tags cannot be read falls through
        // and opens the way it always did.
        if local
            && crate::tags::is_taggable_name(&name)
            && let Some(mut ed) = EditorState::new_tags(name.clone(), path.clone())
        {
            self.prepare_editor(&mut ed);
            self.editor = Some(ed);
            return;
        }
        // Local files too big to load as text open directly in (in-place) hex mode.
        if local && size > crate::editor::MAX_TEXT_EDIT {
            match EditorState::new_hex(name, path) {
                Ok(mut ed) => {
                    self.prepare_editor(&mut ed);
                    self.editor = Some(ed);
                }
                Err(e) => self.show_error(format!("Cannot open file: {e}")),
            }
            return;
        }
        if local {
            match load_file(&backend, &path).await {
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    let mut ed = EditorState::new(name, path, &text);
                    self.prepare_editor(&mut ed);
                    Self::restore_editor_position(&mut ed);
                    self.editor = Some(ed);
                }
                Err(e) => self.show_error(format!("Cannot open file: {e}")),
            }
            return;
        }
        // Remote/archive: in-place hex editing isn't possible (no random write),
        // so editing requires loading into memory — cap the size and stream the
        // download with a cancellable progress bar.
        if size > crate::editor::MAX_TEXT_EDIT {
            self.show_error("File too large to edit over this connection");
            return;
        }
        self.start_fetch(FetchKind::Edit, name, path, backend, size);
    }

    /// Shift-F4: open the editor on `name`, resolved against the active panel's
    /// directory. The file usually does not exist yet — the buffer starts empty
    /// and the file is created by the first save — but a name that is already
    /// taken opens that file with its contents, exactly as F4 would.
    ///
    /// Unlike F4 this consults no `rc.ext` `Edit` rule: those expand macros from
    /// the *cursor* entry, which is not the file being opened here.
    pub(in crate::app::state) async fn open_new_file_editor(&mut self, name: String) -> Flow {
        let p = &self.panels[self.active];
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();
        if self.refuse_lossy(std::slice::from_ref(&path)) {
            return Flow::Continue;
        }
        // A backend that cannot be written to would only fail at save time, with
        // the typing already done — say so before the editor opens instead.
        if !backend.capabilities().writable {
            self.show_error("This filesystem is read-only");
            return Flow::Continue;
        }
        let file_name = path.file_name();
        // An existing name is opened rather than shadowed by an empty buffer that
        // would overwrite it on save; a directory cannot be edited at all.
        let existing = backend.stat(&path).await.ok();
        if existing.as_ref().is_some_and(|e| e.kind.is_dir()) {
            self.show_error(format!("{file_name} is a directory"));
            return Flow::Continue;
        }

        if !self.config.wants_internal_editor() {
            return Flow::RunExternal {
                program: self.config.external_editor().unwrap_or_default(),
                path: path.path,
            };
        }
        if let Some(e) = existing {
            self.edit_existing_file(file_name, path, backend, e.size).await;
            return Flow::Continue;
        }
        // Nothing there yet: an empty buffer aimed at the new path, so the first
        // save creates the file (wherever the panel is — disk, archive, remote).
        let mut ed = EditorState::new(file_name, path, "");
        self.prepare_editor(&mut ed);
        self.editor = Some(ed);
        Flow::Continue
    }

    /// Open a local file directly in the internal editor — used by the `/edit`
    /// startup mode. A missing file opens an empty buffer (so it can be created);
    /// a file too large to load as text opens in in-place hex mode.
    pub(crate) async fn open_path_in_editor(&mut self, path: std::path::PathBuf) {
        // Resolve to an absolute path so saving is unaffected by any later change
        // of the active panel's directory.
        let abs = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().map(|d| d.join(&path)).unwrap_or(path)
        };
        let name = abs
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| abs.to_string_lossy().into_owned());
        let meta = std::fs::metadata(&abs).ok();
        if meta.as_ref().is_some_and(|m| m.is_dir()) {
            self.show_error(format!("{name} is a directory"));
            return;
        }
        let vpath = VfsPath::local(&abs);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        // A shapefile opens as its GeoJSON here too, so `rcedit roads.shp` does
        // what F4 on the same file does.
        if crate::geo::shapefile::is_shapefile_name(&name) {
            match EditorState::new_shapefile(name.clone(), vpath.clone()) {
                Ok((mut ed, warning)) => {
                    self.prepare_editor(&mut ed);
                    self.editor = Some(ed);
                    if let Some(w) = warning {
                        self.show_error(w);
                    }
                    return;
                }
                Err(e) => {
                    self.show_error(e);
                    return;
                }
            }
        }
        // An audio file opens on its tags here too, so `rcedit song.mp3` does
        // what F4 on the same file does.
        if crate::tags::is_taggable_name(&name)
            && let Some(mut ed) = EditorState::new_tags(name.clone(), vpath.clone())
        {
            self.prepare_editor(&mut ed);
            self.editor = Some(ed);
            return;
        }
        if size > crate::editor::MAX_TEXT_EDIT {
            match EditorState::new_hex(name, vpath) {
                Ok(mut ed) => {
                    self.prepare_editor(&mut ed);
                    self.editor = Some(ed);
                }
                Err(e) => self.show_error(format!("Cannot open file: {e}")),
            }
            return;
        }
        let text = std::fs::read(&abs)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let mut ed = EditorState::new(name, vpath, &text);
        self.prepare_editor(&mut ed);
        Self::restore_editor_position(&mut ed);
        self.editor = Some(ed);
    }

    /// Open the editor on a fresh, unnamed buffer (`rc /edit` with no file). It
    /// has no path yet, so the first save is redirected to "Save as" (see
    /// [`Self::save_editor`]).
    pub(crate) fn open_new_editor(&mut self) {
        let mut ed = EditorState::new_unnamed();
        self.prepare_editor(&mut ed);
        self.editor = Some(ed);
    }

    /// Stream a (remote/archive) file to a local temp file for view/edit, showing
    /// a cancellable progress dialog. Delivers `FileFetched` on success.
    fn start_fetch(
        &mut self,
        kind: FetchKind,
        name: String,
        path: VfsPath,
        backend: std::sync::Arc<dyn Vfs>,
        total: u64,
    ) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(id, TaskHandle { id, cancel: cancel.clone(), reply });
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Reading")));

        let temp = crate::util::temp::rc_temp_path("fetch");
        let tx = self.tx.clone();
        let orig_path = path.clone();
        tokio::spawn(async move {
            let outcome =
                fetch_to_temp(&backend, &path, &temp, total, &cancel, id, &name, &tx).await;
            match outcome {
                Ok(true) => {
                    let _ =
                        tx.send(AppEvent::FileFetched { id, kind, name, orig_path, temp }).await;
                }
                Ok(false) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ =
                        tx.send(AppEvent::TaskDone { id, outcome: TaskOutcome::Cancelled }).await;
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ =
                        tx.send(AppEvent::TaskDone { id, outcome: TaskOutcome::Failed(e) }).await;
                }
            }
        });
    }

    /// Persist the editor's contents to its file, optionally closing after.
    pub(in crate::app::state) async fn save_editor(&mut self, close_after: bool) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        // A fresh, unnamed buffer has no path to write to — send the user to
        // "Save as" to choose a filename first (`close_after` can't be honored
        // here; the user saves, then quits again once the buffer has a name).
        if ed.is_unnamed() {
            self.open_save_as(None);
            return;
        }
        // A buffer that came from a shapefile goes back as one: the text is
        // only how it was edited, not what it is.
        if ed.is_shapefile() {
            let contents = ed.contents();
            let origin = ed.shapefile.clone().expect("checked above");
            match crate::geo::shapefile::save(&origin, &contents) {
                Ok(report) => {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.mark_saved();
                    }
                    if let Some(msg) = report.message() {
                        self.show_error(msg);
                    }
                    if close_after {
                        self.close_editor().await;
                    }
                }
                Err(e) => self.show_error(e),
            }
            return;
        }
        // An audio file opened for its tags writes them through the tag writer,
        // which rewrites the container. Any pending byte edits are flushed
        // first, inside `flush_tags`, since a tag write moves everything after
        // it and would leave their offsets stale.
        if ed.tags.is_some() {
            let res = self.editor.as_mut().unwrap().flush_tags();
            match res {
                Ok(()) => {
                    if close_after {
                        self.close_editor().await;
                    } else if let Some(ed) = self.editor.as_mut() {
                        ed.mark_saved();
                    }
                }
                Err(e) => self.show_error(format!("Save failed: {e}")),
            }
            return;
        }
        // Hex mode writes only the changed bytes in place — never rewrite the
        // whole (possibly huge) file from the text buffer.
        if ed.is_hex() {
            let res = self.editor.as_mut().unwrap().flush_hex();
            match res {
                Ok(()) => {
                    if close_after {
                        self.close_editor().await;
                    } else if let Some(ed) = self.editor.as_mut() {
                        ed.mark_saved();
                    }
                }
                Err(e) => self.show_error(format!("Save failed: {e}")),
            }
            return;
        }
        let contents = ed.contents();
        let path = ed.path.clone();
        let backend = match self.registry.resolve(&path) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot save file: {e}")),
        };
        match write_file(&backend, &path, contents.as_bytes()).await {
            Ok(()) => {
                // Editing a config file (themes.toml, the F2 menu, rc.ext, a
                // binary template) applies immediately on save.
                self.reload_config_if_edited(&path);
                if close_after {
                    self.close_editor().await;
                } else if let Some(ed) = self.editor.as_mut() {
                    ed.mark_saved();
                }
            }
            // A failed save (e.g. read-only location, permission denied) drops the
            // user into "Save as" prefilled with the path, so they can redirect it.
            Err(e) => self.open_save_as(Some(format!("Save failed: {e}"))),
        }
    }

    /// Open the editor's "Save as" browser, prefilled with the current file's
    /// directory and name. `error` shows the reason a prior save failed.
    pub(in crate::app::state) fn open_save_as(&mut self, error: Option<String>) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        let cwd = || std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        // An unnamed buffer has no meaningful directory or name to prefill:
        // start in the working directory with an empty name field.
        let (dir, name) = if ed.is_unnamed() {
            (cwd(), String::new())
        } else if ed.path.scheme == "file" {
            let dir = ed.path.path.parent().map(|p| p.to_path_buf()).unwrap_or_else(cwd);
            (dir, ed.name.clone())
        } else {
            // The browser is local-only, so a remote-edited file saves to disk.
            (cwd(), ed.name.clone())
        };
        self.dialog = Some(Dialog::SaveAs(SaveAsDialog::new(dir, name, error)));
    }

    /// Write the editor buffer to `dest` (from the "Save as" dialog) and retarget
    /// the editor at the new path on success.
    pub(in crate::app::state) async fn do_save_as(&mut self, dest: std::path::PathBuf) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        let contents = ed.contents();
        match tokio::fs::write(&dest, contents.as_bytes()).await {
            Ok(()) => {
                let name = dest
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dest.to_string_lossy().into_owned());
                let dark = self.dark_ui();
                let new_path = VfsPath::local(&dest);
                if let Some(ed) = self.editor.as_mut() {
                    ed.path = new_path.clone();
                    ed.name = name;
                    ed.set_named(); // it now has a filename; future saves write in place
                    ed.mark_saved();
                    ed.enable_syntax(dark); // re-detect syntax for the new name
                    ed.detect_kind(); // and whether it is now a table
                }
                self.reload_config_if_edited(&new_path);
                self.reload_all().await;
            }
            // Still failing: re-prompt with the new error so the user can adjust.
            Err(e) => self.open_save_as(Some(format!("Save failed: {e}"))),
        }
    }

    /// Open the visual theme editor (Options → Edit themes), starting on the
    /// app's currently-active theme.
    pub(in crate::app::state) fn open_edit_themes(&mut self) {
        self.theme_editor =
            Some(crate::ui::theme_editor::ThemeEditor::new(&self.config.theme, self.truecolor));
    }

    /// Apply a signal from the visual theme editor: close it, or persist the
    /// edited spec to `themes.toml` and re-derive the live theme.
    pub(in crate::app::state) fn apply_theme_editor_signal(
        &mut self,
        sig: crate::ui::theme_editor::ThemeEditorSignal,
    ) {
        use crate::ui::theme_editor::ThemeEditorSignal;
        match sig {
            ThemeEditorSignal::Stay => {}
            ThemeEditorSignal::Close => self.theme_editor = None,
            ThemeEditorSignal::Save(spec) => match crate::ui::theme::save_spec(*spec) {
                Ok(()) => {
                    if let Some(te) = self.theme_editor.as_mut() {
                        te.mark_saved();
                    }
                    // Re-derive the active theme: if the saved theme is the one in
                    // use, its edits take effect at once; otherwise this is a no-op.
                    self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                }
                Err(e) => self.show_error(format!("Could not save theme: {e}")),
            },
            ThemeEditorSignal::SaveAndClose(spec) => match crate::ui::theme::save_spec(*spec) {
                Ok(()) => {
                    self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                    self.theme_editor = None;
                }
                // Keep the editor open on a write error so the edits aren't lost.
                Err(e) => self.show_error(format!("Could not save theme: {e}")),
            },
        }
    }

    /// Open `rc.ext` (file associations) in the internal editor (Options → Edit
    /// extensions), creating the default file first if it doesn't exist yet.
    pub(in crate::app::state) fn open_edit_extensions(&mut self) {
        let Some(path) = crate::ext::ensure_ext_file() else {
            return self.show_error("No config directory available");
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut ed = EditorState::new("rc.ext".to_string(), VfsPath::local(&path), &text);
                self.prepare_editor(&mut ed);
                self.editor = Some(ed);
            }
            Err(e) => self.show_error(format!("Cannot open rc.ext: {e}")),
        }
    }

    /// Open the F2 user `menu` file in the internal editor (Options → Edit menu
    /// file), creating the default file first if it doesn't exist yet.
    pub(in crate::app::state) fn open_edit_user_menu(&mut self) {
        let Some(path) = crate::usermenu::ensure_menu_file() else {
            return self.show_error("No config directory available");
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut ed = EditorState::new("menu".to_string(), VfsPath::local(&path), &text);
                self.prepare_editor(&mut ed);
                self.editor = Some(ed);
            }
            Err(e) => self.show_error(format!("Cannot open menu: {e}")),
        }
    }

    /// After saving in the internal editor, apply the change to live state at
    /// once when the saved file is one of the user-editable config files —
    /// `themes.toml`, the F2 user `menu`, or `rc.ext` (file associations) — so the
    /// user doesn't have to restart. Ordinary files are ignored.
    fn reload_config_if_edited(&mut self, path: &VfsPath) {
        if path.scheme != "file" {
            return;
        }
        self.reload_themes_if_edited(path);
        self.reload_usermenu_if_edited(path);
        self.reload_ext_if_edited(path);
        // A template's header (masks, ID bytes) may have changed.
        if crate::config::paths::templates_dir().is_some_and(|d| path.path.starts_with(d)) {
            crate::bt::library::invalidate();
        }
    }

    /// If the file just saved is `themes.toml`, re-read the palettes and
    /// re-derive the current theme so the change takes effect at once.
    fn reload_themes_if_edited(&mut self, path: &VfsPath) {
        let is_themes = path.scheme == "file"
            && crate::config::paths::themes_file().is_some_and(|p| p == path.path);
        if !is_themes {
            return;
        }
        match crate::ui::theme::reload_user_themes() {
            Ok(_) => self.theme = Theme::by_name(&self.config.theme, self.truecolor),
            Err(e) => self.show_error(format!("themes.toml: {e}")),
        }
    }

    /// If the file just saved is the F2 user `menu`, re-read it so the next F2
    /// reflects the edit without a restart.
    fn reload_usermenu_if_edited(&mut self, path: &VfsPath) {
        let is_menu = crate::config::paths::menu_file().is_some_and(|p| p == path.path);
        if is_menu {
            self.user_menu = crate::usermenu::load_or_create();
        }
    }

    /// If the file just saved is `rc.ext`, re-read the file associations so the
    /// new rules apply to the next Open/View/Edit without a restart.
    fn reload_ext_if_edited(&mut self, path: &VfsPath) {
        let is_ext = crate::config::paths::ext_file().is_some_and(|p| p == path.path);
        if is_ext {
            self.ext_rules = crate::ext::ExtRules::load_or_create();
        }
    }
    /// Compare the two panels' files and mark the differing ones (selection).
    /// `Quick` marks files missing from the other panel; `Size` additionally
    /// marks the larger of two differently-sized files; `Content` marks both
    /// files whenever their bytes differ.
    pub(in crate::app::state) async fn compare_dirs(&mut self, mode: CompareMode) {
        if self.panels[0].is_panelized() || self.panels[1].is_panelized() {
            return self.show_error("Cannot compare search-result panels");
        }
        let files = |p: &Panel| -> Vec<(String, u64)> {
            p.entries
                .iter()
                .filter(|e| e.kind == VfsKind::File && e.name != "..")
                .map(|e| (e.name.clone(), e.size))
                .collect()
        };
        let a = files(&self.panels[0]);
        let b = files(&self.panels[1]);
        let amap: HashMap<&str, u64> = a.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let bmap: HashMap<&str, u64> = b.iter().map(|(n, s)| (n.as_str(), *s)).collect();

        let mut mark_a: Vec<String> = Vec::new();
        let mut mark_b: Vec<String> = Vec::new();

        // Files present in only one panel are always marked there.
        for (n, _) in &a {
            if !bmap.contains_key(n.as_str()) {
                mark_a.push(n.clone());
            }
        }
        for (n, _) in &b {
            if !amap.contains_key(n.as_str()) {
                mark_b.push(n.clone());
            }
        }

        match mode {
            CompareMode::Quick => {}
            CompareMode::Size => {
                for (n, sa) in &a {
                    if let Some(sb) = bmap.get(n.as_str()) {
                        // Mark only the larger of the two.
                        if sa > sb {
                            mark_a.push(n.clone());
                        } else if sb > sa {
                            mark_b.push(n.clone());
                        }
                    }
                }
            }
            CompareMode::Content => {
                let ba = self.panels[0].backend.clone();
                let ca = self.panels[0].cwd.clone();
                let bb = self.panels[1].backend.clone();
                let cb = self.panels[1].cwd.clone();
                for (n, sa) in &a {
                    if let Some(sb) = bmap.get(n.as_str()) {
                        // Different sizes ⇒ different content (no need to read).
                        let differ =
                            sa != sb || files_differ(&ba, &ca.join(n), &bb, &cb.join(n)).await;
                        if differ {
                            mark_a.push(n.clone());
                            mark_b.push(n.clone());
                        }
                    }
                }
            }
        }

        self.panels[0].selection.clear();
        self.panels[1].selection.clear();
        for n in &mark_a {
            self.panels[0].selection.mark(n);
        }
        for n in &mark_b {
            self.panels[1].selection.mark(n);
        }
    }

    /// Open the side-by-side file comparison view on the files under the cursor
    /// in the left (panel 0) and right (panel 1) panels.
    pub(in crate::app::state) async fn open_compare_files(&mut self) {
        let pick = |p: &Panel| -> Option<(String, VfsPath)> {
            p.current_entry()
                .filter(|e| e.kind == VfsKind::File && e.name != "..")
                .map(|e| (e.name.clone(), p.cwd.join(&e.name)))
        };
        let (Some((ln, lp)), Some((rn, rp))) = (pick(&self.panels[0]), pick(&self.panels[1]))
        else {
            return self.show_error("Put the cursor on a file in both panels to compare");
        };
        use crate::diff::hex::{HexDiffView, Origin};
        // Two local files are compared byte by byte, paged from disk, when
        // either is binary or too large to load as text.
        if lp.scheme == "file" && rp.scheme == "file" {
            let paths = [lp.path.clone(), rp.path.clone()];
            let probe = paths.clone();
            let binary = tokio::task::spawn_blocking(move || {
                probe.iter().try_fold(false, |any, p| {
                    let big = std::fs::metadata(p)?.len() > MAX_VIEW_BYTES as u64;
                    Ok::<_, std::io::Error>(any || big || crate::diff::hex::file_is_binary(p)?)
                })
            })
            .await;
            match binary {
                Ok(Ok(true)) => {
                    let [a, b] = paths;
                    return match HexDiffView::open([ln, rn], [Origin::File(a), Origin::File(b)]) {
                        Ok(v) => self.hexdiff = Some(Box::new(v)),
                        Err(e) => self.show_error(format!("Cannot compare: {e}")),
                    };
                }
                Ok(Err(e)) => return self.show_error(format!("Cannot compare: {e}")),
                Ok(Ok(false)) | Err(_) => {}
            }
        }
        let lback = self.panels[0].backend.clone();
        let rback = self.panels[1].backend.clone();
        let ldata = match load_file(&lback, &lp).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("Cannot read {ln}: {e}")),
        };
        let rdata = match load_file(&rback, &rp).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("Cannot read {rn}: {e}")),
        };
        // Binary files read from elsewhere are compared in memory — as long as
        // they were read whole.
        if crate::diff::hex::is_binary(&ldata) || crate::diff::hex::is_binary(&rdata) {
            if ldata.len() > MAX_VIEW_BYTES || rdata.len() > MAX_VIEW_BYTES {
                return self.show_error("These files are too large to compare here: copy them to a local directory first");
            }
            let origins = [Origin::Mem(ldata.into()), Origin::Mem(rdata.into())];
            return match HexDiffView::open([ln, rn], origins) {
                Ok(v) => self.hexdiff = Some(Box::new(v)),
                Err(e) => self.show_error(format!("Cannot compare: {e}")),
            };
        }
        self.diffview = Some(DiffView::new(ln, lp, &ldata, rn, rp, &rdata));
    }

    /// Write the diff view's changed buffers back to disk.
    pub(in crate::app::state) async fn save_diff(&mut self) {
        let saves = match self.diffview.as_ref() {
            Some(dv) => dv.pending_saves(),
            None => return,
        };
        if saves.is_empty() {
            return;
        }
        let mut ok = true;
        for (path, contents) in saves {
            match self.registry.resolve(&path) {
                Ok(backend) => {
                    if let Err(e) = write_file(&backend, &path, contents.as_bytes()).await {
                        self.show_error(format!("Save failed: {e}"));
                        ok = false;
                    }
                }
                Err(e) => {
                    self.show_error(format!("Cannot save file: {e}"));
                    ok = false;
                }
            }
        }
        if ok {
            if let Some(dv) = self.diffview.as_mut() {
                dv.mark_saved();
            }
            self.reload_all().await;
        }
    }
}
