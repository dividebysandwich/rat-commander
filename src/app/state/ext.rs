//! `rc.ext` action dispatch: mount extfs scripts and run Open/View/Edit
//! commands for the file under the cursor. See [`crate::ext`] for the file
//! format and [`crate::vfs::extfs`] for the extfs backend.

use super::*;
use crate::vfs::extfs::{find_extfs_script, ExtfsFs};
use std::path::Path;
use std::sync::Arc;

impl AppState {
    /// The `rc.ext` rule that applies to the file under the cursor, if any.
    /// Only local files are considered (macros/commands assume a real path).
    fn ext_entry_under_cursor(&self) -> Option<&crate::ext::ExtEntry> {
        let p = &self.panels[self.active];
        if p.cwd.scheme != "file" {
            return None;
        }
        let e = p.current_entry()?;
        if e.kind != VfsKind::File {
            return None;
        }
        self.ext_rules.lookup(&e.name)
    }

    /// The (owned) command template for an rc.ext action key on the file under
    /// the cursor. Owned so the immutable borrow of `self` is released before
    /// the caller mutates `self`.
    fn ext_action(&self, key: &str) -> Option<String> {
        self.ext_entry_under_cursor()?.action(key).map(String::from)
    }

    /// Enter handler for an rc.ext `Open` action. Returns `Some(flow)` when a
    /// rule handled it (extfs mount or a plain command), `None` to fall through
    /// to the default open behaviour.
    pub(in crate::app::state) async fn try_ext_open(&mut self) -> Option<Flow> {
        let open = self.ext_action("Open")?;
        match parse_cd_extfs(&open) {
            Some(prefix) => {
                self.enter_extfs(prefix.to_string()).await;
                Some(Flow::Continue)
            }
            None => Some(Flow::RunCommand(self.expand_macros(&open))),
        }
    }

    /// F3 handler for an rc.ext `View` action: `%view{…} cmd` pipes the command
    /// output into the built-in viewer; a plain command runs in the foreground.
    pub(in crate::app::state) fn try_ext_view(&mut self, name: &str) -> Option<Flow> {
        let view = self.ext_action("View")?;
        match parse_view(&view) {
            Some(cmd) => {
                let expanded = self.expand_macros(cmd);
                self.start_command_capture(name.to_string(), expanded);
                Some(Flow::Continue)
            }
            None => Some(Flow::RunCommand(self.expand_macros(&view))),
        }
    }

    /// F4 handler for an rc.ext `Edit` action: run the command in the foreground.
    pub(in crate::app::state) fn try_ext_edit(&mut self) -> Option<Flow> {
        let edit = self.ext_action("Edit")?;
        Some(Flow::RunCommand(self.expand_macros(&edit)))
    }

    /// Mount the file under the cursor with the extfs script `prefix` and enter
    /// it. The backend is registered lazily (scheme = the prefix) and stays
    /// registered afterwards, like a remote session.
    pub(in crate::app::state) async fn enter_extfs(&mut self, prefix: String) {
        let container = {
            let p = &self.panels[self.active];
            let Some(e) = p.current_entry() else {
                return;
            };
            p.cwd.path.join(&e.name)
        };
        let probe = VfsPath::extfs(&prefix, container, "/");
        if self.registry.resolve(&probe).is_err() {
            let Some(script) = find_extfs_script(&prefix) else {
                return self.show_error(format!(
                    "No extfs script '{prefix}' found (install it in \
                     ~/.local/share/mc/extfs.d or /usr/lib/mc/extfs.d)"
                ));
            };
            self.registry
                .register(prefix.clone(), Arc::new(ExtfsFs::new(prefix, script)));
        }
        let backend = match self.registry.resolve(&probe) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("Cannot open location: {e}")),
        };
        self.active_panel().try_enter(probe, backend, None).await;
    }

    /// Run `cmd` (in the active panel's directory), capture its output to a temp
    /// file, and open the built-in viewer on it — reusing the fetch-to-temp
    /// viewer path (the temp is deleted when the viewer closes). A cancellable
    /// "Running" progress dialog is shown meanwhile.
    pub(in crate::app::state) fn start_command_capture(&mut self, name: String, cmd: String) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(
            id,
            TaskHandle {
                id,
                cancel: cancel.clone(),
                reply,
            },
        );
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Running")));

        let cwd = self.console_cwd();
        let dir = if cwd.scheme == "file" {
            cwd.path.clone()
        } else {
            std::env::temp_dir()
        };
        let temp = crate::util::temp::rc_temp_path("extview");
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match run_capture(&cmd, &dir, &temp, &cancel).await {
                Ok(true) => {
                    let _ = tx
                        .send(AppEvent::FileFetched {
                            id,
                            kind: FetchKind::View,
                            name,
                            orig_path: VfsPath::local(&temp),
                            temp,
                        })
                        .await;
                }
                Ok(false) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ = tx
                        .send(AppEvent::TaskDone {
                            id,
                            outcome: TaskOutcome::Cancelled,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ = tx
                        .send(AppEvent::TaskDone {
                            id,
                            outcome: TaskOutcome::Failed(e),
                        })
                        .await;
                }
            }
        });
    }
}

/// Parse an `Open=%cd <path>/<prefix>://` action, returning the extfs `prefix`
/// (the script name). The container is taken from the file under the cursor, so
/// the path portion of the template is not used here.
pub(in crate::app::state) fn parse_cd_extfs(open: &str) -> Option<&str> {
    let rest = open.trim().strip_prefix("%cd ")?.trim();
    let head = rest.strip_suffix("://")?;
    let prefix = head.rsplit('/').next()?;
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

/// Parse a `View=%view{ascii|hex} cmd` action, returning the command to run and
/// capture into the built-in viewer. Returns `None` when there is no `%view{…}`
/// prefix (the caller then runs the command in the foreground). The `{…}` mode
/// is accepted but the viewer opens in its default (text) mode; F4 toggles hex.
pub(in crate::app::state) fn parse_view(view: &str) -> Option<&str> {
    let rest = view.trim().strip_prefix("%view")?;
    // Optional `{ascii}` / `{hex}` selector.
    let rest = match rest.strip_prefix('{') {
        Some(after) => after.split_once('}')?.1,
        None => rest,
    };
    let cmd = rest.trim();
    if cmd.is_empty() {
        None
    } else {
        Some(cmd)
    }
}

/// Run `cmd` via the shell in `dir`, writing its output to `temp`. Returns
/// `Ok(true)` on completion, `Ok(false)` if cancelled. stderr is used only when
/// stdout is empty, so error text from a failed helper is still shown.
async fn run_capture(
    cmd: &str,
    dir: &Path,
    temp: &Path,
    cancel: &CancelToken,
) -> Result<bool, String> {
    let mut command = build_shell(cmd);
    command
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command
        .spawn()
        .map_err(|e| format!("cannot run command: {e}"))?;

    let out = tokio::select! {
        r = child.wait_with_output() => r.map_err(|e| e.to_string())?,
        _ = cancel.cancelled() => return Ok(false),
    };
    let data = if out.stdout.is_empty() {
        out.stderr
    } else {
        out.stdout
    };
    tokio::fs::write(temp, &data)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// A shell command builder matching the app's foreground runner (`sh -c` /
/// `cmd /C`).
fn build_shell(cmd: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cd_extfs, parse_view};

    #[test]
    fn cd_extfs_extracts_prefix() {
        assert_eq!(parse_cd_extfs("%cd %p/uzip://"), Some("uzip"));
        assert_eq!(parse_cd_extfs("%cd %p/iso9660://"), Some("iso9660"));
        assert_eq!(parse_cd_extfs("  %cd  %p/rpm://  "), Some("rpm"));
        assert_eq!(parse_cd_extfs("%cd rpm://"), Some("rpm"));
    }

    #[test]
    fn cd_extfs_rejects_non_cd() {
        assert_eq!(parse_cd_extfs("unzip %f"), None);
        assert_eq!(parse_cd_extfs("%cd %p/somedir"), None); // no ://
        assert_eq!(parse_cd_extfs("%view{ascii} unzip -v %f"), None);
    }

    #[test]
    fn view_strips_prefix_and_mode() {
        assert_eq!(parse_view("%view{ascii} unzip -v %f"), Some("unzip -v %f"));
        assert_eq!(parse_view("%view{hex} xxd %f"), Some("xxd %f"));
        assert_eq!(parse_view("%view cat %f"), Some("cat %f"));
    }

    #[test]
    fn view_rejects_plain_command() {
        assert_eq!(parse_view("less %f"), None);
        assert_eq!(parse_view("%view{ascii}"), None); // no command
    }
}
