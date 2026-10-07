//! Built-in demo content (the `samples/` folder), so the app can be tried
//! without picking any files.

use std::sync::Arc;

use crate::source::{Entry, MemNode};

macro_rules! files {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../samples/", $path)) as &[u8])),*]
    };
}

const FILES: &[(&str, &[u8])] = files![
    "left/LICENSE",
    "left/docs/README.md",
    "left/package.json",
    "left/src/main.rs",
    "left/src/util/format.ts",
    "left/src/util/legacy.py",
    "right/LICENSE",
    "right/assets/logo.png",
    "right/docs/README.md",
    "right/package.json",
    "right/src/main.rs",
    "right/src/util/format.ts",
    "right/src/util/stats.py",
    "merge/base.rs",
    "merge/local.rs",
    "merge/remote.rs",
];

fn insert(children: &mut Vec<MemNode>, parts: &[&str], data: &[u8]) {
    let [first, rest @ ..] = parts else { return };
    if rest.is_empty() {
        children.push(MemNode::File {
            name: (*first).to_owned(),
            data: Arc::new(data.to_vec()),
        });
        return;
    }
    let pos = children
        .iter()
        .position(|c| matches!(c, MemNode::Dir { name, .. } if name == first));
    let idx = pos.unwrap_or_else(|| {
        children.push(MemNode::Dir {
            name: (*first).to_owned(),
            children: Vec::new(),
        });
        children.len() - 1
    });
    if let MemNode::Dir { children: sub, .. } = &mut children[idx] {
        // Children are stored as Arc; rebuild while constructing.
        let mut owned: Vec<MemNode> = sub.drain(..).map(|c| Arc::try_unwrap(c).expect("unshared")).collect();
        insert(&mut owned, rest, data);
        *sub = owned.into_iter().map(Arc::new).collect();
    }
}

fn tree(root: &str) -> Entry {
    let mut children = Vec::new();
    for (path, data) in FILES {
        if let Some(rest) = path.strip_prefix(&format!("{root}/")) {
            let parts: Vec<&str> = rest.split('/').collect();
            insert(&mut children, &parts, data);
        }
    }
    Entry::Mem(Arc::new(MemNode::Dir {
        name: root.to_owned(),
        children: children.into_iter().map(Arc::new).collect(),
    }))
}

fn file(path: &str) -> Entry {
    let (_, data) = FILES.iter().find(|(p, _)| *p == path).expect("demo file");
    Entry::Mem(Arc::new(MemNode::File {
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        data: Arc::new(data.to_vec()),
    }))
}

pub fn folders() -> (Entry, Entry) {
    (tree("left"), tree("right"))
}

/// (base, local, remote)
pub fn merge() -> (Entry, Entry, Entry) {
    (file("merge/base.rs"), file("merge/local.rs"), file("merge/remote.rs"))
}
