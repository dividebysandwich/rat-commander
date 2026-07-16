//! Form dialog (settings, chmod, chown, symlink, connect, formatter).

use super::widgets::*;
use super::{ConfirmValues, DialogResult, DupCriteria, SettingsValues, Submit};
use crate::util::checksum::ChecksumKind;
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

/// The Settings form's three visual groups: `(title, field count)`, in the
/// order the fields are built in [`FormDialog::settings`]. The field counts must
/// sum to the number of settings fields.
const SETTINGS_GROUPS: &[(&str, usize)] = &[
    ("Language", 2),
    ("Edit/View", 4),
    ("Visual", 6),
];

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
    #[allow(dead_code)] // used by form tests
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
    /// Checksum this file (algorithm + optional comparison digest collected here).
    Checksum(VfsPath),
    /// A guided Git dialog. The form builds its own `git` argv on submit (see
    /// [`GitForm`]), so every git action reaches the app as one `Submit::GitRun`.
    Git(GitForm),
    /// Collect the options for a directory sync; the app then plans it.
    Sync,
}

/// The sync-mode choices, in the order they appear in the dropdown. The safest
/// (adds nothing back, removes nothing) comes first, the destructive mirror last.
pub(crate) const SYNC_MODES: [&str; 3] = [
    "One-way: copy new and changed files",
    "One-way mirror: also delete extraneous files",
    "Two-way: newer file wins",
];

/// The [`SyncMode`](crate::ops::sync::SyncMode) a [`SYNC_MODES`] label selects.
pub(crate) fn sync_mode_of(label: &str) -> crate::ops::sync::SyncMode {
    use crate::ops::sync::SyncMode;
    match label {
        l if l == SYNC_MODES[1] => SyncMode::OneWay { delete_extraneous: true },
        l if l == SYNC_MODES[2] => SyncMode::TwoWay,
        _ => SyncMode::OneWay { delete_extraneous: false },
    }
}

/// `git reset` modes, least destructive first, so the dialog opens on the safe
/// one. The label before the space is passed to git as `--<mode>`.
pub(crate) const RESET_MODES: [&str; 3] = [
    "soft   (keep index + working tree)",
    "mixed  (keep working tree)",
    "hard   (discard all changes!)",
];

/// The bare git mode name from a [`RESET_MODES`] label (`"hard  (…)"` → `"hard"`).
pub(crate) fn reset_mode_name(label: &str) -> &str {
    label.split_whitespace().next().unwrap_or("mixed")
}

/// Which guided Git dialog a [`FormPurpose::Git`] form is. Variants carry the
/// repository facts the argv needs but the fields don't hold (e.g. the branch to
/// name on a `push`).
#[derive(Debug, Clone)]
pub enum GitForm {
    Commit,
    Clone,
    Fetch,
    Pull,
    /// `push` names `<remote> <branch>`; the branch comes from the repo, not the user.
    Push { branch: String },
    Checkout,
    Reset,
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
        // Fields are ordered to match the three visual groups drawn by `render`
        // (see `SETTINGS_GROUPS`): Language, then Edit/View, then Visual. The
        // submit block below reads them back by these indices.
        let form = Form::new(vec![
            // --- Language ---
            Field::choice("Language", crate::l10n::available(), &crate::l10n::active_name()),
            Field::check("Reshape RTL text", cfg.reshape_rtl),
            // --- Edit/View ---
            Field::text("External editor", cfg.editor.clone()),
            Field::text("External viewer", cfg.viewer.clone()),
            Field::check("Use internal viewer", cfg.use_internal_viewer),
            Field::check("Use internal editor", cfg.use_internal_editor),
            // --- Visual ---
            Field::choice("Theme", crate::ui::theme::palette_names(), &cfg.theme),
            Field::check("Truecolor (gradients)", truecolor),
            Field::check("Animations", cfg.animation),
            Field::check("System status widget", cfg.system_status),
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
            Field::choice(
                "Brief view columns",
                (1..=6).map(|n| n.to_string()).collect(),
                &cfg.brief_columns.to_string(),
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

    /// Build the checksum options form for `path`: pick an algorithm and,
    /// optionally, paste a checksum to compare the result against. The file name
    /// is shown in the title.
    pub fn checksum(path: VfsPath) -> Self {
        let form = Form::new(vec![
            Field::choice("Algorithm", ChecksumKind::labels(), ChecksumKind::Sha256.label()),
            Field::text("Compare to (optional)", ""),
        ]);
        FormDialog {
            title: format!("Checksum: {}", path.file_name()),
            form,
            purpose: FormPurpose::Checksum(path),
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
    /// Collect the options for mirroring the active panel's directory onto the
    /// other one. `src` / `dst` are the two directories, shown so it is obvious
    /// which way round the sync runs before anything is planned.
    pub fn sync(src: &str, dst: &str) -> Self {
        // The option strings stay in English on purpose: `sync_mode_of` maps the
        // picked value back to a mode by matching them, and a Choice renders its
        // options raw (only the field *label* is translated) — the same deal as
        // the checksum-algorithm and filesystem pickers.
        let modes: Vec<String> = SYNC_MODES.iter().map(|s| s.to_string()).collect();
        FormDialog::from_fields(
            "Synchronize",
            vec![Field::choice("Mode", modes, SYNC_MODES[0])],
            FormPurpose::Sync,
        )
        // The title carries the direction: it is the one thing the user must get
        // right, and it is longer than a field label should be.
        .titled(format!("{}:  {src}  →  {dst}", crate::l10n::trd("Synchronize")))
    }

    /// Replace the dialog title (kept out of `from_fields` so the common case
    /// stays a plain translated key).
    fn titled(mut self, title: String) -> Self {
        self.title = title;
        self
    }

    /// A plain form: a title, its fields, and what to do on submit.
    fn from_fields(title: &str, fields: Vec<Field>, purpose: FormPurpose) -> Self {
        FormDialog {
            title: title.to_string(),
            form: Form::new(fields),
            purpose,
            connect: None,
        }
    }

    // --- Guided Git dialogs ------------------------------------------------
    //
    // Each collects the options for one git command; `Submit::GitRun` carries the
    // argv they build (see the `FormPurpose::Git` arm in `handle_key`). Field
    // order is load-bearing — the argv builders read them positionally.

    /// Commit the index: message, plus the two flags people reach for most.
    pub fn git_commit() -> Self {
        FormDialog::from_fields(
            "Commit",
            vec![
                Field::text("Message", ""),
                Field::check("Stage all tracked changes (-a)", false),
                Field::check("Amend the last commit", false),
            ],
            FormPurpose::Git(GitForm::Commit),
        )
    }

    /// Clone a URL into the panel's directory. An empty target lets git name it.
    pub fn git_clone() -> Self {
        FormDialog::from_fields(
            "Clone",
            vec![Field::text("Repository URL", ""), Field::text("Into directory (optional)", "")],
            FormPurpose::Git(GitForm::Clone),
        )
    }

    pub fn git_fetch(remotes: Vec<String>) -> Self {
        let all = remotes.len() > 1;
        FormDialog::from_fields(
            "Fetch",
            vec![
                Field::check("All remotes (--all)", all),
                Field::check("Prune deleted branches (--prune)", false),
            ],
            FormPurpose::Git(GitForm::Fetch),
        )
    }

    pub fn git_pull() -> Self {
        FormDialog::from_fields(
            "Pull",
            vec![Field::check("Rebase instead of merge (--rebase)", false)],
            FormPurpose::Git(GitForm::Pull),
        )
    }

    /// Push the current branch. `remotes` populates the dropdown; the force flags
    /// are off by default and `--force-with-lease` is offered above the raw one.
    pub fn git_push(remotes: Vec<String>, branch: String) -> Self {
        let first = remotes.first().cloned().unwrap_or_default();
        FormDialog::from_fields(
            "Push",
            vec![
                Field::choice("Remote", remotes, &first),
                Field::check("Set upstream (--set-upstream)", false),
                Field::check("Force with lease (safer)", false),
                Field::check("Force (overwrites the remote!)", false),
            ],
            FormPurpose::Git(GitForm::Push { branch }),
        )
    }

    /// Switch branches. The dropdown lists local then remote-tracking branches
    /// (current first); filling in the name field creates a new branch instead.
    pub fn git_checkout(branches: Vec<String>) -> Self {
        let first = branches.first().cloned().unwrap_or_default();
        FormDialog::from_fields(
            "Checkout",
            vec![
                Field::choice("Branch", branches, &first),
                Field::text("…or create a new branch named", ""),
            ],
            FormPurpose::Git(GitForm::Checkout),
        )
    }

    /// Reset the current branch. Modes are ordered least- to most-destructive so
    /// the dialog opens on the safe one.
    pub fn git_reset() -> Self {
        let modes: Vec<String> = RESET_MODES.iter().map(|s| s.to_string()).collect();
        FormDialog::from_fields(
            "Reset",
            vec![
                Field::choice("Mode", modes, RESET_MODES[0]),
                Field::text("Target commit", "HEAD"),
            ],
            FormPurpose::Git(GitForm::Reset),
        )
    }

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
        let mut fields = vec![
            Field::text("Host", ""),
            Field::text("Port", protocol.default_port().to_string()),
            Field::text("Username", ""),
            Field::password("Password"),
            Field::text("Remote path (blank = home)", ""),
        ];
        // Passive mode (PASV) is a plain-FTP concept; SFTP/SCP tunnel their data
        // over the single SSH connection, so the checkbox is FTP-only.
        if matches!(protocol, Protocol::Ftp) {
            fields.push(Field::check("Passive mode (PASV)", true));
        }
        let form = Form::new(fields);
        // Only this protocol's recent connections.
        let history: Vec<_> = history
            .into_iter()
            .filter(|e| e.protocol == protocol.scheme_prefix())
            .collect();
        FormDialog {
            // The proto prefix stays literal; the word is translated (the title
            // is passed through `trd` again at render, harmlessly, for RTL shaping).
            title: format!(
                "{} {}",
                protocol.scheme_prefix().to_uppercase(),
                crate::l10n::tr("connection")
            ),
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

    /// Build a connect form already filled in from a stored remote (its protocol,
    /// host, port, user and path), focused on the password field — for the command
    /// palette's "reconnect to a saved server" entries. `None` if the stored
    /// protocol string is unrecognized.
    pub fn connect_from(entry: &crate::config::RemoteHistoryEntry, side: usize) -> Option<Self> {
        let protocol = match entry.protocol.as_str() {
            "sftp" => Protocol::Sftp,
            "ftp" => Protocol::Ftp,
            "scp" => Protocol::Scp,
            _ => return None,
        };
        let mut dlg = FormDialog::connect(protocol, side, vec![entry.clone()]);
        dlg.apply_history(0);
        Some(dlg)
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
        // Restore the remembered PASV choice (FTP forms only have this checkbox).
        if let Some(Field::Check { value, .. }) = self.form.fields.get_mut(5) {
            *value = entry.passive;
        }
        self.form.focus = 3; // password
    }

    /// Move focus onto the OK (`primary`) or Cancel button slot. Used when the
    /// mouse clicks a button so the synthetic Enter/Esc submits or cancels the
    /// form rather than acting on the field that happened to be focused (e.g.
    /// opening a Choice dropdown). Also closes any open Choice dropdown.
    pub(crate) fn focus_button(&mut self, primary: bool) {
        for field in &mut self.form.fields {
            if let Field::Choice { open, .. } = field {
                *open = false;
            }
        }
        self.form.focus = if primary { self.form.ok_slot() } else { self.form.cancel_slot() };
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
                // Indices follow the grouped field order built in `settings()`.
                language: fields[0].as_text().to_string(),
                reshape_rtl: fields[1].as_bool(),
                editor: fields[2].as_text().trim().to_string(),
                viewer: fields[3].as_text().trim().to_string(),
                use_internal_viewer: fields[4].as_bool(),
                use_internal_editor: fields[5].as_bool(),
                theme: fields[6].as_text().to_string(),
                truecolor: fields[7].as_bool(),
                animation: fields[8].as_bool(),
                system_status: fields[9].as_bool(),
                graphics: graphics_pref(fields[10].as_text()),
                brief_columns: fields[11].as_text().parse().unwrap_or(2).clamp(1, 6),
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
            FormPurpose::Checksum(path) => Submit::Checksum {
                path: path.clone(),
                kind: ChecksumKind::from_label(fields[0].as_text()).unwrap_or(ChecksumKind::Sha256),
                expected: fields[1].as_text().trim().to_string(),
            },
            // Each guided git form builds its own argv here, so the app just runs
            // it. Field indices mirror the constructors above.
            FormPurpose::Git(kind) => {
                use crate::git::ops;
                match kind {
                    GitForm::Commit => {
                        let msg = fields[0].as_text().trim().to_string();
                        // Git would reject an empty message anyway; say so here
                        // rather than opening an output box on a certain failure.
                        if msg.is_empty() {
                            return DialogResult::Cancel;
                        }
                        Submit::GitRun {
                            title: "commit".into(),
                            args: ops::commit_args(&msg, fields[1].as_bool(), fields[2].as_bool()),
                        }
                    }
                    GitForm::Clone => {
                        let url = fields[0].as_text().trim().to_string();
                        if url.is_empty() {
                            return DialogResult::Cancel;
                        }
                        Submit::GitRun {
                            title: "clone".into(),
                            args: ops::clone_args(&url, fields[1].as_text()),
                        }
                    }
                    GitForm::Fetch => Submit::GitRun {
                        title: "fetch".into(),
                        args: ops::fetch_args(fields[0].as_bool(), fields[1].as_bool()),
                    },
                    GitForm::Pull => Submit::GitRun {
                        title: "pull".into(),
                        args: ops::pull_args(fields[0].as_bool()),
                    },
                    GitForm::Push { branch } => Submit::GitRun {
                        title: "push".into(),
                        args: ops::push_args(
                            fields[0].as_text(),
                            branch,
                            fields[3].as_bool(),
                            fields[2].as_bool(),
                            fields[1].as_bool(),
                        ),
                    },
                    GitForm::Checkout => {
                        // A typed name creates that branch; otherwise switch to
                        // the one picked from the dropdown.
                        let new = fields[1].as_text().trim().to_string();
                        let (target, create) = if new.is_empty() {
                            (fields[0].as_text().to_string(), false)
                        } else {
                            (new, true)
                        };
                        if target.is_empty() {
                            return DialogResult::Cancel;
                        }
                        Submit::GitRun {
                            title: "checkout".into(),
                            args: ops::checkout_args(&target, create),
                        }
                    }
                    GitForm::Reset => Submit::GitRun {
                        title: "reset".into(),
                        args: ops::reset_args(reset_mode_name(fields[0].as_text()), fields[1].as_text()),
                    },
                }
            }
            // The app plans the sync in the background, then previews it.
            FormPurpose::Sync => Submit::SyncPlan(sync_mode_of(fields[0].as_text())),
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
                        // The PASV checkbox exists only on FTP forms; SFTP/SCP
                        // ignore the value, so a missing field is fine.
                        passive: fields.get(5).map(Field::as_bool).unwrap_or(false),
                    },
                )
            }
        };
        DialogResult::Submit(submit)
    }

    /// The dialog's outer box size for the current form. The Settings form is
    /// wider and taller to fit its three bordered group boxes; every other form
    /// keeps the compact one-row-per-field box.
    fn outer_dims(&self, area: Rect) -> (u16, u16) {
        if matches!(self.purpose, FormPurpose::Settings) {
            // Each group box = its fields + 2 border rows; plus a spacer and the
            // hint/button row inside, and the outer border.
            let group_rows: u16 = SETTINGS_GROUPS.iter().map(|(_, c)| *c as u16 + 2).sum();
            let height = group_rows + 1 /* spacer */ + 1 /* hint */ + 2 /* border */;
            let w = 72u16.min(area.width.saturating_sub(4));
            (w, height)
        } else {
            let height = self.form.fields.len() as u16 + 4;
            let w = 60u16.min(area.width.saturating_sub(4));
            (w, height)
        }
    }

    /// The centered outer box rect (used by `render` and by click hit-testing).
    pub(crate) fn outer_rect(&self, area: Rect) -> Rect {
        let (w, h) = self.outer_dims(area);
        centered(area, w, h)
    }

    /// For the Settings form, the three group boxes (title + rect) laid out
    /// vertically inside `inner`.
    fn group_boxes(inner: Rect) -> Vec<(&'static str, Rect)> {
        let mut boxes = Vec::with_capacity(SETTINGS_GROUPS.len());
        let mut y = inner.y;
        for (title, count) in SETTINGS_GROUPS {
            let box_h = *count as u16 + 2;
            boxes.push((*title, Rect { x: inner.x, y, width: inner.width, height: box_h }));
            y += box_h;
        }
        boxes
    }

    /// The on-screen row rect for each field. Settings rows sit inside their
    /// group box (inset by the border); other forms stack one row per field.
    fn field_rows(&self, inner: Rect) -> Vec<Rect> {
        if matches!(self.purpose, FormPurpose::Settings) {
            let mut rows = Vec::with_capacity(self.form.fields.len());
            for (_, brect) in Self::group_boxes(inner) {
                let inner_box = Rect {
                    x: brect.x + 1,
                    y: brect.y + 1,
                    width: brect.width.saturating_sub(2),
                    height: brect.height.saturating_sub(2),
                };
                for k in 0..inner_box.height {
                    rows.push(Rect { y: inner_box.y + k, height: 1, ..inner_box });
                }
            }
            rows
        } else {
            (0..self.form.fields.len())
                .map(|i| Rect { y: inner.y + i as u16, height: 1, ..inner })
                .collect()
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let rect = self.outer_rect(area);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        // The Settings dialog doubles as the "about" surface: append the program
        // name and version to its title. The version isn't translated.
        let title = if matches!(self.purpose, FormPurpose::Settings) {
            format!(
                "{} — Rat Commander {}",
                crate::l10n::trd(&self.title),
                env!("CARGO_PKG_VERSION")
            )
        } else {
            crate::l10n::trd(&self.title)
        };
        let block = dialog_block(&title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let focus_style = theme.dialog_selection;

        // Settings groups its fields into three titled sub-boxes; other forms are
        // a flat one-row-per-field column. `field_rows` maps each field index to
        // its on-screen row either way.
        if matches!(self.purpose, FormPurpose::Settings) {
            for (title, brect) in Self::group_boxes(inner) {
                let gblock = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.dialog_border_fg).bg(theme.dialog_bg))
                    .title(Span::styled(
                        format!(" {} ", crate::l10n::trd(title)),
                        Style::default().fg(theme.dialog_title).bg(theme.dialog_bg),
                    ))
                    .style(base);
                f.render_widget(gblock, brect);
            }
        }
        let rows = self.field_rows(inner);

        // The Host field of a connect form gets a ▼ chevron to open the history.
        let connect_host = self.connect.as_ref().is_some_and(|c| !c.history.is_empty());
        let mut host_chevron: Option<Rect> = None;

        let mut caret: Option<Position> = None;
        for (i, field) in self.form.fields.iter().enumerate() {
            let row = rows[i];
            let y = row.y;
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

        let choice_open = self.form.fields.iter().any(|f| matches!(f, Field::Choice { open: true, .. }));

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
        let mut gfx = gfx;
        let ok_txt = crate::l10n::tr("OK");
        let cancel_txt = crate::l10n::tr("Cancel");
        // Graphical buttons only when the font can render the labels; otherwise
        // fall back to the text button row (terminal font handles any script).
        if gfx.as_deref().is_some_and(|g| g.buttons_ok()) && all_renderable(&[&ok_txt, &cancel_txt]) {
            // Graphical buttons: OK at the left, Cancel at the right, with the
            // navigation hint between them. Left/right halves still hit-test OK/Cancel.
            let ok_w = 10u16.min(hint.width);
            let cancel_w = 12u16.min(hint.width.saturating_sub(ok_w));
            let ok_rect = Rect { x: hint.x, y: hint.y, width: ok_w, height: 1 };
            let cancel_rect =
                Rect { x: hint.x + hint.width - cancel_w, y: hint.y, width: cancel_w, height: 1 };
            let mid_x = ok_rect.x + ok_rect.width + 1;
            let mid_w = cancel_rect.x.saturating_sub(mid_x);
            if mid_w > 0 {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!("Tab/↑↓ move  Space toggle{extra}"),
                        base,
                    )))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                    Rect { x: mid_x, y: hint.y, width: mid_w, height: 1 },
                );
            }
            gfx_button(
                f,
                gfx.as_deref_mut(),
                Slot::Button(0),
                ok_rect,
                &ok_txt,
                self.form.on_ok(),
                theme,
            );
            gfx_button(
                f,
                gfx,
                Slot::Button(1),
                cancel_rect,
                &cancel_txt,
                self.form.on_cancel(),
                theme,
            );
        } else {
            // Text buttons are laid out exactly like the graphical ones above —
            // OK against the left edge, Cancel against the right, the hint
            // centered in the gap. (Rendering them as one left-aligned line
            // instead would pile all the slack up on the right.)
            let ok_txt = crate::l10n::trd("OK");
            let cancel_txt = crate::l10n::trd("Cancel");
            let label = |text: &str, focused: bool| {
                if focused { format!("[< {text} >]") } else { format!("[  {text}  ]") }
            };
            let ok_label = label(&ok_txt, self.form.on_ok());
            let cancel_label = label(&cancel_txt, self.form.on_cancel());
            let ok_w = (ok_label.chars().count() as u16).min(hint.width);
            let cancel_w =
                (cancel_label.chars().count() as u16).min(hint.width.saturating_sub(ok_w));
            let cancel_x = hint.x + hint.width - cancel_w;
            let styled = |text: String, focused: bool| {
                let style = if focused { theme.button_focused } else { theme.button };
                Paragraph::new(Line::from(Span::styled(text, style))).style(base)
            };
            f.render_widget(
                styled(ok_label, self.form.on_ok()),
                Rect { x: hint.x, y: hint.y, width: ok_w, height: 1 },
            );
            f.render_widget(
                styled(cancel_label, self.form.on_cancel()),
                Rect { x: cancel_x, y: hint.y, width: cancel_w, height: 1 },
            );
            let mid_x = hint.x + ok_w;
            let mid_w = cancel_x.saturating_sub(mid_x);
            if mid_w > 0 {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!("Tab/↑↓ move  Space toggle{extra}"),
                        base,
                    )))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                    Rect { x: mid_x, y: hint.y, width: mid_w, height: 1 },
                );
            }
        }

        // An open Choice dropdown is drawn last, so it overlays everything it
        // spills across — the button/hint row and the dialog border included.
        // (It is sized against the screen, not the dialog, so a long list can
        // reach well past the box; see `choice_dropdown_geom`.) The scroll offset
        // `top` is nudged only when the highlight leaves the window, so the cursor
        // moves freely within it.
        for (i, field) in self.form.fields.iter_mut().enumerate() {
            if let Field::Choice { options, sel, top, open: true, .. } = field {
                let field_y = rows[i].y;
                let visible = choice_visible_rows(inner, area, field_y, options.len());
                *top = crate::util::scroll::scroll_to_visible(*top, *sel, visible);
                render_choice_dropdown(f, inner, area, field_y, options, *sel, *top, theme);
            }
        }

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
        let rect = self.outer_rect(area);
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
        let rows = self.field_rows(inner);
        // An open dropdown: pick the clicked option, or close on an outside click.
        if let Some(fi) = self
            .form
            .fields
            .iter()
            .position(|f| matches!(f, Field::Choice { open: true, .. }))
        {
            let field_y = rows[fi].y;
            if let Some(Field::Choice { options, idx, open, sel, top, .. }) = self.form.fields.get_mut(fi) {
                let (rect, visible) = choice_dropdown_geom(inner, area, field_y, options.len());
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
            let r = rows[i];
            let on_row = row == r.y && col >= r.x && col < r.x + r.width;
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

    /// Route a click onto a Text/Password/Check field row: focus a text field and
    /// place its caret under the pointer, or focus and toggle a checkbox. Returns
    /// `Some` when a field row was hit. Choice rows are left to `click_choice`
    /// (which opens their dropdown), and the OK/Cancel row to the button handler.
    pub(crate) fn click_field(&mut self, area: Rect, col: u16, row: u16) -> Option<DialogResult> {
        let inner = self.dialog_inner(area);
        let rows = self.field_rows(inner);
        let i = rows.iter().position(|r| row == r.y && col >= r.x && col < r.x + r.width)?;
        let r = rows[i];
        match self.form.fields.get_mut(i)? {
            Field::Check { value, .. } => {
                *value = !*value;
                self.form.focus = i;
                Some(DialogResult::None)
            }
            Field::Text { label, value, cursor } | Field::Password { label, value, cursor } => {
                // Place the caret under the click, mirroring the label width and
                // horizontal scroll used by `render`/`draw_input_field`.
                let label_str = crate::l10n::display(&format!("{}: ", crate::l10n::tr(label)));
                let lw = (label_str.chars().count() as u16).min(r.width);
                let value_x = r.x + lw;
                let char_count = value.chars().count();
                if col >= value_x {
                    let field_w = r.width.saturating_sub(lw) as usize;
                    let inner_w = field_w.saturating_sub(3); // room for the "[^]" affordance
                    let start = cursor.saturating_sub(inner_w.saturating_sub(1));
                    *cursor = (start + (col - value_x) as usize).min(char_count);
                }
                self.form.focus = i;
                Some(DialogResult::None)
            }
            // A Choice row opens via `click_choice`, not here.
            Field::Choice { .. } => None,
        }
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
                format!(" {} ", crate::l10n::trd("Recent")),
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
///
/// It is sized against the whole `screen`, not the dialog interior, so a long
/// list (say, every branch in a repository) is not squeezed into the few rows a
/// small dialog happens to have — it overlays the dialog's border and whatever is
/// behind it. Horizontally it stays aligned with `inner`, under its own field.
fn choice_dropdown_geom(inner: Rect, screen: Rect, field_y: u16, options_len: usize) -> (Rect, usize) {
    let below = (screen.y + screen.height).saturating_sub(field_y + 1) as usize; // rows under the field
    let above = field_y.saturating_sub(screen.y) as usize; // rows over the field
    let want = options_len + 2; // options + top/bottom border
    // Prefer dropping down; flip up only when down can't fit and up has more room.
    let open_up = below < want && above > below;
    let room = if open_up { above } else { below };
    let visible = options_len.min(room.saturating_sub(2).max(1)).max(1);
    let box_h = (visible + 2) as u16;
    let y = if open_up { field_y.saturating_sub(box_h) } else { field_y + 1 };
    (Rect { x: inner.x, y, width: inner.width, height: box_h }, visible)
}

/// Number of option rows visible in a Choice dropdown whose field is at `field_y`.
fn choice_visible_rows(inner: Rect, screen: Rect, field_y: u16, options_len: usize) -> usize {
    choice_dropdown_geom(inner, screen, field_y, options_len).1
}


/// Draw a Choice field's scrollable dropdown just below its row (at `field_y`),
/// showing options from `top` with `sel` highlighted.
#[allow(clippy::too_many_arguments)]
fn render_choice_dropdown(
    f: &mut Frame,
    inner: Rect,
    screen: Rect,
    field_y: u16,
    options: &[String],
    sel: usize,
    top: usize,
    theme: &Theme,
) {
    if options.is_empty() {
        return;
    }
    let (rect, visible) = choice_dropdown_geom(inner, screen, field_y, options.len());
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

#[cfg(test)]
mod choice_geom_tests {
    use super::*;

    #[test]
    fn dropdown_extends_past_the_dialog_onto_the_screen() {
        // A short dialog (7 rows) near the top of a tall screen, field on row 3.
        let screen = Rect::new(0, 0, 80, 40);
        let inner = Rect::new(10, 2, 40, 5);
        let (rect, visible) = choice_dropdown_geom(inner, screen, 3, 30);
        // All 30 options fit below the field on this screen, even though the
        // dialog interior ends at row 7 — the list is not clipped to the box.
        assert_eq!(visible, 30, "the whole list is shown, not just the dialog's rows");
        assert!(
            rect.y + rect.height > inner.y + inner.height,
            "the dropdown overhangs the dialog interior"
        );
        assert!(rect.y + rect.height <= screen.y + screen.height, "but stays on screen");
        // It stays aligned with its field horizontally.
        assert_eq!((rect.x, rect.width), (inner.x, inner.width));
    }

    #[test]
    fn dropdown_flips_above_the_field_when_below_is_cramped() {
        // Field near the bottom of the screen: more room above than below.
        let screen = Rect::new(0, 0, 80, 24);
        let inner = Rect::new(5, 16, 40, 6);
        let (rect, visible) = choice_dropdown_geom(inner, screen, 21, 20);
        assert!(rect.y < 21, "opens upward");
        assert!(visible >= 1);
        assert!(rect.y >= screen.y, "stays on screen");
    }

    #[test]
    fn dropdown_is_clamped_to_the_screen_not_the_dialog() {
        // A huge list on a short screen: bounded by the screen's rows.
        let screen = Rect::new(0, 0, 80, 12);
        let inner = Rect::new(0, 1, 30, 4);
        let (rect, visible) = choice_dropdown_geom(inner, screen, 2, 500);
        assert!(visible < 500, "clamped");
        assert!(rect.y + rect.height <= screen.y + screen.height, "never runs off screen");
        assert!(visible >= 1, "always shows at least one option");
    }
}
