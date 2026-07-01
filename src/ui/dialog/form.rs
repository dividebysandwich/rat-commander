//! Form dialog (settings, chmod, chown, symlink, connect, formatter).

use super::widgets::*;
use super::{ConfirmValues, DialogResult, DupCriteria, SettingsValues, Submit};
use crate::vfs::remote::{Protocol, RemoteCreds};

/// The display label for a `graphics` config preference, for the settings chooser.
fn graphics_label(pref: &str) -> &'static str {
    match pref.trim().to_ascii_lowercase().as_str() {
        "off" => "Off",
        "kitty" => "Kitty",
        "sixel" => "Sixel",
        "iterm" | "iterm2" => "iTerm2",
        _ => "Auto",
    }
}

/// The canonical `graphics` config value for a chooser display label.
fn graphics_pref(label: &str) -> String {
    match label.trim().to_ascii_lowercase().as_str() {
        "off" => "off",
        "kitty" => "kitty",
        "sixel" => "sixel",
        "iterm2" | "iterm" => "iterm",
        _ => "auto",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Form dialog (settings, chmod, chown, symlink)
// ---------------------------------------------------------------------------

/// A single editable field in a [`Form`].
pub enum Field {
    Text {
        label: String,
        value: String,
        cursor: usize,
    },
    Password {
        label: String,
        value: String,
        cursor: usize,
    },
    Check {
        label: String,
        value: bool,
    },
    /// A choice picked from a scrollable dropdown (Enter opens it).
    Choice {
        label: String,
        options: Vec<String>,
        idx: usize,
        /// Whether the dropdown list is currently open.
        open: bool,
        /// Highlighted option while the dropdown is open.
        sel: usize,
        /// First visible option while open (scroll offset); adjusted only when
        /// the highlight would leave the window, so the cursor moves freely.
        top: usize,
    },
}

impl Field {
    pub fn text(label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Field::Text {
            label: label.to_string(),
            value,
            cursor,
        }
    }

    pub fn password(label: &str) -> Self {
        Field::Password {
            label: label.to_string(),
            value: String::new(),
            cursor: 0,
        }
    }

    pub fn check(label: &str, value: bool) -> Self {
        Field::Check {
            label: label.to_string(),
            value,
        }
    }

    pub fn choice(label: &str, options: Vec<String>, selected: &str) -> Self {
        let idx = options.iter().position(|o| o == selected).unwrap_or(0);
        Field::Choice {
            label: label.to_string(),
            options,
            idx,
            open: false,
            sel: idx,
            top: 0,
        }
    }

    fn as_text(&self) -> &str {
        match self {
            Field::Text { value, .. } | Field::Password { value, .. } => value,
            Field::Choice { options, idx, .. } => options.get(*idx).map(|s| s.as_str()).unwrap_or(""),
            Field::Check { .. } => "",
        }
    }

    fn as_bool(&self) -> bool {
        matches!(self, Field::Check { value: true, .. })
    }
}

/// A vertical list of editable fields with a single focused row.
pub struct Form {
    fields: Vec<Field>,
    pub(crate) focus: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        Form { fields, focus: 0 }
    }

    /// Number of fields (used to compute the dialog height for click geometry).
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    // The focus cycles over the fields plus two trailing "slots" for the OK and
    // Cancel buttons, so they can be reached and activated with the keyboard.
    fn slots(&self) -> usize {
        self.fields.len() + 2
    }
    fn ok_slot(&self) -> usize {
        self.fields.len()
    }
    fn cancel_slot(&self) -> usize {
        self.fields.len() + 1
    }
    /// Whether focus is on the OK or Cancel button (not a field).
    fn on_button(&self) -> bool {
        self.focus >= self.fields.len()
    }
    fn on_ok(&self) -> bool {
        self.focus == self.ok_slot()
    }
    fn on_cancel(&self) -> bool {
        self.focus == self.cancel_slot()
    }

    fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.slots();
    }

    fn focus_prev(&mut self) {
        self.focus = (self.focus + self.slots() - 1) % self.slots();
    }

    /// Handle a key for the focused field. Returns true if Enter (submit) was
    /// pressed.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => return true,
            KeyCode::Tab | KeyCode::Down => self.focus_next(),
            KeyCode::BackTab | KeyCode::Up => self.focus_prev(),
            KeyCode::Char(' ') if matches!(self.fields.get(self.focus), Some(Field::Check { .. })) => {
                if let Some(Field::Check { value, .. }) = self.fields.get_mut(self.focus) {
                    *value = !*value;
                }
            }
            // Choice fields are changed via their dropdown (opened with Enter),
            // handled in `FormDialog::handle_key` — arrows just move focus.
            _ => match self.fields.get_mut(self.focus) {
                Some(Field::Text { value, cursor, .. })
                | Some(Field::Password { value, cursor, .. }) => edit_text(value, cursor, key),
                _ => {}
            },
        }
        false
    }
}

/// A dialog title like `"Chmod: file.txt"` for one target or `"Chmod: 3 items"`
/// for several.
fn form_target_title(verb: &str, targets: &[VfsPath]) -> String {
    match targets {
        [one] => format!("{verb}: {}", one.file_name()),
        many => format!("{verb}: {} items", many.len()),
    }
}

/// What a form's values should become on submit.
pub enum FormPurpose {
    Settings,
    Confirmations,
    /// Change permissions of these targets (recursing into dirs if requested).
    Chmod(Vec<VfsPath>),
    /// Change ownership of these targets (recursing into dirs if requested).
    Chown(Vec<VfsPath>),
    /// Create a symlink inside this directory.
    Symlink(VfsPath),
    /// Open a remote connection of this protocol on the given panel side.
    Connect(Protocol, usize),
    /// Format this device node (disk manager).
    Format(String),
    /// Collect the "Find duplicates" comparison criteria.
    FindDuplicates,
}

/// Connect-form history dropdown state (recent servers).
pub(crate) struct ConnectDropdown {
    history: Vec<crate::config::RemoteHistoryEntry>,
    pub(crate) open: bool,
    sel: usize,
    /// Click geometry recorded at render time: chevron, plus (rect, index) per
    /// visible dropdown entry.
    chevron: Option<Rect>,
    entries: Vec<(Rect, usize)>,
}

pub struct FormDialog {
    pub title: String,
    pub form: Form,
    pub purpose: FormPurpose,
    /// Present only for connect forms (drives the recent-servers dropdown).
    pub(crate) connect: Option<ConnectDropdown>,
}

impl FormDialog {
    pub fn settings(cfg: &crate::config::Config, truecolor: bool) -> Self {
        let form = Form::new(vec![
            Field::choice("Theme", crate::ui::theme::palette_names(), &cfg.theme),
            Field::choice("Language", crate::l10n::available(), &crate::l10n::active_name()),
            Field::check("Truecolor (gradients)", truecolor),
            Field::check("Animations", cfg.animation),
            Field::check("System status widget", cfg.system_status),
            Field::text("External editor", cfg.editor.clone()),
            Field::text("External viewer", cfg.viewer.clone()),
            Field::check("Use internal viewer", cfg.use_internal_viewer),
            Field::check("Use internal editor", cfg.use_internal_editor),
            Field::check("Reshape RTL text", cfg.reshape_rtl),
            Field::choice(
                "Graphics",
                vec![
                    "Auto".into(),
                    "Off".into(),
                    "Kitty".into(),
                    "Sixel".into(),
                    "iTerm2".into(),
                ],
                graphics_label(&cfg.graphics),
            ),
        ]);
        FormDialog {
            title: "Settings".to_string(),
            form,
            purpose: FormPurpose::Settings,
            connect: None,
        }
    }

    /// Build the Confirmations form (which actions require a confirmation).
    pub fn confirmations(cfg: &crate::config::Config) -> Self {
        let form = Form::new(vec![
            Field::check("Confirm delete", cfg.confirm_delete),
            Field::check("Confirm overwrite", cfg.confirm_overwrite),
            Field::check("Confirm execute", cfg.confirm_execute),
            Field::check("Confirm unmount", cfg.confirm_unmount),
            Field::check("Confirm exit", cfg.confirm_exit),
        ]);
        FormDialog {
            title: "Confirmations".to_string(),
            form,
            purpose: FormPurpose::Confirmations,
            connect: None,
        }
    }

    /// Build the "Find duplicates" options form. With size/date/content all off,
    /// only file names are compared; name matching is case-sensitive by default.
    pub fn find_duplicates() -> Self {
        let form = Form::new(vec![
            Field::check("Also compare size", false),
            Field::check("Also compare date/time", false),
            Field::check("Also compare content", false),
            Field::check("Case-sensitive names", true),
        ]);
        FormDialog {
            title: "Find duplicates".to_string(),
            form,
            purpose: FormPurpose::FindDuplicates,
            connect: None,
        }
    }

    /// Build the disk formatter form for `dev`.
    pub fn format(dev: String) -> Self {
        let fs_options: Vec<String> =
            crate::mount::FsType::ALL.iter().map(|f| f.label().to_string()).collect();
        let form = Form::new(vec![
            Field::choice("Filesystem", fs_options, "FAT32"),
            Field::text("Volume label", ""),
            Field::check("Quick format (NTFS)", false),
            Field::text("Bytes/inode (ext, blank=auto)", ""),
        ]);
        FormDialog {
            title: format!("Format {dev}"),
            form,
            purpose: FormPurpose::Format(dev),
            connect: None,
        }
    }

    /// Build a chmod form for `targets` from the current mode bits. The trailing
    /// "Recurse into directories" checkbox makes the change apply into any
    /// directories in the selection.
    pub fn chmod(targets: Vec<VfsPath>, mode: u32) -> Self {
        let bit = |m: u32| mode & m != 0;
        let form = Form::new(vec![
            Field::check("Owner read    (400)", bit(0o400)),
            Field::check("Owner write   (200)", bit(0o200)),
            Field::check("Owner exec    (100)", bit(0o100)),
            Field::check("Group read    (040)", bit(0o040)),
            Field::check("Group write   (020)", bit(0o020)),
            Field::check("Group exec    (010)", bit(0o010)),
            Field::check("Other read    (004)", bit(0o004)),
            Field::check("Other write   (002)", bit(0o002)),
            Field::check("Other exec    (001)", bit(0o001)),
            Field::check("Recurse into directories", false),
        ]);
        FormDialog {
            title: form_target_title("Chmod", &targets),
            form,
            purpose: FormPurpose::Chmod(targets),
            connect: None,
        }
    }

    pub fn chown(targets: Vec<VfsPath>, owner: String, group: String) -> Self {
        let form = Form::new(vec![
            Field::text("Owner (name or uid)", owner),
            Field::text("Group (name or gid)", group),
            Field::check("Recurse into directories", false),
        ]);
        FormDialog {
            title: form_target_title("Chown", &targets),
            form,
            purpose: FormPurpose::Chown(targets),
            connect: None,
        }
    }

    pub fn symlink(dir: VfsPath, target: String, name: String) -> Self {
        let form = Form::new(vec![
            Field::text("Points to (target)", target),
            Field::text("Link name", name),
        ]);
        FormDialog {
            title: "Create symlink".to_string(),
            form,
            purpose: FormPurpose::Symlink(dir),
            connect: None,
        }
    }

    /// The currently-selected theme name in the settings form (for live
    /// preview), or `None` if this isn't the settings form.
    pub fn theme_choice(&self) -> Option<&str> {
        self.choice_value("Theme")
    }

    /// The currently-selected language name in the settings form (for live
    /// preview), or `None` if this isn't the settings form.
    pub fn lang_choice(&self) -> Option<&str> {
        self.choice_value("Language")
    }

    /// The currently-selected graphics preference (`auto|off|kitty|sixel|iterm`)
    /// in the settings form (for live preview), or `None` if not the settings form.
    pub fn graphics_choice(&self) -> Option<String> {
        self.choice_value("Graphics").map(graphics_pref)
    }

    /// The value of the settings `Check` field labelled `label_key` (for live
    /// preview), or `None` if this isn't the settings form.
    pub fn check_value(&self, label_key: &str) -> Option<bool> {
        if !matches!(self.purpose, FormPurpose::Settings) {
            return None;
        }
        self.form.fields.iter().find_map(|f| match f {
            Field::Check { label, value } if label == label_key => Some(*value),
            _ => None,
        })
    }

    /// The highlighted option of the settings `Choice` field labelled `label`.
    fn choice_value(&self, label_key: &str) -> Option<&str> {
        if !matches!(self.purpose, FormPurpose::Settings) {
            return None;
        }
        self.form.fields.iter().find_map(|f| match f {
            Field::Choice { label, options, idx, open, sel, .. } if label == label_key => {
                // While the dropdown is open, preview the highlighted option so
                // scrolling shows a live theme/language preview.
                options.get(if *open { *sel } else { *idx }).map(|s| s.as_str())
            }
            _ => None,
        })
    }

    pub fn connect(
        protocol: Protocol,
        side: usize,
        history: Vec<crate::config::RemoteHistoryEntry>,
    ) -> Self {
        let form = Form::new(vec![
            Field::text("Host", ""),
            Field::text("Port", protocol.default_port().to_string()),
            Field::text("Username", ""),
            Field::password("Password"),
            Field::text("Remote path (blank = home)", ""),
        ]);
        // Only this protocol's recent connections.
        let history: Vec<_> = history
            .into_iter()
            .filter(|e| e.protocol == protocol.scheme_prefix())
            .collect();
        FormDialog {
            title: format!("{} connection", protocol.scheme_prefix().to_uppercase()),
            form,
            purpose: FormPurpose::Connect(protocol, side),
            connect: Some(ConnectDropdown {
                history,
                open: false,
                sel: 0,
                chevron: None,
                entries: Vec::new(),
            }),
        }
    }

    /// Fill the host/port/user/path fields from history entry `idx` and move the
    /// focus to the password field.
    fn apply_history(&mut self, idx: usize) {
        let entry = match self.connect.as_ref().and_then(|c| c.history.get(idx).cloned()) {
            Some(e) => e,
            None => return,
        };
        if let Some(c) = self.connect.as_mut() {
            c.open = false;
        }
        set_text_field(&mut self.form.fields[0], &entry.host);
        set_text_field(&mut self.form.fields[1], &entry.port.to_string());
        set_text_field(&mut self.form.fields[2], &entry.user);
        if let Some(field) = self.form.fields.get_mut(4) {
            set_text_field(field, &entry.path);
        }
        self.form.focus = 3; // password
    }

    /// Route a click for the connect dropdown. Returns `Some` if the click hit
    /// the chevron or a dropdown entry (or dismissed an open dropdown).
    pub(crate) fn click_dropdown(&mut self, col: u16, row: u16) -> Option<DialogResult> {
        let hit = |r: &Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        let cd = self.connect.as_ref()?;
        if cd.chevron.is_some_and(|r| hit(&r)) {
            let cd = self.connect.as_mut().unwrap();
            cd.open = !cd.open;
            cd.sel = 0;
            return Some(DialogResult::None);
        }
        if !cd.open {
            return None;
        }
        let hidx = cd.entries.iter().find(|(r, _)| hit(r)).map(|&(_, i)| i);
        match hidx {
            Some(i) => self.apply_history(i),
            None => self.connect.as_mut().unwrap().open = false,
        }
        Some(DialogResult::None)
    }

    fn chmod_mode(&self) -> u32 {
        const BITS: [u32; 9] = [
            0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
        ];
        // `zip` stops at the 9 permission bits, so the trailing "Recurse"
        // checkbox is ignored here.
        let mut mode = 0;
        for (f, bit) in self.form.fields.iter().zip(BITS) {
            if f.as_bool() {
                mode |= bit;
            }
        }
        mode
    }

    /// Whether the chmod/chown "Recurse into directories" checkbox (always the
    /// last field of those forms) is ticked.
    fn recursive(&self) -> bool {
        self.form.fields.last().map(|f| f.as_bool()).unwrap_or(false)
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        // Connect-form history dropdown: while open it captures navigation keys;
        // closed, pressing ↓ on the Host field opens it.
        let drop_open = self.connect.as_ref().is_some_and(|c| c.open);
        if drop_open {
            match key.code {
                KeyCode::Esc => self.connect.as_mut().unwrap().open = false,
                KeyCode::Up => {
                    let c = self.connect.as_mut().unwrap();
                    c.sel = c.sel.saturating_sub(1);
                }
                KeyCode::Down => {
                    let c = self.connect.as_mut().unwrap();
                    if c.sel + 1 < c.history.len() {
                        c.sel += 1;
                    }
                }
                KeyCode::Enter => {
                    let i = self.connect.as_ref().unwrap().sel;
                    self.apply_history(i);
                }
                _ => {}
            }
            return DialogResult::None;
        }
        if matches!(key.code, KeyCode::Down)
            && self.form.focus == 0
            && self.connect.as_ref().is_some_and(|c| !c.history.is_empty())
        {
            let c = self.connect.as_mut().unwrap();
            c.open = true;
            c.sel = 0;
            return DialogResult::None;
        }

        // A Choice field's scrollable dropdown: Enter on a closed choice opens
        // it; while open, the arrows move the highlight, Enter picks, Esc closes.
        if let Some(Field::Choice { options, idx, open, sel, .. }) =
            self.form.fields.get_mut(self.form.focus)
        {
            if *open {
                let last = options.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => *open = false,
                    KeyCode::Up => *sel = sel.saturating_sub(1),
                    KeyCode::Down => *sel = (*sel + 1).min(last),
                    KeyCode::PageUp => *sel = sel.saturating_sub(8),
                    KeyCode::PageDown => *sel = (*sel + 8).min(last),
                    KeyCode::Home => *sel = 0,
                    KeyCode::End => *sel = last,
                    KeyCode::Enter => {
                        *idx = *sel;
                        *open = false;
                    }
                    _ => {}
                }
                return DialogResult::None;
            }
            if key.code == KeyCode::Enter {
                *sel = *idx;
                *open = true;
                return DialogResult::None;
            }
        }

        if let KeyCode::Esc = key.code {
            return DialogResult::Cancel;
        }

        // Focus on the OK / Cancel buttons (the two slots after the fields):
        // arrows move between them and back to the fields; Enter/Space activates.
        if self.form.on_button() {
            match key.code {
                KeyCode::Left | KeyCode::Right => {
                    self.form.focus =
                        if self.form.on_cancel() { self.form.ok_slot() } else { self.form.cancel_slot() };
                    return DialogResult::None;
                }
                KeyCode::Up | KeyCode::BackTab => {
                    self.form.focus_prev();
                    return DialogResult::None;
                }
                KeyCode::Down | KeyCode::Tab => {
                    self.form.focus_next();
                    return DialogResult::None;
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if self.form.on_cancel() {
                        return DialogResult::Cancel;
                    }
                    // OK: fall through to build the submit payload below.
                }
                _ => return DialogResult::None,
            }
        } else if !self.form.handle_key(key) {
            return DialogResult::None;
        }
        // Enter (on a field or OK) → build the submit payload.
        let fields = &self.form.fields;
        let submit = match &self.purpose {
            FormPurpose::Settings => Submit::Settings(SettingsValues {
                theme: fields[0].as_text().to_string(),
                language: fields[1].as_text().to_string(),
                truecolor: fields[2].as_bool(),
                animation: fields[3].as_bool(),
                system_status: fields[4].as_bool(),
                editor: fields[5].as_text().trim().to_string(),
                viewer: fields[6].as_text().trim().to_string(),
                use_internal_viewer: fields[7].as_bool(),
                use_internal_editor: fields[8].as_bool(),
                reshape_rtl: fields[9].as_bool(),
                graphics: graphics_pref(fields[10].as_text()),
            }),
            FormPurpose::Confirmations => Submit::Confirmations(ConfirmValues {
                delete: fields[0].as_bool(),
                overwrite: fields[1].as_bool(),
                execute: fields[2].as_bool(),
                unmount: fields[3].as_bool(),
                exit: fields[4].as_bool(),
            }),
            FormPurpose::Format(dev) => {
                let fs = crate::mount::FsType::from_label(fields[0].as_text())
                    .unwrap_or(crate::mount::FsType::Fat32);
                Submit::Format(crate::mount::FormatSpec {
                    dev: dev.clone(),
                    fs,
                    label: fields[1].as_text().trim().to_string(),
                    quick: fields[2].as_bool(),
                    inode_bytes: fields[3].as_text().trim().to_string(),
                })
            }
            FormPurpose::FindDuplicates => Submit::FindDuplicates(DupCriteria {
                size: fields[0].as_bool(),
                date: fields[1].as_bool(),
                content: fields[2].as_bool(),
                case_sensitive: fields[3].as_bool(),
            }),
            FormPurpose::Chmod(paths) => {
                Submit::Chmod(paths.clone(), self.chmod_mode(), self.recursive())
            }
            FormPurpose::Chown(paths) => Submit::Chown(
                paths.clone(),
                fields[0].as_text().trim().to_string(),
                fields[1].as_text().trim().to_string(),
                self.recursive(),
            ),
            FormPurpose::Symlink(dir) => {
                let target = fields[0].as_text().trim().to_string();
                let name = fields[1].as_text().trim().to_string();
                if target.is_empty() || name.is_empty() {
                    return DialogResult::Cancel;
                }
                Submit::Symlink {
                    dir: dir.clone(),
                    target,
                    name,
                }
            }
            FormPurpose::Connect(protocol, side) => {
                let host = fields[0].as_text().trim().to_string();
                if host.is_empty() {
                    return DialogResult::Cancel;
                }
                let port = fields[1]
                    .as_text()
                    .trim()
                    .parse::<u16>()
                    .unwrap_or(protocol.default_port());
                Submit::Connect(
                    *side,
                    RemoteCreds {
                        protocol: *protocol,
                        host,
                        port,
                        user: fields[2].as_text().trim().to_string(),
                        password: fields[3].as_text().to_string(),
                        path: fields[4].as_text().trim().to_string(),
                    },
                )
            }
        };
        DialogResult::Submit(submit)
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let n = self.form.fields.len() as u16;
        let height = n + 4;
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd(&self.title), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let focus_style = theme.dialog_selection;

        // The Host field of a connect form gets a ▼ chevron to open the history.
        let connect_host = self.connect.as_ref().is_some_and(|c| !c.history.is_empty());
        let mut host_chevron: Option<Rect> = None;

        let mut caret: Option<Position> = None;
        for (i, field) in self.form.fields.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height.saturating_sub(1) {
                break;
            }
            let row = Rect {
                y,
                height: 1,
                ..inner
            };
            let focused = i == self.form.focus;
            match field {
                Field::Text {
                    label,
                    value,
                    cursor,
                }
                | Field::Password {
                    label,
                    value,
                    cursor,
                } => {
                    let masked = matches!(field, Field::Password { .. });
                    let label_str = crate::l10n::display(&format!("{}: ", crate::l10n::tr(label)));
                    let lw = (label_str.chars().count() as u16).min(row.width);
                    let style = if focused { focus_style } else { base };
                    f.render_widget(
                        Paragraph::new(Span::styled(label_str, style)),
                        Rect { width: lw, ..row },
                    );
                    let mut field_area = Rect {
                        x: row.x + lw,
                        width: row.width.saturating_sub(lw),
                        ..row
                    };
                    // Reserve room for the chevron on the Host field.
                    if i == 0 && connect_host && field_area.width > 4 {
                        let cx = field_area.x + field_area.width - 2;
                        host_chevron = Some(Rect { x: cx, y, width: 2, height: 1 });
                        field_area.width -= 2;
                    }
                    if let Some(pos) =
                        draw_input_field(f, field_area, value, *cursor, focused, masked, theme)
                    {
                        caret = Some(pos);
                    }
                }
                Field::Check { label, value } => {
                    let mark = if *value { "[x]" } else { "[ ]" };
                    let style = if focused { focus_style } else { base };
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            crate::l10n::display(&format!("{mark} {}", crate::l10n::tr(label))),
                            style,
                        ))),
                        row,
                    );
                }
                Field::Choice { label, options, idx, .. } => {
                    let style = if focused { focus_style } else { base };
                    let val = options.get(*idx).map(|s| s.as_str()).unwrap_or("");
                    // A ▾ affordance signals the Enter-to-open dropdown.
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            crate::l10n::display(&format!("{}: {val} ▾", crate::l10n::tr(label))),
                            style,
                        ))),
                        row,
                    );
                }
            }
        }

        // Draw the chevron and (when open) the recent-servers dropdown.
        if let Some(chev) = host_chevron {
            let style = base.add_modifier(Modifier::BOLD);
            f.buffer_mut().set_string(chev.x, chev.y, "▼", style);
        }
        let dropdown_open = self.connect.as_ref().is_some_and(|c| c.open);
        if let Some(c) = self.connect.as_mut() {
            c.chevron = host_chevron;
            c.entries.clear();
        }
        if dropdown_open {
            self.render_dropdown(f, inner, theme);
        }

        // Draw an open Choice field's scrollable dropdown over the fields. The
        // scroll offset `top` is nudged only when the highlight leaves the
        // window, so the cursor moves freely within it.
        let choice_open = self.form.fields.iter().any(|f| matches!(f, Field::Choice { open: true, .. }));
        for (i, field) in self.form.fields.iter_mut().enumerate() {
            if let Field::Choice { options, sel, top, open: true, .. } = field {
                let visible = choice_visible_rows(inner, i, options.len());
                if *sel < *top {
                    *top = *sel;
                } else if *sel >= *top + visible {
                    *top = *sel + 1 - visible;
                }
                render_choice_dropdown(f, inner, i, options, *sel, *top, theme);
            }
        }

        let hint = Rect {
            y: inner.y + inner.height.saturating_sub(1),
            height: 1,
            ..inner
        };
        let extra = match &self.purpose {
            FormPurpose::Chmod(_) => format!("  octal {:03o}", self.chmod_mode()),
            _ => String::new(),
        };
        // OK / Cancel buttons highlight when focused (reachable via ↑↓/Tab).
        let ok_txt = crate::l10n::trd("OK");
        let cancel_txt = crate::l10n::trd("Cancel");
        let ok = if self.form.on_ok() {
            Span::styled(format!("[< {ok_txt} >]"), theme.button_focused)
        } else {
            Span::styled(format!("[  {ok_txt}  ]"), theme.button)
        };
        let cancel = if self.form.on_cancel() {
            Span::styled(format!("[< {cancel_txt} >]"), theme.button_focused)
        } else {
            Span::styled(format!("[  {cancel_txt}  ]"), theme.button)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                ok,
                Span::styled(format!("  Tab/↑↓ move  Space toggle{extra}  "), base),
                cancel,
            ]))
            .style(base),
            hint,
        );

        if let Some(pos) = caret
            && !dropdown_open
            && !choice_open
        {
            f.set_cursor_position(pos);
        }
    }

    /// Recompute the dialog's interior rect (mirrors `render`), for click/scroll
    /// hit-testing of the Choice dropdown.
    fn dialog_inner(&self, area: Rect) -> Rect {
        let height = self.form.fields.len() as u16 + 4;
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, height);
        Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        }
    }

    /// Route a click when a Choice dropdown is (or should be) involved: click a
    /// closed Choice row to open it; click an option to pick it; click elsewhere
    /// (while open) to close. Returns `Some` if the click was consumed.
    pub(crate) fn click_choice(&mut self, area: Rect, col: u16, row: u16) -> Option<DialogResult> {
        let inner = self.dialog_inner(area);
        // An open dropdown: pick the clicked option, or close on an outside click.
        if let Some(fi) = self
            .form
            .fields
            .iter()
            .position(|f| matches!(f, Field::Choice { open: true, .. }))
        {
            if let Some(Field::Choice { options, idx, open, sel, top, .. }) = self.form.fields.get_mut(fi) {
                let (rect, visible) = choice_dropdown_geom(inner, fi, options.len());
                let (list_x, list_y, list_w) = (rect.x + 1, rect.y + 1, rect.width.saturating_sub(2));
                if row >= list_y
                    && row < list_y + visible as u16
                    && col >= list_x
                    && col < list_x + list_w
                {
                    let chosen = *top + (row - list_y) as usize;
                    if chosen < options.len() {
                        *idx = chosen;
                        *sel = chosen;
                    }
                }
                *open = false;
            }
            return Some(DialogResult::None);
        }
        // No dropdown open: a click on a Choice row opens it.
        let hit = self.form.fields.iter().enumerate().find_map(|(i, f)| {
            let on_row = row == inner.y + i as u16 && col >= inner.x && col < inner.x + inner.width;
            (matches!(f, Field::Choice { .. }) && on_row).then_some(i)
        });
        if let Some(i) = hit {
            if let Field::Choice { idx, open, sel, .. } = &mut self.form.fields[i] {
                *sel = *idx;
                *open = true;
            }
            self.form.focus = i;
            return Some(DialogResult::None);
        }
        None
    }

    /// Test accessor: `(sel, top)` of the currently open Choice dropdown, if any.
    #[cfg(test)]
    pub(crate) fn open_choice_state(&self) -> Option<(usize, usize)> {
        self.form.fields.iter().find_map(|f| match f {
            Field::Choice { sel, top, open: true, .. } => Some((*sel, *top)),
            _ => None,
        })
    }

    /// Move the open Choice dropdown's highlight (mouse wheel); `delta` in rows.
    pub(crate) fn scroll_choice(&mut self, delta: isize) -> bool {
        if let Some(Field::Choice { options, sel, open: true, .. }) =
            self.form.fields.iter_mut().find(|f| matches!(f, Field::Choice { open: true, .. }))
        {
            let last = options.len().saturating_sub(1) as isize;
            *sel = (*sel as isize + delta).clamp(0, last) as usize;
            return true;
        }
        false
    }

    /// Render the recent-servers list under the Host field and record per-entry
    /// click rects. Scrolls so the selection stays visible.
    fn render_dropdown(&mut self, f: &mut Frame, inner: Rect, theme: &Theme) {
        let Some(c) = self.connect.as_mut() else {
            return;
        };
        if c.history.is_empty() {
            return;
        }
        // The list opens just below the Host row, capped to the dialog interior.
        let top = inner.y + 1;
        let avail = (inner.y + inner.height).saturating_sub(top) as usize;
        let visible = c.history.len().min(avail.saturating_sub(2).max(1));
        let rect = Rect {
            x: inner.x,
            y: top,
            width: inner.width,
            height: (visible + 2) as u16,
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.dialog_title).bg(theme.dialog_bg))
            .title(Span::styled(
                " Recent ",
                Style::default().fg(theme.dialog_title).bg(theme.dialog_bg),
            ))
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
        let list = block.inner(rect);
        f.render_widget(block, rect);

        // Scroll so the selection is on screen.
        let offset = if c.sel >= visible {
            c.sel + 1 - visible
        } else {
            0
        };
        let normal = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let sel_style = theme.dialog_selection;
        for vi in 0..visible {
            let idx = offset + vi;
            let Some(entry) = c.history.get(idx) else {
                break;
            };
            let row = Rect {
                x: list.x,
                y: list.y + vi as u16,
                width: list.width,
                height: 1,
            };
            let style = if idx == c.sel { sel_style } else { normal };
            let text = crate::util::text::ellipsize(&entry.label(), list.width as usize);
            let text = crate::util::text::pad_right(&text, list.width as usize);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(text, style))),
                row,
            );
            c.entries.push((row, idx));
        }
    }
}

/// Geometry of a Choice field's dropdown box: its rect and how many option rows
/// are visible. The dropdown normally drops *below* the field, but opens *above*
/// it when there isn't enough room below (so the last fields in a tall dialog
/// still show their list on screen).
fn choice_dropdown_geom(inner: Rect, fi: usize, options_len: usize) -> (Rect, usize) {
    let field_y = inner.y + fi as u16;
    let below = (inner.y + inner.height).saturating_sub(field_y + 1) as usize; // rows under the field
    let above = fi; // rows over the field, within the dialog interior
    let want = options_len + 2; // options + top/bottom border
    // Prefer dropping down; flip up only when down can't fit and up has more room.
    let open_up = below < want && above > below;
    let room = if open_up { above } else { below };
    let visible = options_len.min(room.saturating_sub(2).max(1)).max(1);
    let box_h = (visible + 2) as u16;
    let y = if open_up { field_y.saturating_sub(box_h) } else { field_y + 1 };
    (Rect { x: inner.x, y, width: inner.width, height: box_h }, visible)
}

/// Number of option rows visible in a Choice dropdown for field `fi`.
fn choice_visible_rows(inner: Rect, fi: usize, options_len: usize) -> usize {
    choice_dropdown_geom(inner, fi, options_len).1
}

/// Draw a Choice field's scrollable dropdown just below its row (field `fi`),
/// showing options from `top` with `sel` highlighted.
fn render_choice_dropdown(
    f: &mut Frame,
    inner: Rect,
    fi: usize,
    options: &[String],
    sel: usize,
    top: usize,
    theme: &Theme,
) {
    if options.is_empty() {
        return;
    }
    let (rect, visible) = choice_dropdown_geom(inner, fi, options.len());
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dialog_title).bg(theme.dialog_bg))
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let list = block.inner(rect);
    f.render_widget(block, rect);

    let normal = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
    for vi in 0..visible {
        let idx = top + vi;
        let Some(opt) = options.get(idx) else {
            break;
        };
        let row = Rect { x: list.x, y: list.y + vi as u16, width: list.width, height: 1 };
        let style = if idx == sel { theme.dialog_selection } else { normal };
        let opt = crate::l10n::display(opt);
        let text = crate::util::text::pad_right(
            &crate::util::text::ellipsize(&opt, list.width as usize),
            list.width as usize,
        );
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), row);
    }
}

