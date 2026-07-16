//! File-checksum algorithms.
//!
//! A small dispatch layer over the individual hash crates so the UI can offer a
//! choice of algorithm and the app can stream a file through the chosen one,
//! producing a lowercase-hex digest. The pure computation lives here (no I/O or
//! VFS), so it can be unit-tested with known vectors; the app-side streaming
//! task is in `crate::app::state::checksum`.

use digest::Digest;

/// A supported checksum / hash algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumKind {
    Crc32,
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl ChecksumKind {
    /// Every algorithm, in the order shown in the picker (fast/legacy → strong).
    pub const ALL: [ChecksumKind; 5] = [
        ChecksumKind::Crc32,
        ChecksumKind::Md5,
        ChecksumKind::Sha1,
        ChecksumKind::Sha256,
        ChecksumKind::Sha512,
    ];

    /// The human label shown in the dialog (and matched by [`from_label`]).
    ///
    /// [`from_label`]: ChecksumKind::from_label
    pub fn label(self) -> &'static str {
        match self {
            ChecksumKind::Crc32 => "CRC32",
            ChecksumKind::Md5 => "MD5",
            ChecksumKind::Sha1 => "SHA-1",
            ChecksumKind::Sha256 => "SHA-256",
            ChecksumKind::Sha512 => "SHA-512",
        }
    }

    /// The labels of every algorithm, for a `Choice` field's options.
    pub fn labels() -> Vec<String> {
        Self::ALL.iter().map(|k| k.label().to_string()).collect()
    }

    /// Parse a label back into a kind. Case-insensitive and tolerant of the
    /// separator-less spellings (`SHA1`/`SHA256`/…) as well as the display ones.
    pub fn from_label(s: &str) -> Option<ChecksumKind> {
        let norm = s.trim().to_ascii_uppercase().replace('-', "");
        match norm.as_str() {
            "CRC32" => Some(ChecksumKind::Crc32),
            "MD5" => Some(ChecksumKind::Md5),
            "SHA1" => Some(ChecksumKind::Sha1),
            "SHA256" => Some(ChecksumKind::Sha256),
            "SHA512" => Some(ChecksumKind::Sha512),
            _ => None,
        }
    }

    /// Start a fresh streaming hasher for this algorithm.
    pub fn hasher(self) -> Hasher {
        match self {
            ChecksumKind::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
            ChecksumKind::Md5 => Hasher::Md5(md5::Context::new()),
            ChecksumKind::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            ChecksumKind::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            ChecksumKind::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }
}

/// A streaming hasher: fed the file in chunks via [`update`], then consumed by
/// [`finalize`] to a lowercase-hex digest.
///
/// [`update`]: Hasher::update
/// [`finalize`]: Hasher::finalize
pub enum Hasher {
    Crc32(crc32fast::Hasher),
    Md5(md5::Context),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    /// Feed the next chunk of data into the hash.
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Crc32(h) => h.update(data),
            Hasher::Md5(c) => c.consume(data),
            Hasher::Sha1(h) => Digest::update(h, data),
            Hasher::Sha256(h) => Digest::update(h, data),
            Hasher::Sha512(h) => Digest::update(h, data),
        }
    }

    /// Consume the hasher and return the digest as lowercase hex (no separators).
    pub fn finalize(self) -> String {
        match self {
            Hasher::Crc32(h) => format!("{:08x}", h.finalize()),
            Hasher::Md5(c) => to_hex(c.finalize().0),
            Hasher::Sha1(h) => to_hex(h.finalize()),
            Hasher::Sha256(h) => to_hex(h.finalize()),
            Hasher::Sha512(h) => to_hex(h.finalize()),
        }
    }
}

/// Hash all of `data` in one shot, returning lowercase hex. Convenience for
/// tests and small inputs; the app streams a file via [`ChecksumKind::hasher`].
#[cfg(test)]
pub fn hash_bytes(kind: ChecksumKind, data: &[u8]) -> String {
    let mut h = kind.hasher();
    h.update(data);
    h.finalize()
}

/// Lowercase-hex encode a byte slice.
fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Normalize a user-pasted comparison checksum: strip all whitespace and
/// lowercase it, so digests copied with stray spaces/newlines or in upper case
/// still compare equal. Returns `None` when nothing meaningful remains.
pub fn normalize_expected(s: &str) -> Option<String> {
    let norm: String = s.split_whitespace().collect();
    (!norm.is_empty()).then(|| norm.to_ascii_lowercase())
}

/// The finished result of a checksum task, handed to the result dialog.
#[derive(Debug, Clone)]
pub struct ChecksumReport {
    pub kind: ChecksumKind,
    /// The file name (for the dialog title/body).
    pub name: String,
    /// The computed digest, lowercase hex.
    pub digest: String,
    /// The comparison checksum the user supplied, already normalized, if any.
    pub expected: Option<String>,
}

impl ChecksumReport {
    /// The pass/fail verdict: `Some(true)` on match, `Some(false)` on mismatch,
    /// `None` when no comparison checksum was supplied.
    pub fn verdict(&self) -> Option<bool> {
        self.expected.as_ref().map(|e| e == &self.digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Well-known digests of the ASCII string "abc".
    const ABC: &[u8] = b"abc";

    #[test]
    fn known_vectors_for_abc() {
        assert_eq!(hash_bytes(ChecksumKind::Crc32, ABC), "352441c2");
        assert_eq!(hash_bytes(ChecksumKind::Md5, ABC), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_bytes(ChecksumKind::Sha1, ABC), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hash_bytes(ChecksumKind::Sha256, ABC),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_bytes(ChecksumKind::Sha512, ABC),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn empty_input_vectors() {
        assert_eq!(hash_bytes(ChecksumKind::Crc32, b""), "00000000");
        assert_eq!(hash_bytes(ChecksumKind::Md5, b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hash_bytes(ChecksumKind::Sha1, b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn streaming_matches_one_shot() {
        // Feeding the input in several chunks yields the same digest.
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        for kind in ChecksumKind::ALL {
            let one_shot = hash_bytes(kind, &data);
            let mut h = kind.hasher();
            for chunk in data.chunks(97) {
                h.update(chunk);
            }
            assert_eq!(h.finalize(), one_shot, "{} streamed mismatch", kind.label());
        }
    }

    #[test]
    fn labels_round_trip() {
        for kind in ChecksumKind::ALL {
            assert_eq!(ChecksumKind::from_label(kind.label()), Some(kind));
        }
        // Tolerant parsing of alternate spellings / case / whitespace.
        assert_eq!(ChecksumKind::from_label("sha256"), Some(ChecksumKind::Sha256));
        assert_eq!(ChecksumKind::from_label("  Sha-512 "), Some(ChecksumKind::Sha512));
        assert_eq!(ChecksumKind::from_label("crc32"), Some(ChecksumKind::Crc32));
        assert_eq!(ChecksumKind::from_label("whirlpool"), None);
    }

    #[test]
    fn verdict_is_case_and_whitespace_insensitive() {
        let digest = hash_bytes(ChecksumKind::Sha256, ABC);
        let report = |expected: &str| ChecksumReport {
            kind: ChecksumKind::Sha256,
            name: "f".into(),
            digest: digest.clone(),
            expected: normalize_expected(expected),
        };
        // Exact, upper-cased, and space/newline-littered copies all match.
        assert_eq!(report(&digest).verdict(), Some(true));
        assert_eq!(report(&digest.to_uppercase()).verdict(), Some(true));
        let spaced = format!("  {}\n {} ", &digest[..32], &digest[32..]);
        assert_eq!(report(&spaced).verdict(), Some(true));
        // A different value is a mismatch.
        assert_eq!(report("deadbeef").verdict(), Some(false));
        // No comparison supplied ⇒ no verdict.
        assert_eq!(report("   ").verdict(), None);
    }
}
