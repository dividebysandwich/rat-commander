//! Cross-cutting utilities: error type, formatting, async plumbing.

pub mod async_bridge;
pub mod bytes;
pub mod checksum;
pub mod error;
pub mod sysinfo;
pub mod text;

pub use error::{Error, Result};
