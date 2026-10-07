//! Where compared content comes from: the local file system or an in-memory
//! tree (the built-in demo).

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Names that are skipped when walking folders.
pub const IGNORED_NAMES: &[&str] = &[".git", ".DS_Store", "node_modules", ".hg", ".svn"];

pub fn is_ignored(name: &str) -> bool {
    IGNORED_NAMES.contains(&name)
}

/// An in-memory file or directory (used by the built-in demo).
#[derive(Debug)]
pub enum MemNode {
    File { name: String, data: Arc<Vec<u8>> },
    Dir { name: String, children: Vec<Arc<MemNode>> },
}

impl MemNode {
    pub fn name(&self) -> &str {
        match self {
            Self::File { name, .. } | Self::Dir { name, .. } => name,
        }
    }
}

/// A file or folder that can be compared.
#[derive(Clone, Debug)]
pub enum Entry {
    Fs(PathBuf),
    Mem(Arc<MemNode>),
}

impl Entry {
    pub fn name(&self) -> String {
        match self {
            Self::Fs(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            Self::Mem(n) => n.name().to_owned(),
        }
    }

    /// Full path for display (file system) or just the name (in-memory).
    pub fn display_path(&self) -> String {
        match self {
            Self::Fs(p) => p.display().to_string(),
            Self::Mem(n) => n.name().to_owned(),
        }
    }

    pub fn fs_path(&self) -> Option<&Path> {
        match self {
            Self::Fs(p) => Some(p),
            Self::Mem(_) => None,
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            Self::Fs(p) => p.is_dir(),
            Self::Mem(n) => matches!(**n, MemNode::Dir { .. }),
        }
    }

    pub fn len(&self) -> Option<u64> {
        match self {
            Self::Fs(p) => std::fs::metadata(p).ok().map(|m| m.len()),
            Self::Mem(n) => match &**n {
                MemNode::File { data, .. } => Some(data.len() as u64),
                MemNode::Dir { .. } => None,
            },
        }
    }

    /// Children of a directory, without ignored names, sorted by name.
    pub fn children(&self) -> Result<Vec<Entry>, String> {
        let mut out: Vec<Entry> = match self {
            Self::Fs(p) => std::fs::read_dir(p)
                .map_err(|e| format!("{}: {e}", p.display()))?
                .filter_map(|e| e.ok())
                .map(|e| Entry::Fs(e.path()))
                .collect(),
            Self::Mem(n) => match &**n {
                MemNode::Dir { children, .. } => {
                    children.iter().map(|c| Entry::Mem(c.clone())).collect()
                }
                MemNode::File { .. } => Vec::new(),
            },
        };
        out.retain(|e| !is_ignored(&e.name()));
        out.sort_by_key(|e| e.name().to_lowercase());
        Ok(out)
    }

    pub fn read(&self) -> Result<Arc<Vec<u8>>, String> {
        match self {
            Self::Fs(p) => std::fs::read(p)
                .map(Arc::new)
                .map_err(|e| format!("{}: {e}", p.display())),
            Self::Mem(n) => match &**n {
                MemNode::File { data, .. } => Ok(data.clone()),
                MemNode::Dir { name, .. } => Err(format!("{name} is a folder")),
            },
        }
    }
}
