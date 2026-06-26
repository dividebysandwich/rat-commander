//! Backend registry: resolves a [`VfsPath`] scheme to a concrete `Arc<dyn Vfs>`.
//!
//! In Phase 1 only the `file` backend is registered. Phase 4 (archives) and
//! Phase 5 (remote) register additional schemes here, so panel/ops code can
//! resolve any path without knowing the backend list.

use super::local::LocalFs;
use super::{Vfs, VfsPath};
use crate::util::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;

/// Maps a scheme string to a backend handle.
pub struct Registry {
    backends: HashMap<String, Arc<dyn Vfs>>,
}

impl Registry {
    /// Create a registry with the always-present local backend.
    pub fn new() -> Self {
        let mut backends: HashMap<String, Arc<dyn Vfs>> = HashMap::new();
        backends.insert("file".to_string(), Arc::new(LocalFs::new()));
        Registry { backends }
    }

    /// Register (or replace) a backend for a scheme.
    pub fn register(&mut self, scheme: impl Into<String>, backend: Arc<dyn Vfs>) {
        self.backends.insert(scheme.into(), backend);
    }

    /// Resolve the backend that owns this path.
    pub fn resolve(&self, path: &VfsPath) -> Result<Arc<dyn Vfs>> {
        self.backends
            .get(&path.scheme)
            .cloned()
            .ok_or_else(|| Error::other(format!("no backend for scheme '{}'", path.scheme)))
    }

    /// The local backend handle (always present).
    pub fn local(&self) -> Arc<dyn Vfs> {
        self.backends["file"].clone()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
