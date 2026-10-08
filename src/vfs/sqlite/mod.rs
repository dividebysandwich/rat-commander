//! `sqlite://` VFS backend — browse a database as a directory.
//!
//! ```text
//! /                     one directory per table and view, plus `_schema.sql`
//! /<table>              its rows — or, past PAGE_ROWS of them, page directories
//! /<table>/<page>       a slice of the rows
//! /<table>/<row>        one row, as `column = value` lines
//! ```
//!
//! **This is the one provider in the family that does not build a tree up
//! front.** Archives, ISO images and git revisions all hand over a complete
//! member list cheaply, so [`VfsTree`](crate::vfs::tree::VfsTree) grafts it once
//! and every later listing is a lookup. A table with five million rows has no
//! such list: materializing one would mean five million `Child` records before
//! the first frame could be drawn. So each `read_dir` issues a query scoped to
//! exactly what is being looked at, and deep tables are paged.
//!
//! Opened **read-only and `immutable`**, so a database another process is
//! writing is neither blocked nor altered by being looked at.

use crate::util::{Error, Result};
use crate::vfs::membuf::MemReader;
use crate::vfs::tree;
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Rows per page directory, once a table has more than this many.
const PAGE_ROWS: usize = 1_000;
/// A blob is described rather than dumped; this much of it is shown as hex.
const BLOB_PREVIEW: usize = 64;
/// The schema dump's name at the root.
const SCHEMA: &str = "_schema.sql";

/// Open `path` read-only, without touching it or waiting on another writer.
///
/// `immutable=1` is what makes this safe to point at a database that something
/// else has open: SQLite then reads it without locking and without replaying a
/// journal, which is right for looking but would be wrong for using.
fn open(path: &Path) -> Result<Connection> {
    let uri = format!("file:{}?immutable=1", encode_uri(&path.to_string_lossy()));
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| Error::other(format!("sqlite: {e}")))
}

/// Percent-encode what a SQLite URI filename cannot carry literally.
fn encode_uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'?' | b'#' | b'%' => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    out
}

/// Whether `path` is a SQLite database — the probe that decides whether this
/// backend claims a file, read from its 16-byte header magic.
pub fn looks_like_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut magic = [0u8; 16];
    f.read_exact(&mut magic).is_ok() && &magic == b"SQLite format 3\0"
}

/// A table or view, and how its rows are named.
struct Table {
    name: String,
    /// The expression that names a row. `rowid` for an ordinary table; the
    /// primary key's columns joined for a `WITHOUT ROWID` one; the ordinal
    /// position when neither can identify a row.
    key: Key,
    rows: usize,
}

enum Key {
    /// An ordinary table: `rowid` is unique, stable and already an integer.
    RowId,
    /// `WITHOUT ROWID`, or a view: name rows by these columns, joined by `-`.
    Columns(Vec<String>),
    /// Nothing identifies a row, so they are numbered by position.
    Ordinal,
}

/// Quote an identifier for interpolation — SQLite has no placeholder for one.
fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// The tables and views in a database, in name order.
fn tables(db: &Connection) -> Result<Vec<Table>> {
    let mut st = db
        .prepare(
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(sql)?;
    let listed: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(sql)?
        .collect::<std::result::Result<_, _>>()
        .map_err(sql)?;

    let mut out = Vec::with_capacity(listed.len());
    for (name, kind) in listed {
        let key = row_key(db, &name, &kind);
        // A view's body can fail at read time; a table that cannot be counted is
        // shown empty rather than hiding the whole database.
        let rows = db
            .query_row(&format!("SELECT COUNT(*) FROM {}", quote(&name)), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as usize;
        out.push(Table { name, key, rows });
    }
    Ok(out)
}

/// How rows of `name` should be named.
fn row_key(db: &Connection, name: &str, kind: &str) -> Key {
    if kind == "view" {
        return Key::Ordinal;
    }
    // `rowid` is there unless the table was declared WITHOUT ROWID; asking for
    // it is the cheapest way to find out.
    if db.query_row(&format!("SELECT rowid FROM {} LIMIT 1", quote(name)), [], |_| Ok(())).is_ok() {
        return Key::RowId;
    }
    let Ok(mut st) = db.prepare(&format!("PRAGMA table_info({})", quote(name))) else {
        return Key::Ordinal;
    };
    // `table_info` gives (cid, name, type, notnull, dflt, pk); `pk` is the
    // 1-based position in the primary key, or 0.
    let pk: Vec<(i64, String)> = st
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
        .map(|rows| rows.flatten().filter(|(p, _)| *p > 0).collect())
        .unwrap_or_default();
    if pk.is_empty() {
        return Key::Ordinal;
    }
    let mut pk = pk;
    pk.sort_by_key(|(p, _)| *p);
    Key::Columns(pk.into_iter().map(|(_, n)| n).collect())
}

fn sql(e: rusqlite::Error) -> Error {
    Error::other(format!("sqlite: {e}"))
}

/// Zero-pad `n` to the width `total` needs, so sorting by name sorts by number.
fn padded(n: usize, total: usize) -> String {
    let width = total.saturating_sub(1).to_string().len().max(1);
    format!("{n:0width$}")
}

/// Make a row name safe as one path component.
fn sanitize(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '-' } else { c })
        .collect();
    if out.is_empty() {
        out.push('-');
    }
    out.truncate(120);
    out
}

/// Render one value the way a row file shows it.
fn render(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        // A blob is described rather than dumped: the alternative is binary
        // pasted into a text view, which helps nobody.
        ValueRef::Blob(b) => {
            let head: String = b.iter().take(BLOB_PREVIEW).map(|x| format!("{x:02x}")).collect();
            let more = if b.len() > BLOB_PREVIEW { "…" } else { "" };
            format!("<blob, {} bytes> {head}{more}", b.len())
        }
    }
}

/// A row, as the file that stands for it.
fn row_text(db: &Connection, table: &Table, index: usize) -> Result<String> {
    let sel = match &table.key {
        Key::RowId => format!("SELECT * FROM {} WHERE rowid = ?1", quote(&table.name)),
        _ => format!("SELECT * FROM {} LIMIT 1 OFFSET ?1", quote(&table.name)),
    };
    let mut st = db.prepare(&sel).map_err(sql)?;
    let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
    let mut rows = st.query([index as i64]).map_err(sql)?;
    let Some(r) = rows.next().map_err(sql)? else {
        return Err(Error::NotFound(format!("row {index}")));
    };
    let mut out = String::new();
    for (i, name) in names.iter().enumerate() {
        let v = r.get_ref(i).map_err(sql)?;
        out.push_str(&format!("{name} = {}\n", render(v)));
    }
    Ok(out)
}

/// The names of a table's rows, for `index..index + limit`.
fn row_names(db: &Connection, table: &Table, from: usize, limit: usize) -> Result<Vec<String>> {
    let total = table.rows;
    match &table.key {
        Key::RowId => {
            let q = format!(
                "SELECT rowid FROM {} ORDER BY rowid LIMIT ?1 OFFSET ?2",
                quote(&table.name)
            );
            let mut st = db.prepare(&q).map_err(sql)?;
            let ids: Vec<i64> = st
                .query_map([limit as i64, from as i64], |r| r.get(0))
                .map_err(sql)?
                .collect::<std::result::Result<_, _>>()
                .map_err(sql)?;
            Ok(ids.into_iter().map(|id| id.to_string()).collect())
        }
        Key::Columns(cols) => {
            let list: Vec<String> = cols.iter().map(|c| quote(c)).collect();
            let q = format!(
                "SELECT {} FROM {} LIMIT ?1 OFFSET ?2",
                list.join(", "),
                quote(&table.name)
            );
            let mut st = db.prepare(&q).map_err(sql)?;
            let mut rows = st.query([limit as i64, from as i64]).map_err(sql)?;
            let mut out = Vec::new();
            let mut n = from;
            while let Some(r) = rows.next().map_err(sql)? {
                let parts: Vec<String> =
                    (0..cols.len()).map(|i| r.get_ref(i).map(render).unwrap_or_default()).collect();
                let name = sanitize(&parts.join("-"));
                // A key that renders to nothing usable still needs a name of its
                // own, or two rows would collide.
                out.push(if name == "-" { padded(n, total) } else { name });
                n += 1;
            }
            Ok(out)
        }
        Key::Ordinal => Ok((from..(from + limit).min(total)).map(|n| padded(n, total)).collect()),
    }
}

fn dir_entry(name: String, mtime: Option<SystemTime>) -> VfsEntry {
    VfsEntry {
        name,
        kind: VfsKind::Dir,
        size: 0,
        mtime,
        atime: None,
        ctime: None,
        btime: None,
        inode: None,
        mode: None,
        uid: None,
        gid: None,
        symlink_target: None,
        symlink_broken: false,
        symlink_dir: false,
    }
}

fn file_entry(name: String, size: u64, mtime: Option<SystemTime>) -> VfsEntry {
    VfsEntry { kind: VfsKind::File, size, ..dir_entry(name, mtime) }
}

/// Where an inner path points.
enum Loc {
    Root,
    Table(Table),
    /// A page of a large table.
    Page(Table, usize),
    /// One row: the table and its index within it.
    Row(Table, usize),
    Schema,
}

fn container_of(path: &VfsPath) -> Result<&PathBuf> {
    path.container.as_ref().ok_or_else(|| Error::InvalidPath("not a sqlite path".to_string()))
}

/// How many page directories a table needs, or 0 when its rows are listed flat.
fn pages_of(t: &Table) -> usize {
    if t.rows > PAGE_ROWS { t.rows.div_ceil(PAGE_ROWS) } else { 0 }
}

fn page_name(page: usize, t: &Table) -> String {
    let last = (page * PAGE_ROWS + PAGE_ROWS - 1).min(t.rows.saturating_sub(1));
    format!("{}-{}", padded(page * PAGE_ROWS, t.rows), padded(last, t.rows))
}

/// Resolve an inner path against the database.
fn locate(db: &Connection, inner: &str) -> Result<Loc> {
    let norm = tree::normalize(inner);
    if norm == "/" {
        return Ok(Loc::Root);
    }
    let parts: Vec<&str> = norm[1..].split('/').collect();
    if parts[0] == SCHEMA && parts.len() == 1 {
        return Ok(Loc::Schema);
    }
    let table = tables(db)?
        .into_iter()
        .find(|t| t.name == parts[0])
        .ok_or_else(|| Error::NotFound(norm.clone()))?;
    match parts.len() {
        1 => Ok(Loc::Table(table)),
        2 if pages_of(&table) > 0 => {
            let page = (0..pages_of(&table))
                .find(|&p| page_name(p, &table) == parts[1])
                .ok_or_else(|| Error::NotFound(norm.clone()))?;
            Ok(Loc::Page(table, page))
        }
        2 => index_of(db, &table, 0, parts[1]).map(|i| Loc::Row(table, i)),
        3 => {
            let page = (0..pages_of(&table))
                .find(|&p| page_name(p, &table) == parts[1])
                .ok_or_else(|| Error::NotFound(norm.clone()))?;
            index_of(db, &table, page * PAGE_ROWS, parts[2]).map(|i| Loc::Row(table, i))
        }
        _ => Err(Error::NotFound(norm)),
    }
}

/// Find the index of the row named `name`, searching from `from`.
fn index_of(db: &Connection, table: &Table, from: usize, name: &str) -> Result<usize> {
    let limit = if pages_of(table) > 0 { PAGE_ROWS } else { table.rows };
    let names = row_names(db, table, from, limit)?;
    names
        .iter()
        .position(|n| n == name)
        .map(|i| match table.key {
            // A rowid names itself, so the index *is* the rowid.
            Key::RowId => name.parse::<usize>().unwrap_or(from + i),
            _ => from + i,
        })
        .ok_or_else(|| Error::NotFound(name.to_string()))
}

/// A database, browsed as a directory.
pub struct SqliteFs;

impl SqliteFs {
    pub fn new() -> Self {
        SqliteFs
    }

    /// Run `f` against the database on a blocking thread.
    async fn with_db<T, F>(&self, container: &Path, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let path = container.to_path_buf();
        tokio::task::spawn_blocking(move || f(&open(&path)?))
            .await
            .map_err(|e| Error::other(e.to_string()))?
    }
}

impl Default for SqliteFs {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Vfs for SqliteFs {
    fn scheme(&self) -> &str {
        "sqlite"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::read_only()
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let inner = dir.posix_path();
        let mtime = tree::stamp_mtime(container_of(dir)?).await;
        self.with_db(container_of(dir)?, move |db| match locate(db, &inner)? {
            Loc::Root => {
                let mut out = vec![file_entry(SCHEMA.to_string(), 0, mtime)];
                out.extend(tables(db)?.into_iter().map(|t| dir_entry(t.name, mtime)));
                Ok(out)
            }
            // Past a thousand rows the listing is paged: a directory of five
            // million entries is not a listing anyone can use, and building one
            // would cost more than the whole rest of this backend.
            Loc::Table(t) if pages_of(&t) > 0 => {
                Ok((0..pages_of(&t)).map(|p| dir_entry(page_name(p, &t), mtime)).collect())
            }
            Loc::Table(t) => {
                let names = row_names(db, &t, 0, t.rows)?;
                Ok(names.into_iter().map(|n| file_entry(n, 0, mtime)).collect())
            }
            Loc::Page(t, page) => {
                let names = row_names(db, &t, page * PAGE_ROWS, PAGE_ROWS)?;
                Ok(names.into_iter().map(|n| file_entry(n, 0, mtime)).collect())
            }
            Loc::Row(..) | Loc::Schema => Err(Error::other("not a directory")),
        })
        .await
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let inner = path.posix_path();
        let name = tree::base_name(&tree::normalize(&inner));
        let mtime = tree::stamp_mtime(container_of(path)?).await;
        self.with_db(container_of(path)?, move |db| match locate(db, &inner)? {
            Loc::Root => Ok(dir_entry(String::new(), mtime)),
            Loc::Table(_) | Loc::Page(..) => Ok(dir_entry(name, mtime)),
            Loc::Schema => Ok(file_entry(name, schema_sql(db)?.len() as u64, mtime)),
            Loc::Row(t, i) => Ok(file_entry(name, row_text(db, &t, i)?.len() as u64, mtime)),
        })
        .await
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let inner = path.posix_path();
        let text = self
            .with_db(container_of(path)?, move |db| match locate(db, &inner)? {
                Loc::Schema => schema_sql(db),
                Loc::Row(t, i) => row_text(db, &t, i),
                _ => Err(Error::other("is a directory")),
            })
            .await?;
        Ok(Box::new(MemReader::new(text.into_bytes())))
    }

    async fn open_write(&self, _path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        Err(Error::Unsupported)
    }
    async fn mkdir(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn remove_file(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn remove_dir(&self, _path: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
    async fn rename(&self, _from: &VfsPath, _to: &VfsPath) -> Result<()> {
        Err(Error::Unsupported)
    }
}

/// The database's own schema, as the statements that would rebuild it.
fn schema_sql(db: &Connection) -> Result<String> {
    let mut st = db
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
        .map_err(sql)?;
    let rows: Vec<String> = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<std::result::Result<_, _>>()
        .map_err(sql)?;
    Ok(rows.iter().map(|s| format!("{s};\n")).collect())
}

#[cfg(test)]
mod tests;
