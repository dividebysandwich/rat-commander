//! Per-format archive adapters: list metadata, read one entry, read/write the
//! whole archive (used for create / add / remove via rebuild).
//!
//! Everything here is synchronous and meant to run on a blocking thread.

use crate::util::{Error, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// Supported archive formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    SevenZ,
    Rar,
}

impl ArchiveFormat {
    /// Detect the format from a file name's extension.
    pub fn from_name(name: &str) -> Option<ArchiveFormat> {
        let n = name.to_ascii_lowercase();
        if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else if n.ends_with(".tar.bz2") || n.ends_with(".tbz2") || n.ends_with(".tbz") {
            Some(ArchiveFormat::TarBz2)
        } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
            Some(ArchiveFormat::TarXz)
        } else if n.ends_with(".tar") {
            Some(ArchiveFormat::Tar)
        } else if n.ends_with(".zip") {
            Some(ArchiveFormat::Zip)
        } else if n.ends_with(".7z") {
            Some(ArchiveFormat::SevenZ)
        } else if n.ends_with(".rar") {
            Some(ArchiveFormat::Rar)
        } else {
            None
        }
    }

    pub fn from_path(path: &Path) -> Option<ArchiveFormat> {
        path.file_name()
            .and_then(|n| n.to_str())
            .and_then(ArchiveFormat::from_name)
    }

    /// Whether files can be added to / removed from this format (via rebuild).
    pub fn writable(self) -> bool {
        !matches!(self, ArchiveFormat::Rar)
    }
}

/// One archive member: normalized inner path, dir flag, uncompressed size.
pub struct RawEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

/// A member with its bytes (used for rebuild).
pub struct FullEntry {
    pub path: String,
    pub is_dir: bool,
    pub data: Vec<u8>,
}

/// Normalize an archive member path to `/a/b` form (no `./`, no trailing `/`).
pub fn normalize(name: &str) -> String {
    let mut s = name.replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    let trimmed = s.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn io<E: std::fmt::Display>(e: E) -> Error {
    Error::other(e.to_string())
}

// ---------------------------------------------------------------------------
// Listing (metadata only)
// ---------------------------------------------------------------------------

pub fn list_entries(format: ArchiveFormat, container: &Path) -> Result<Vec<RawEntry>> {
    match format {
        ArchiveFormat::Zip => list_zip(container),
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 | ArchiveFormat::TarXz => {
            list_tar(format, container)
        }
        ArchiveFormat::SevenZ => list_7z(container),
        ArchiveFormat::Rar => list_rar(container),
    }
}

fn list_zip(container: &Path) -> Result<Vec<RawEntry>> {
    let mut za = zip::ZipArchive::new(File::open(container)?).map_err(io)?;
    let mut out = Vec::with_capacity(za.len());
    for i in 0..za.len() {
        let f = za.by_index(i).map_err(io)?;
        out.push(RawEntry {
            path: normalize(f.name()),
            is_dir: f.is_dir(),
            size: f.size(),
        });
    }
    Ok(out)
}

fn list_tar(format: ArchiveFormat, container: &Path) -> Result<Vec<RawEntry>> {
    let reader = tar_reader(format, File::open(container)?)?;
    let mut ar = tar::Archive::new(reader);
    let mut out = Vec::new();
    for e in ar.entries().map_err(io)? {
        let e = e.map_err(io)?;
        let path = e.path().map_err(io)?.to_string_lossy().into_owned();
        let is_dir = e.header().entry_type().is_dir();
        let size = e.header().size().unwrap_or(0);
        out.push(RawEntry {
            path: normalize(&path),
            is_dir,
            size,
        });
    }
    Ok(out)
}

fn list_7z(container: &Path) -> Result<Vec<RawEntry>> {
    let archive = sevenz_rust2::Archive::open(container).map_err(io)?;
    Ok(archive
        .files
        .iter()
        .map(|f| RawEntry {
            path: normalize(f.name()),
            is_dir: f.is_directory(),
            size: f.size(),
        })
        .collect())
}

fn list_rar(container: &Path) -> Result<Vec<RawEntry>> {
    let archive = unrar::Archive::new(container)
        .open_for_listing()
        .map_err(io)?;
    let mut out = Vec::new();
    for entry in archive {
        let e = entry.map_err(io)?;
        out.push(RawEntry {
            path: normalize(&e.filename.to_string_lossy()),
            is_dir: e.is_directory(),
            size: e.unpacked_size,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reading a single entry
// ---------------------------------------------------------------------------

/// Read one member identified by its normalized inner path (`/a/b`).
pub fn read_entry(format: ArchiveFormat, container: &Path, inner: &str) -> Result<Vec<u8>> {
    let target = normalize(inner);
    match format {
        ArchiveFormat::Zip => {
            let mut za = zip::ZipArchive::new(File::open(container)?).map_err(io)?;
            for i in 0..za.len() {
                let mut f = za.by_index(i).map_err(io)?;
                if normalize(f.name()) == target {
                    let mut data = Vec::new();
                    f.read_to_end(&mut data)?;
                    return Ok(data);
                }
            }
            Err(Error::NotFound(target))
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 | ArchiveFormat::TarXz => {
            let reader = tar_reader(format, File::open(container)?)?;
            let mut ar = tar::Archive::new(reader);
            for e in ar.entries().map_err(io)? {
                let mut e = e.map_err(io)?;
                let path = e.path().map_err(io)?.to_string_lossy().into_owned();
                if normalize(&path) == target {
                    let mut data = Vec::new();
                    e.read_to_end(&mut data)?;
                    return Ok(data);
                }
            }
            Err(Error::NotFound(target))
        }
        ArchiveFormat::SevenZ => {
            let archive = sevenz_rust2::Archive::open(container).map_err(io)?;
            let name = archive
                .files
                .iter()
                .find(|f| normalize(f.name()) == target)
                .map(|f| f.name().to_string())
                .ok_or_else(|| Error::NotFound(target.clone()))?;
            let mut reader =
                sevenz_rust2::ArchiveReader::open(container, sevenz_rust2::Password::empty())
                    .map_err(io)?;
            reader.read_file(&name).map_err(io)
        }
        ArchiveFormat::Rar => {
            let mut ar = unrar::Archive::new(container)
                .open_for_processing()
                .map_err(io)?;
            while let Some(header) = ar.read_header().map_err(io)? {
                let name = normalize(&header.entry().filename.to_string_lossy());
                if name == target {
                    let (data, _next) = header.read().map_err(io)?;
                    return Ok(data);
                }
                ar = header.skip().map_err(io)?;
            }
            Err(Error::NotFound(target))
        }
    }
}

// ---------------------------------------------------------------------------
// Whole-archive read (for rebuild)
// ---------------------------------------------------------------------------

pub fn read_all(format: ArchiveFormat, container: &Path) -> Result<Vec<FullEntry>> {
    match format {
        ArchiveFormat::Zip => {
            let mut za = zip::ZipArchive::new(File::open(container)?).map_err(io)?;
            let mut out = Vec::with_capacity(za.len());
            for i in 0..za.len() {
                let mut f = za.by_index(i).map_err(io)?;
                let is_dir = f.is_dir();
                let path = normalize(f.name());
                let mut data = Vec::new();
                if !is_dir {
                    f.read_to_end(&mut data)?;
                }
                out.push(FullEntry { path, is_dir, data });
            }
            Ok(out)
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 | ArchiveFormat::TarXz => {
            let reader = tar_reader(format, File::open(container)?)?;
            let mut ar = tar::Archive::new(reader);
            let mut out = Vec::new();
            for e in ar.entries().map_err(io)? {
                let mut e = e.map_err(io)?;
                let is_dir = e.header().entry_type().is_dir();
                let path = normalize(&e.path().map_err(io)?.to_string_lossy());
                let mut data = Vec::new();
                if !is_dir {
                    e.read_to_end(&mut data)?;
                }
                out.push(FullEntry { path, is_dir, data });
            }
            Ok(out)
        }
        ArchiveFormat::SevenZ => {
            let archive = sevenz_rust2::Archive::open(container).map_err(io)?;
            let mut reader =
                sevenz_rust2::ArchiveReader::open(container, sevenz_rust2::Password::empty())
                    .map_err(io)?;
            let mut out = Vec::new();
            for f in &archive.files {
                let is_dir = f.is_directory();
                let data = if is_dir {
                    Vec::new()
                } else {
                    reader.read_file(f.name()).map_err(io)?
                };
                out.push(FullEntry {
                    path: normalize(f.name()),
                    is_dir,
                    data,
                });
            }
            Ok(out)
        }
        ArchiveFormat::Rar => {
            let mut ar = unrar::Archive::new(container)
                .open_for_processing()
                .map_err(io)?;
            let mut out = Vec::new();
            while let Some(header) = ar.read_header().map_err(io)? {
                let e = header.entry();
                let path = normalize(&e.filename.to_string_lossy());
                let is_dir = e.is_directory();
                if is_dir {
                    out.push(FullEntry { path, is_dir, data: Vec::new() });
                    ar = header.skip().map_err(io)?;
                } else {
                    let (data, next) = header.read().map_err(io)?;
                    out.push(FullEntry { path, is_dir, data });
                    ar = next;
                }
            }
            Ok(out)
        }
    }
}

// ---------------------------------------------------------------------------
// Whole-archive write (create / rebuild). `dest` is the file to (over)write.
// ---------------------------------------------------------------------------

pub fn write_all(format: ArchiveFormat, dest: &Path, entries: &[FullEntry]) -> Result<()> {
    match format {
        ArchiveFormat::Zip => write_zip(dest, entries),
        ArchiveFormat::Tar => {
            let f = File::create(dest)?;
            build_tar(f, entries)?.flush()?;
            Ok(())
        }
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(File::create(dest)?, flate2::Compression::default());
            build_tar(enc, entries)?.finish().map_err(io)?;
            Ok(())
        }
        ArchiveFormat::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(File::create(dest)?, bzip2::Compression::default());
            build_tar(enc, entries)?.finish().map_err(io)?;
            Ok(())
        }
        ArchiveFormat::TarXz => {
            let enc = xz2::write::XzEncoder::new(File::create(dest)?, 6);
            build_tar(enc, entries)?.finish().map_err(io)?;
            Ok(())
        }
        ArchiveFormat::SevenZ => write_7z(dest, entries),
        ArchiveFormat::Rar => Err(Error::other("creating RAR archives is not supported")),
    }
}

fn write_zip(dest: &Path, entries: &[FullEntry]) -> Result<()> {
    let mut zw = zip::ZipWriter::new(File::create(dest)?);
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
    for e in entries {
        let name = e.path.trim_start_matches('/');
        if name.is_empty() {
            continue;
        }
        if e.is_dir {
            zw.add_directory(name, opts).map_err(io)?;
        } else {
            zw.start_file(name, opts).map_err(io)?;
            zw.write_all(&e.data)?;
        }
    }
    zw.finish().map_err(io)?;
    Ok(())
}

fn build_tar<W: Write>(w: W, entries: &[FullEntry]) -> Result<W> {
    let mut b = tar::Builder::new(w);
    for e in entries {
        let name = e.path.trim_start_matches('/');
        if name.is_empty() {
            continue;
        }
        let mut header = tar::Header::new_gnu();
        if e.is_dir {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            let dir_name = format!("{name}/");
            header.set_cksum();
            b.append_data(&mut header, dir_name, std::io::empty())
                .map_err(io)?;
        } else {
            header.set_size(e.data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            b.append_data(&mut header, name, &e.data[..]).map_err(io)?;
        }
    }
    b.into_inner().map_err(io)
}

fn write_7z(dest: &Path, entries: &[FullEntry]) -> Result<()> {
    let mut w = sevenz_rust2::ArchiveWriter::create(dest).map_err(io)?;
    for e in entries {
        let name = e.path.trim_start_matches('/');
        if name.is_empty() {
            continue;
        }
        if e.is_dir {
            w.push_archive_entry::<&[u8]>(
                sevenz_rust2::ArchiveEntry::new_directory(name),
                None,
            )
            .map_err(io)?;
        } else {
            w.push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file(name),
                Some(&e.data[..]),
            )
            .map_err(io)?;
        }
    }
    w.finish().map_err(io)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// tar decompression reader
// ---------------------------------------------------------------------------

fn tar_reader(format: ArchiveFormat, file: File) -> Result<Box<dyn Read>> {
    Ok(match format {
        ArchiveFormat::Tar => Box::new(file),
        ArchiveFormat::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        ArchiveFormat::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        ArchiveFormat::TarXz => Box::new(xz2::read::XzDecoder::new(file)),
        _ => return Err(Error::other("not a tar format")),
    })
}
