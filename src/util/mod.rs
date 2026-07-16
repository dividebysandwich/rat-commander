//! Cross-cutting utilities: error type, formatting, async plumbing.

pub mod async_bridge;
pub mod bytes;
pub mod checksum;
pub mod error;
pub mod img;
pub mod qr;
pub mod scroll;
pub mod sysinfo;
pub mod temp;
pub mod text;

pub use error::{Error, Result};
