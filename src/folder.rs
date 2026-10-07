//! Recursive folder comparison.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::source::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Same,
    Changed,
    /// Only in A (left) – "removed".
    LeftOnly,
    /// Only in B (right) – "added".
    RightOnly,
    Error,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    /// Path relative to the compared roots, `/` separated.
    pub rel: String,
    pub is_dir: bool,
    pub left: Option<Entry>,
    pub right: Option<Entry>,
    pub status: Status,
    pub children: Vec<Node>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub same: usize,
    pub changed: usize,
    pub left_only: usize,
    pub right_only: usize,
}

impl Node {
    pub fn walk_files<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        if self.is_dir {
            for c in &self.children {
                c.walk_files(f);
            }
        } else {
            f(self);
        }
    }

    pub fn find(&self, rel: &str) -> Option<&Node> {
        if self.rel == rel {
            return Some(self);
        }
        self.children
            .iter()
            .filter(|c| rel.strip_prefix(c.rel.as_str()).is_some_and(|r| r.is_empty() || r.starts_with('/')))
            .find_map(|c| c.find(rel))
    }
}

/// Progress shared with the UI while a comparison runs on a background thread.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicUsize,
}

pub fn compare_roots(left: &Entry, right: &Entry, progress: &Arc<Progress>) -> Node {
    let mut root = compare_dir(
        String::new(),
        String::new(),
        Some(left.clone()),
        Some(right.clone()),
        progress,
    );
    root.name = format!("{} ↔ {}", left.name(), right.name());
    root
}

fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}

fn compare_dir(
    name: String,
    rel: String,
    left: Option<Entry>,
    right: Option<Entry>,
    progress: &Arc<Progress>,
) -> Node {
    let mut error = None;
    let mut list = |e: &Option<Entry>| -> Vec<Entry> {
        match e.as_ref().map(Entry::children) {
            Some(Ok(v)) => v,
            Some(Err(err)) => {
                error = Some(err);
                Vec::new()
            }
            None => Vec::new(),
        }
    };
    let lc = list(&left);
    let rc = list(&right);

    // Merge the two sorted child lists by name (and kind).
    let mut names: Vec<(String, bool)> = lc
        .iter()
        .chain(rc.iter())
        .map(|e| (e.name(), e.is_dir()))
        .collect();
    names.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            .then_with(|| a.0.cmp(&b.0))
    });
    names.dedup();

    let mut children = Vec::with_capacity(names.len());
    for (child_name, is_dir) in names {
        let pick = |v: &[Entry]| {
            v.iter()
                .find(|e| e.name() == child_name && e.is_dir() == is_dir)
                .cloned()
        };
        let (l, r) = (pick(&lc), pick(&rc));
        let child_rel = join(&rel, &child_name);
        let node = if is_dir {
            compare_dir(child_name, child_rel, l, r, progress)
        } else {
            compare_file(child_name, child_rel, l, r, progress)
        };
        children.push(node);
    }

    let status = match (&left, &right) {
        (Some(_), None) => Status::LeftOnly,
        (None, Some(_)) => Status::RightOnly,
        _ if error.is_some() => Status::Error,
        _ if children.iter().all(|c| c.status == Status::Same) => Status::Same,
        _ => Status::Changed,
    };
    Node {
        name,
        rel,
        is_dir: true,
        left,
        right,
        status,
        children,
        error,
    }
}

fn compare_file(
    name: String,
    rel: String,
    left: Option<Entry>,
    right: Option<Entry>,
    progress: &Arc<Progress>,
) -> Node {
    progress.files.fetch_add(1, Ordering::Relaxed);
    let mut error = None;
    let status = match (&left, &right) {
        (Some(_), None) => Status::LeftOnly,
        (None, Some(_)) => Status::RightOnly,
        (Some(l), Some(r)) => {
            if l.len() != r.len() {
                Status::Changed
            } else {
                match (l.read(), r.read()) {
                    (Ok(a), Ok(b)) if a == b => Status::Same,
                    (Ok(_), Ok(_)) => Status::Changed,
                    (Err(e), _) | (_, Err(e)) => {
                        error = Some(e);
                        Status::Error
                    }
                }
            }
        }
        (None, None) => Status::Error,
    };
    Node {
        name,
        rel,
        is_dir: false,
        left,
        right,
        status,
        children: Vec::new(),
        error,
    }
}
