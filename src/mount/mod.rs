//! Disk manager: a full-screen tool listing block devices as a disk→partition
//! tree (left) and current mounts (right), with mount / unmount / sync / format
//! actions. Privileged operations (mount/unmount/format) need root; when the
//! program isn't running as root it shells out through `sudo` (non-interactively
//! when possible, otherwise prompting for a password the app feeds to `sudo -S`).
//!
//! Device/mount enumeration reads Linux `/proc`+`/sys`; on other platforms the
//! lists are simply empty (and the tool isn't offered in the menu).

pub mod render;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

/// Two left clicks on the same row within this window count as a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// A block device (whole disk or partition).
#[derive(Debug, Clone, Default)]
pub struct BlockDevice {
    /// Kernel name, e.g. `sda1`.
    pub name: String,
    /// Device node, e.g. `/dev/sda1`.
    pub dev: String,
    /// Size in bytes.
    pub size: u64,
    /// Filesystem type (from udev / the active mount; blank if unknown).
    pub fstype: String,
    /// Filesystem volume label, if any.
    pub label: String,
    /// Where it is currently mounted, if at all.
    pub mountpoint: Option<String>,
    /// Parent whole-disk name when this is a partition (`None` for whole disks).
    pub parent: Option<String>,
    /// Manufacturer / vendor (from the parent disk's sysfs), if known.
    pub vendor: String,
    /// Model name (from the parent disk's sysfs), if known.
    pub model: String,
    /// Serial number (from the parent disk's sysfs), if known.
    pub serial: String,
    /// Whether the backing disk is removable (sysfs `removable` == 1). Flashing a
    /// non-removable device is flagged as extra-dangerous.
    pub removable: bool,
}

/// One active mount of a block device.
#[derive(Debug, Clone)]
pub struct MountEntry {
    pub dev: String,
    pub mountpoint: String,
    pub fstype: String,
}

/// Which side of the tool has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Devices,
    Mounts,
}

/// What handling a key asks the app to do.
pub enum MountSignal {
    Stay,
    Close,
    /// Open the action menu for this block device (Mount/Format or Unmount).
    DeviceMenu(Box<BlockDevice>),
    /// Open the action menu for this mount point (Unmount/Sync).
    MountMenu(String),
    /// Unmount this mount point (the `u` shortcut; the app confirms).
    Unmount(String),
}

/// A filesystem the disk formatter can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    Fat32,
    Ntfs,
    Vfat,
    Fat16,
    Ext4,
    Btrfs,
    Ext3,
    Ext2,
}

impl FsType {
    /// In menu order (most common first).
    pub const ALL: [FsType; 8] = [
        FsType::Fat32,
        FsType::Ntfs,
        FsType::Vfat,
        FsType::Fat16,
        FsType::Ext4,
        FsType::Btrfs,
        FsType::Ext3,
        FsType::Ext2,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FsType::Fat32 => "FAT32",
            FsType::Ntfs => "NTFS",
            FsType::Vfat => "VFAT",
            FsType::Fat16 => "FAT16",
            FsType::Ext4 => "EXT4",
            FsType::Btrfs => "BTRFS",
            FsType::Ext3 => "EXT3",
            FsType::Ext2 => "EXT2",
        }
    }

    pub fn from_label(s: &str) -> Option<FsType> {
        FsType::ALL.into_iter().find(|f| f.label() == s)
    }

    pub fn is_ext(self) -> bool {
        matches!(self, FsType::Ext4 | FsType::Ext3 | FsType::Ext2)
    }
}

/// A requested format operation, collected from the formatter dialog.
#[derive(Debug, Clone)]
pub struct FormatSpec {
    pub dev: String,
    pub fs: FsType,
    /// Volume label (empty = none).
    pub label: String,
    /// Quick format (NTFS only).
    pub quick: bool,
    /// Bytes-per-inode for ext filesystems (empty = mkfs default).
    pub inode_bytes: String,
}

/// Build the `mkfs` shell command for a format request.
pub fn format_command(s: &FormatSpec) -> String {
    let dev = shell_quote(&s.dev);
    let label = s.label.trim();
    let labeled = |flag: &str| {
        if label.is_empty() {
            String::new()
        } else {
            format!(" {flag} {}", shell_quote(label))
        }
    };
    match s.fs {
        FsType::Fat32 => format!("mkfs.vfat -F 32{} {dev}", labeled("-n")),
        FsType::Fat16 => format!("mkfs.vfat -F 16{} {dev}", labeled("-n")),
        FsType::Vfat => format!("mkfs.vfat{} {dev}", labeled("-n")),
        FsType::Ntfs => {
            let q = if s.quick { " -Q" } else { "" };
            format!("mkfs.ntfs{q} -F{} {dev}", labeled("-L"))
        }
        FsType::Btrfs => format!("mkfs.btrfs -f{} {dev}", labeled("-L")),
        FsType::Ext4 | FsType::Ext3 | FsType::Ext2 => {
            let kind = match s.fs {
                FsType::Ext4 => "ext4",
                FsType::Ext3 => "ext3",
                _ => "ext2",
            };
            let inode = if s.inode_bytes.trim().is_empty() {
                String::new()
            } else {
                format!(" -i {}", shell_quote(s.inode_bytes.trim()))
            };
            format!("mkfs.{kind} -F{}{} {dev}", labeled("-L"), inode)
        }
    }
}

pub struct MountView {
    pub devices: Vec<BlockDevice>,
    pub mounts: Vec<MountEntry>,
    pub focus: Pane,
    pub dev_cursor: usize,
    pub mnt_cursor: usize,
    /// Last operation result/hint, shown on the status line.
    pub status: String,
    /// Visible rows per list, set by the renderer for paging math.
    pub view_rows: usize,
    /// Hit-test geometry recorded by the renderer for mouse mapping: the inner
    /// row area of each list and the index of its first visible row.
    pub dev_hit: ListHit,
    pub mnt_hit: ListHit,
    /// The last left click (pane, row index, when), for double-click detection.
    last_click: Option<(Pane, usize, Instant)>,
}

/// The on-screen geometry of a scrolled list, recorded at render time so a click
/// can be mapped back to an entry index.
#[derive(Clone, Copy, Default)]
pub struct ListHit {
    /// Inner area holding the rows (inside the panel border).
    pub area: Rect,
    /// Index of the entry drawn on the first visible row.
    pub top: usize,
}

impl ListHit {
    /// The entry index at screen point `(col, row)`, if it falls on a row.
    fn index_at(&self, col: u16, row: u16, len: usize) -> Option<usize> {
        let a = self.area;
        if a.width == 0
            || a.height == 0
            || col < a.x
            || col >= a.x + a.width
            || row < a.y
            || row >= a.y + a.height
        {
            return None;
        }
        let idx = self.top + (row - a.y) as usize;
        (idx < len).then_some(idx)
    }

    fn contains(&self, col: u16, row: u16) -> bool {
        let a = self.area;
        a.width > 0
            && a.height > 0
            && col >= a.x
            && col < a.x + a.width
            && row >= a.y
            && row < a.y + a.height
    }
}

impl MountView {
    pub fn new() -> Self {
        let mut v = MountView {
            devices: Vec::new(),
            mounts: Vec::new(),
            focus: Pane::Devices,
            dev_cursor: 0,
            mnt_cursor: 0,
            status: if is_root() {
                "Enter: mount   u: unmount".to_string()
            } else {
                crate::l10n::trd("Mounting needs root — sudo is used (you may be asked for a password)")
            },
            view_rows: 1,
            dev_hit: ListHit::default(),
            mnt_hit: ListHit::default(),
            last_click: None,
        };
        v.refresh();
        v
    }

    /// Re-read the device and mount lists (keeping cursors in range).
    pub fn refresh(&mut self) {
        self.mounts = list_mounts();
        self.devices = list_block_devices(&self.mounts);
        self.clamp();
    }

    fn clamp(&mut self) {
        self.dev_cursor = self.dev_cursor.min(self.devices.len().saturating_sub(1));
        self.mnt_cursor = self.mnt_cursor.min(self.mounts.len().saturating_sub(1));
    }

    pub fn selected_device(&self) -> Option<&BlockDevice> {
        self.devices.get(self.dev_cursor)
    }

    pub fn selected_mount(&self) -> Option<&MountEntry> {
        self.mounts.get(self.mnt_cursor)
    }

    /// The block device whose details to show: the selected device, or (when the
    /// mounts pane has focus) the device backing the selected mount.
    pub fn detail_device(&self) -> Option<&BlockDevice> {
        match self.focus {
            Pane::Devices => self.selected_device(),
            Pane::Mounts => {
                let m = self.selected_mount()?;
                self.devices.iter().find(|d| d.dev == m.dev)
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MountSignal {
        self.status.clear();
        match key.code {
            KeyCode::Esc | KeyCode::F(10) | KeyCode::Char('q') | KeyCode::Char('Q') => {
                return MountSignal::Close;
            }
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.focus = match self.focus {
                    Pane::Devices => Pane::Mounts,
                    Pane::Mounts => Pane::Devices,
                };
            }
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-(self.view_rows as isize).max(1)),
            KeyCode::PageDown => self.move_cursor(self.view_rows as isize),
            KeyCode::Home => self.set_cursor(0),
            KeyCode::End => self.set_cursor(usize::MAX),
            KeyCode::Char('r') | KeyCode::Char('R') => self.refresh(),
            KeyCode::Enter => return self.activate(),
            // Unmount: the selected mount (right pane) or selected device's mount.
            KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Delete | KeyCode::F(8) => {
                let mp = match self.focus {
                    Pane::Mounts => self.selected_mount().map(|m| m.mountpoint.clone()),
                    Pane::Devices => {
                        self.selected_device().and_then(|d| d.mountpoint.clone())
                    }
                };
                if let Some(mp) = mp {
                    return MountSignal::Unmount(mp);
                }
                self.status = crate::l10n::trd("Select a mounted entry to unmount");
            }
            _ => {}
        }
        MountSignal::Stay
    }

    /// Route a mouse event. A left click sets focus + cursor on the clicked
    /// list entry; a double click opens that entry's action menu (like Enter).
    /// The wheel scrolls the list under the pointer.
    pub fn handle_mouse(&mut self, ev: MouseEvent) -> MountSignal {
        let (col, row) = (ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.status.clear();
                // Which list (if any) was hit?
                let (pane, idx) = if let Some(i) =
                    self.dev_hit.index_at(col, row, self.devices.len())
                {
                    (Pane::Devices, i)
                } else if let Some(i) = self.mnt_hit.index_at(col, row, self.mounts.len()) {
                    (Pane::Mounts, i)
                } else {
                    return MountSignal::Stay;
                };
                self.focus = pane;
                *self.cursor_mut() = idx;

                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|(p, i, t)| p == pane && i == idx && now - t < DOUBLE_CLICK);
                if double {
                    self.last_click = None; // don't let a third click re-fire
                    return self.activate();
                }
                self.last_click = Some((pane, idx, now));
                MountSignal::Stay
            }
            MouseEventKind::ScrollDown => {
                self.scroll_pointer(col, row, 1);
                MountSignal::Stay
            }
            MouseEventKind::ScrollUp => {
                self.scroll_pointer(col, row, -1);
                MountSignal::Stay
            }
            _ => MountSignal::Stay,
        }
    }

    /// The action for the focused entry — what Enter would do.
    fn activate(&self) -> MountSignal {
        match self.focus {
            Pane::Devices => match self.selected_device() {
                Some(d) => MountSignal::DeviceMenu(Box::new(d.clone())),
                None => MountSignal::Stay,
            },
            Pane::Mounts => match self.selected_mount() {
                Some(m) => MountSignal::MountMenu(m.mountpoint.clone()),
                None => MountSignal::Stay,
            },
        }
    }

    /// Scroll the list under the pointer (or, failing that, the focused one).
    fn scroll_pointer(&mut self, col: u16, row: u16, delta: isize) {
        let prev = self.focus;
        if self.dev_hit.contains(col, row) {
            self.focus = Pane::Devices;
        } else if self.mnt_hit.contains(col, row) {
            self.focus = Pane::Mounts;
        } else {
            self.focus = prev;
        }
        self.move_cursor(delta);
    }

    fn len(&self) -> usize {
        match self.focus {
            Pane::Devices => self.devices.len(),
            Pane::Mounts => self.mounts.len(),
        }
    }

    fn cursor_mut(&mut self) -> &mut usize {
        match self.focus {
            Pane::Devices => &mut self.dev_cursor,
            Pane::Mounts => &mut self.mnt_cursor,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let max = self.len().saturating_sub(1) as isize;
        if max < 0 {
            return;
        }
        let c = self.cursor_mut();
        *c = (*c as isize + delta).clamp(0, max) as usize;
    }

    fn set_cursor(&mut self, to: usize) {
        let max = self.len().saturating_sub(1);
        *self.cursor_mut() = to.min(max);
    }
}

impl Default for MountView {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Privilege / process helpers
// ---------------------------------------------------------------------------

/// Whether the program runs with effective root privileges.
#[cfg(unix)]
pub fn is_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}

/// Quote a string for safe use inside an `sh -c` command (single-quoted).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Well-known mount points whose removal can render the system unusable or
/// unbootable. Unmounting any of these warrants a loud, extra warning.
pub fn is_essential_mount(mountpoint: &str) -> bool {
    const ESSENTIAL: &[&str] = &[
        "/", "/boot", "/boot/efi", "/efi", "/usr", "/var", "/etc", "/home",
        "/bin", "/sbin", "/lib", "/lib64", "/opt", "/srv", "/run", "/proc",
        "/sys", "/dev",
    ];
    let mp = mountpoint.trim_end_matches('/');
    let mp = if mp.is_empty() { "/" } else { mp };
    ESSENTIAL.contains(&mp)
}

/// Run `sh -c cmd` directly (used when already root).
pub async fn run_shell(cmd: &str) -> Result<(), String> {
    let out = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    status_to_result(out)
}

/// True when `sudo` can run without prompting (passwordless rule or a still-valid
/// cached credential), so a privileged command needs no password from us.
pub async fn sudo_can_noninteractive() -> bool {
    tokio::process::Command::new("sudo")
        .arg("-n")
        .arg("true")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `sudo -n sh -c cmd` (non-interactive — relies on cached/passwordless sudo).
pub async fn run_sudo_noninteractive(cmd: &str) -> Result<(), String> {
    let out = tokio::process::Command::new("sudo")
        .arg("-n")
        .arg("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    status_to_result(out)
}

/// Run `sudo -S sh -c cmd`, feeding `password` on stdin so the TUI never has to
/// be suspended.
pub async fn run_sudo_password(cmd: &str, password: &str) -> Result<(), String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("sudo")
        .arg("-S")
        .arg("-p")
        .arg("")
        .arg("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{password}\n").as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    status_to_result(out)
}

/// Refresh sudo's cached credential by feeding `password` to `sudo -S -v`. Once
/// validated, a following `sudo -n …` runs without prompting — letting a
/// long-running privileged command (e.g. flashing) keep stdin for its own data.
pub async fn sudo_validate(password: &str) -> Result<(), String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("sudo")
        .arg("-S")
        .arg("-p")
        .arg("")
        .arg("-v")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{password}\n").as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    status_to_result(out)
}

fn status_to_result(out: std::process::Output) -> Result<(), String> {
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // Strip the sudo password prompt echo if present.
    let err = err.trim_start_matches("[sudo] password for").trim().to_string();
    Err(if err.is_empty() {
        "command failed".to_string()
    } else {
        err
    })
}

// ---------------------------------------------------------------------------
// Sampling (Linux /proc)
// ---------------------------------------------------------------------------

/// Block devices from `/proc/partitions`, annotated with mount info.
#[cfg(target_os = "linux")]
pub fn list_block_devices(mounts: &[MountEntry]) -> Vec<BlockDevice> {
    let Ok(parts) = std::fs::read_to_string("/proc/partitions") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in parts.lines().skip(2) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let (major, minor) = (f[0], f[1]);
        let blocks: u64 = f[2].parse().unwrap_or(0);
        let name = f[3].to_string();
        // Skip pseudo devices that can't be mounted as ordinary filesystems.
        if name.starts_with("ram") || name.starts_with("loop") || name.starts_with("zram") {
            continue;
        }
        let dev = format!("/dev/{name}");
        let m = mounts.iter().find(|m| m.dev == dev);
        // Filesystem type + label from udev (works for unmounted devices too);
        // fall back to the active mount's fstype.
        let (mut fstype, label) = udev_fs_info(major, minor);
        if fstype.is_empty()
            && let Some(m) = m
        {
            fstype = m.fstype.clone();
        }
        // Vendor/model/serial live on the *parent disk's* sysfs node.
        let disk = parent_disk(&name);
        let parent = (disk != name).then(|| disk.clone());
        let base = format!("/sys/block/{disk}/device");
        out.push(BlockDevice {
            name,
            dev,
            size: blocks * 1024,
            fstype,
            label,
            mountpoint: m.map(|m| m.mountpoint.clone()),
            parent,
            vendor: read_sys(&format!("{base}/vendor")),
            model: read_sys(&format!("{base}/model")),
            serial: read_sys(&format!("{base}/serial")),
            removable: read_sys(&format!("/sys/block/{disk}/removable")) == "1",
        });
    }
    tree_order(out)
}

/// Read the filesystem type and label for device `major:minor` from the udev
/// database (`/run/udev/data/b<maj>:<min>`), which is readable without root.
#[cfg(target_os = "linux")]
fn udev_fs_info(major: &str, minor: &str) -> (String, String) {
    std::fs::read_to_string(format!("/run/udev/data/b{major}:{minor}"))
        .map(|s| parse_udev_fs_info(&s))
        .unwrap_or_default()
}

/// Extract `(fstype, label)` from the body of a udev database record. Lines look
/// like `E:ID_FS_TYPE=ext4` / `E:ID_FS_LABEL=mydisk`.
#[cfg(target_os = "linux")]
fn parse_udev_fs_info(content: &str) -> (String, String) {
    let (mut fstype, mut label) = (String::new(), String::new());
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("E:ID_FS_TYPE=") {
            fstype = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("E:ID_FS_LABEL=") {
            label = v.trim().to_string();
        }
    }
    (fstype, label)
}

/// Reorder devices into a disk→partition tree: each whole disk is immediately
/// followed by its partitions (in their original order).
fn tree_order(devices: Vec<BlockDevice>) -> Vec<BlockDevice> {
    let mut out = Vec::with_capacity(devices.len());
    let mut placed = vec![false; devices.len()];
    for (i, d) in devices.iter().enumerate() {
        if d.parent.is_some() {
            continue; // partitions are emitted under their disk
        }
        out.push(d.clone());
        placed[i] = true;
        for (j, c) in devices.iter().enumerate() {
            if c.parent.as_deref() == Some(d.name.as_str()) {
                out.push(c.clone());
                placed[j] = true;
            }
        }
    }
    // Any partitions whose parent disk wasn't listed (rare) go at the end.
    for (i, d) in devices.into_iter().enumerate() {
        if !placed[i] {
            out.push(d);
        }
    }
    out
}

/// The whole-disk device backing `name` (itself if it is a whole disk).
#[cfg(target_os = "linux")]
fn parent_disk(name: &str) -> String {
    if std::path::Path::new(&format!("/sys/block/{name}")).exists() {
        return name.to_string();
    }
    if let Ok(rd) = std::fs::read_dir("/sys/block") {
        for e in rd.flatten() {
            let disk = e.file_name().to_string_lossy().into_owned();
            // Partitions appear as a subdirectory of their parent disk.
            if std::path::Path::new(&format!("/sys/block/{disk}/{name}")).exists() {
                return disk;
            }
        }
    }
    name.to_string()
}

/// Read a sysfs attribute, trimmed; empty string when absent/unreadable.
#[cfg(target_os = "linux")]
fn read_sys(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
pub fn list_block_devices(_mounts: &[MountEntry]) -> Vec<BlockDevice> {
    Vec::new()
}

/// Mounts of real block devices from `/proc/mounts`.
#[cfg(target_os = "linux")]
pub fn list_mounts() -> Vec<MountEntry> {
    let Ok(text) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        // Only block-device mounts (skip proc/sysfs/tmpfs/cgroup/etc.).
        if !f[0].starts_with("/dev/") {
            continue;
        }
        out.push(MountEntry {
            dev: unescape_mount(f[0]),
            mountpoint: unescape_mount(f[1]),
            fstype: f[2].to_string(),
        });
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn list_mounts() -> Vec<MountEntry> {
    Vec::new()
}

/// Decode `/proc/mounts` octal escapes (`\040` space, `\011` tab, `\012` newline,
/// `\134` backslash).
#[cfg(target_os = "linux")]
fn unescape_mount(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &s[i + 1..i + 4];
            if let Ok(n) = u8::from_str_radix(oct, 8) {
                out.push(n as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn unescape_handles_octal_spaces() {
        assert_eq!(unescape_mount("/mnt/my\\040disk"), "/mnt/my disk");
        assert_eq!(unescape_mount("/plain/path"), "/plain/path");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/mnt/a b"), "'/mnt/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn essential_mounts_are_recognized() {
        for mp in ["/", "/boot", "/boot/efi", "/usr", "/var", "/home", "/etc"] {
            assert!(is_essential_mount(mp), "{mp} should be flagged essential");
        }
        // Trailing slashes don't fool it.
        assert!(is_essential_mount("/boot/"));
        // Ordinary removable mounts are not essential.
        for mp in ["/mnt/usb", "/media/stick", "/home/me/data", "/run/media/x"] {
            assert!(!is_essential_mount(mp), "{mp} should not be flagged");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_udev_extracts_type_and_label() {
        let body = "S:disk/by-uuid/abc\nE:ID_FS_TYPE=ext4\nE:ID_FS_LABEL=mydisk\nE:ID_FS_LABEL_ENC=mydisk\n";
        assert_eq!(parse_udev_fs_info(body), ("ext4".to_string(), "mydisk".to_string()));
        // A record with no filesystem yields blanks.
        assert_eq!(parse_udev_fs_info("E:ID_MODEL=foo\n"), (String::new(), String::new()));
    }

    #[test]
    fn navigation_switches_panes_and_moves() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut v = MountView::new();
        v.devices = vec![
            BlockDevice { name: "sda".into(), dev: "/dev/sda".into(), size: 0, fstype: String::new(), mountpoint: None , ..Default::default() },
            BlockDevice { name: "sdb".into(), dev: "/dev/sdb".into(), size: 0, fstype: String::new(), mountpoint: None , ..Default::default() },
        ];
        v.mounts = vec![MountEntry { dev: "/dev/sda1".into(), mountpoint: "/mnt/x".into(), fstype: "ext4".into() }];
        v.focus = Pane::Devices;
        v.dev_cursor = 0;
        v.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(v.dev_cursor, 1);
        v.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(v.focus, Pane::Mounts);
    }

    #[test]
    fn mouse_clicks_drive_cursor_and_double_click_opens_menus() {
        use ratatui::crossterm::event::KeyModifiers;
        let down = |col, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };

        let mut v = MountView::new();
        v.devices = vec![
            BlockDevice { name: "sda".into(), dev: "/dev/sda".into(), ..Default::default() },
            BlockDevice { name: "sdb".into(), dev: "/dev/sdb".into(), ..Default::default() },
            BlockDevice { name: "sdc".into(), dev: "/dev/sdc".into(), ..Default::default() },
        ];
        v.mounts = vec![
            MountEntry { dev: "/dev/sda1".into(), mountpoint: "/mnt/a".into(), fstype: "ext4".into() },
            MountEntry { dev: "/dev/sdb1".into(), mountpoint: "/mnt/b".into(), fstype: "ext4".into() },
        ];
        // Simulate the geometry a render would record: two side-by-side lists.
        v.dev_hit = ListHit { area: Rect::new(1, 1, 20, 8), top: 0 };
        v.mnt_hit = ListHit { area: Rect::new(25, 1, 20, 8), top: 0 };

        // A single click in the devices list sets focus + cursor, no action.
        let sig = v.handle_mouse(down(3, 3)); // row 3 → index 2 (top=0, area.y=1)
        assert!(matches!(sig, MountSignal::Stay));
        assert_eq!(v.focus, Pane::Devices);
        assert_eq!(v.dev_cursor, 2);

        // A second click on the same row counts as a double-click → DeviceMenu.
        match v.handle_mouse(down(3, 3)) {
            MountSignal::DeviceMenu(d) => assert_eq!(d.name, "sdc"),
            _ => panic!("expected DeviceMenu on double click"),
        }

        // Clicking the mounts list switches focus and selects that mount; a
        // double-click there opens the mount menu.
        let sig = v.handle_mouse(down(27, 1)); // row 1 → index 0
        assert!(matches!(sig, MountSignal::Stay));
        assert_eq!(v.focus, Pane::Mounts);
        assert_eq!(v.mnt_cursor, 0);
        match v.handle_mouse(down(27, 1)) {
            MountSignal::MountMenu(mp) => assert_eq!(mp, "/mnt/a"),
            _ => panic!("expected MountMenu on double click"),
        }

        // A click in neither list is ignored (cursor unchanged).
        let before = v.mnt_cursor;
        assert!(matches!(v.handle_mouse(down(100, 100)), MountSignal::Stay));
        assert_eq!(v.mnt_cursor, before);

        // The wheel scrolls the list under the pointer.
        v.focus = Pane::Mounts;
        v.mnt_cursor = 0;
        v.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 27,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(v.mnt_cursor, 1, "wheel over the mounts list advances it");
    }

    #[test]
    fn two_clicks_on_different_rows_are_not_a_double_click() {
        use ratatui::crossterm::event::KeyModifiers;
        let down = |col, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let mut v = MountView::new();
        v.devices = vec![
            BlockDevice { name: "sda".into(), dev: "/dev/sda".into(), ..Default::default() },
            BlockDevice { name: "sdb".into(), dev: "/dev/sdb".into(), ..Default::default() },
        ];
        v.dev_hit = ListHit { area: Rect::new(0, 0, 20, 8), top: 0 };
        assert!(matches!(v.handle_mouse(down(2, 0)), MountSignal::Stay));
        // Different row → still just a selection, not an activation.
        assert!(matches!(v.handle_mouse(down(2, 1)), MountSignal::Stay));
        assert_eq!(v.dev_cursor, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_shell_reports_success_and_failure() {
        assert!(run_shell("true").await.is_ok());
        let err = run_shell("echo boom >&2; false").await;
        assert!(err.is_err(), "non-zero exit is an error");
        assert!(err.unwrap_err().contains("boom"), "stderr surfaces in the message");
    }

    #[test]
    fn enter_on_device_requests_mount() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut v = MountView::new();
        v.devices = vec![BlockDevice {
            name: "sdb1".into(),
            dev: "/dev/sdb1".into(),
            size: 0,
            fstype: String::new(),
            mountpoint: None,
            ..Default::default()
        }];
        v.focus = Pane::Devices;
        v.dev_cursor = 0;
        match v.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            MountSignal::DeviceMenu(d) => assert_eq!(d.dev, "/dev/sdb1"),
            _ => panic!("Enter on a device should open its action menu"),
        }
    }

    #[test]
    fn format_command_builds_per_filesystem() {
        let spec = |fs, label: &str, quick, inode: &str| FormatSpec {
            dev: "/dev/sdb1".into(),
            fs,
            label: label.into(),
            quick,
            inode_bytes: inode.into(),
        };
        assert_eq!(
            format_command(&spec(FsType::Fat32, "DATA", false, "")),
            "mkfs.vfat -F 32 -n 'DATA' '/dev/sdb1'"
        );
        assert_eq!(
            format_command(&spec(FsType::Ext4, "root", false, "16384")),
            "mkfs.ext4 -F -L 'root' -i '16384' '/dev/sdb1'"
        );
        assert_eq!(
            format_command(&spec(FsType::Ntfs, "", true, "")),
            "mkfs.ntfs -Q -F '/dev/sdb1'"
        );
        assert_eq!(
            format_command(&spec(FsType::Btrfs, "vol", false, "")),
            "mkfs.btrfs -f -L 'vol' '/dev/sdb1'"
        );
    }

    #[test]
    fn tree_order_nests_partitions_under_disks() {
        let dev = |name: &str, parent: Option<&str>| BlockDevice {
            name: name.into(),
            dev: format!("/dev/{name}"),
            parent: parent.map(str::to_string),
            ..Default::default()
        };
        // Deliberately out of order: a partition before its disk.
        let ordered = tree_order(vec![
            dev("sda1", Some("sda")),
            dev("sdb", None),
            dev("sda", None),
            dev("sdb1", Some("sdb")),
        ]);
        let names: Vec<&str> = ordered.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["sdb", "sdb1", "sda", "sda1"]);
    }
}
