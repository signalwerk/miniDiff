//! Three-way merge (diff3) and conflict-marker parsing.

use std::borrow::Cow;

use similar::DiffOp;

use crate::diff::line_ops;
use crate::text::TextDoc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkKind {
    /// Unchanged on both sides.
    Stable,
    /// Changed only locally (ours) – merged automatically.
    Local,
    /// Changed only remotely (theirs) – merged automatically.
    Remote,
    /// Both sides made the identical change.
    Both,
    /// Both sides changed the same region differently.
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Unresolved,
    Local,
    Remote,
    LocalThenRemote,
    RemoteThenLocal,
    Base,
    Custom(Vec<String>),
}

/// Where a line of the merged result came from (used for colouring).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Stable,
    Local,
    Remote,
    Base,
    Custom,
    Marker,
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub kind: ChunkKind,
    pub base: Vec<String>,
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub resolution: Resolution,
}

impl Chunk {
    fn new(kind: ChunkKind, base: Vec<String>, local: Vec<String>, remote: Vec<String>) -> Self {
        Self {
            resolution: default_resolution(kind),
            kind,
            base,
            local,
            remote,
        }
    }

    pub fn is_change(&self) -> bool {
        self.kind != ChunkKind::Stable
    }

    pub fn result(&self) -> Vec<(&str, Origin)> {
        fn tag(v: &[String], o: Origin) -> Vec<(&str, Origin)> {
            v.iter().map(|s| (s.as_str(), o)).collect()
        }
        if self.kind == ChunkKind::Stable {
            return tag(&self.base, Origin::Stable);
        }
        match &self.resolution {
            Resolution::Local => tag(&self.local, Origin::Local),
            Resolution::Remote => tag(&self.remote, Origin::Remote),
            Resolution::Base => tag(&self.base, Origin::Base),
            Resolution::LocalThenRemote => {
                let mut v = tag(&self.local, Origin::Local);
                v.extend(tag(&self.remote, Origin::Remote));
                v
            }
            Resolution::RemoteThenLocal => {
                let mut v = tag(&self.remote, Origin::Remote);
                v.extend(tag(&self.local, Origin::Local));
                v
            }
            Resolution::Custom(lines) => tag(lines, Origin::Custom),
            Resolution::Unresolved => {
                let mut v = vec![(MARK_LOCAL, Origin::Marker)];
                v.extend(tag(&self.local, Origin::Local));
                v.push((MARK_SEP, Origin::Marker));
                v.extend(tag(&self.remote, Origin::Remote));
                v.push((MARK_REMOTE, Origin::Marker));
                v
            }
        }
    }
}

const MARK_LOCAL: &str = "<<<<<<< LOCAL";
const MARK_SEP: &str = "=======";
const MARK_REMOTE: &str = ">>>>>>> REMOTE";

pub fn default_resolution(kind: ChunkKind) -> Resolution {
    match kind {
        ChunkKind::Stable | ChunkKind::Local | ChunkKind::Both => Resolution::Local,
        ChunkKind::Remote => Resolution::Remote,
        ChunkKind::Conflict => Resolution::Unresolved,
    }
}

#[derive(Clone, Debug, Default)]
pub struct MergeDoc {
    pub chunks: Vec<Chunk>,
    pub crlf: bool,
    pub trailing_newline: bool,
    /// False when parsed from two-way conflict markers without a base section.
    pub has_base: bool,
}

fn lines_of(doc: &TextDoc, r: std::ops::Range<usize>) -> Vec<String> {
    r.map(|i| doc.line(i).to_owned()).collect()
}

/// A changed region: base[b0..b1] became side[s0..s1].
#[derive(Clone, Copy, Debug)]
struct Region {
    b0: usize,
    b1: usize,
    s0: usize,
    s1: usize,
}

fn regions(ops: &[DiffOp]) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::new();
    for op in ops {
        if matches!(op, DiffOp::Equal { .. }) {
            continue;
        }
        let (o, n) = (op.old_range(), op.new_range());
        if let Some(last) = out.last_mut()
            && last.b1 == o.start
            && last.s1 == n.start
        {
            last.b1 = o.end;
            last.s1 = n.end;
            continue;
        }
        out.push(Region {
            b0: o.start,
            b1: o.end,
            s0: n.start,
            s1: n.end,
        });
    }
    out
}

fn keys(d: &TextDoc) -> Vec<Cow<'_, str>> {
    (0..d.len()).map(|i| Cow::Borrowed(d.line(i))).collect()
}

impl MergeDoc {
    pub fn three_way(base: &TextDoc, local: &TextDoc, remote: &TextDoc) -> Self {
        let (kb, kl, kr) = (keys(base), keys(local), keys(remote));
        let rl = regions(&line_ops(&kb, &kl, true));
        let rr = regions(&line_ops(&kb, &kr, true));

        let mut chunks = Vec::new();
        let (mut i, mut j, mut pos) = (0, 0, 0);
        loop {
            let next_l = rl.get(i).map(|r| r.b0);
            let next_r = rr.get(j).map(|r| r.b0);
            let start = match (next_l, next_r) {
                (None, None) => break,
                (a, b) => a.unwrap_or(usize::MAX).min(b.unwrap_or(usize::MAX)),
            };
            if start > pos {
                let s = lines_of(base, pos..start);
                chunks.push(Chunk::new(ChunkKind::Stable, s.clone(), s.clone(), s));
            }
            // Collect all regions from both sides that overlap (or touch) the group.
            let (gl0, gr0) = (i, j);
            // Like git, changes that merely touch are treated as one region.
            let mut hi = start;
            loop {
                let mut grew = false;
                if let Some(r) = rl.get(i)
                    && r.b0 <= hi
                {
                    hi = hi.max(r.b1);
                    i += 1;
                    grew = true;
                }
                if let Some(r) = rr.get(j)
                    && r.b0 <= hi
                {
                    hi = hi.max(r.b1);
                    j += 1;
                    grew = true;
                }
                if !grew {
                    break;
                }
            }
            let lo = start;
            let side = |regs: &[Region], doc: &TextDoc| -> Option<Vec<String>> {
                let (f, l) = (regs.first()?, regs.last()?);
                Some(lines_of(doc, f.s0 - (f.b0 - lo)..l.s1 + (hi - l.b1)))
            };
            let base_lines = lines_of(base, lo..hi);
            let ll = side(&rl[gl0..i], local);
            let rl_ = side(&rr[gr0..j], remote);
            let chunk = match (ll, rl_) {
                (Some(l), None) => Chunk::new(ChunkKind::Local, base_lines.clone(), l, base_lines),
                (None, Some(r)) => {
                    Chunk::new(ChunkKind::Remote, base_lines.clone(), base_lines, r)
                }
                (Some(l), Some(r)) if l == r => Chunk::new(ChunkKind::Both, base_lines, l.clone(), l),
                (Some(l), Some(r)) => Chunk::new(ChunkKind::Conflict, base_lines, l, r),
                (None, None) => unreachable!(),
            };
            chunks.push(chunk);
            pos = hi;
        }
        if pos < base.len() {
            let s = lines_of(base, pos..base.len());
            chunks.push(Chunk::new(ChunkKind::Stable, s.clone(), s.clone(), s));
        }

        Self {
            chunks: trim_chunks(chunks),
            crlf: local.crlf,
            trailing_newline: local.trailing_newline || local.len() == 0,
            has_base: true,
        }
    }

    /// Merge without a common ancestor: every difference is a conflict.
    pub fn two_way(local: &TextDoc, remote: &TextDoc) -> Self {
        let (kl, kr) = (keys(local), keys(remote));
        let mut chunks: Vec<Chunk> = Vec::new();
        for op in line_ops(&kl, &kr, true) {
            let (o, n) = (op.old_range(), op.new_range());
            if matches!(op, DiffOp::Equal { .. }) {
                let s = lines_of(local, o);
                chunks.push(Chunk::new(ChunkKind::Stable, s.clone(), s.clone(), s));
            } else if let Some(last) = chunks.last_mut().filter(|c| c.kind == ChunkKind::Conflict) {
                last.local.extend(lines_of(local, o));
                last.remote.extend(lines_of(remote, n));
            } else {
                chunks.push(Chunk::new(ChunkKind::Conflict, Vec::new(), lines_of(local, o), lines_of(remote, n)));
            }
        }
        Self {
            chunks,
            crlf: local.crlf,
            trailing_newline: local.trailing_newline || local.len() == 0,
            has_base: false,
        }
    }

    /// Parse a file containing git conflict markers (merge or diff3 style).
    /// Returns `None` if the file has no conflicts.
    pub fn from_conflict_markers(doc: &TextDoc) -> Option<Self> {
        #[derive(PartialEq)]
        enum State {
            Outside,
            Local,
            Base,
            Remote,
        }
        let mut chunks = Vec::new();
        let mut stable: Vec<String> = Vec::new();
        let (mut local, mut base, mut remote) = (Vec::new(), Vec::new(), Vec::new());
        let mut state = State::Outside;
        let mut saw_base = false;
        let mut conflicts = 0;
        for i in 0..doc.len() {
            let line = doc.line(i);
            match state {
                State::Outside if line.starts_with("<<<<<<<") => {
                    if !stable.is_empty() {
                        let s = std::mem::take(&mut stable);
                        chunks.push(Chunk::new(ChunkKind::Stable, s.clone(), s.clone(), s));
                    }
                    state = State::Local;
                }
                State::Outside => stable.push(line.to_owned()),
                State::Local | State::Base if line.starts_with("=======") => state = State::Remote,
                State::Local if line.starts_with("|||||||") => {
                    saw_base = true;
                    state = State::Base;
                }
                State::Local => local.push(line.to_owned()),
                State::Base => base.push(line.to_owned()),
                State::Remote if line.starts_with(">>>>>>>") => {
                    conflicts += 1;
                    chunks.push(Chunk::new(
                        ChunkKind::Conflict,
                        std::mem::take(&mut base),
                        std::mem::take(&mut local),
                        std::mem::take(&mut remote),
                    ));
                    state = State::Outside;
                }
                State::Remote => remote.push(line.to_owned()),
            }
        }
        if conflicts == 0 || state != State::Outside {
            return None;
        }
        if !stable.is_empty() {
            chunks.push(Chunk::new(ChunkKind::Stable, stable.clone(), stable.clone(), stable));
        }
        Some(Self {
            chunks,
            crlf: doc.crlf,
            trailing_newline: doc.trailing_newline,
            has_base: saw_base,
        })
    }

    pub fn conflict_count(&self) -> usize {
        self.chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Conflict)
            .count()
    }

    pub fn unresolved_count(&self) -> usize {
        self.chunks
            .iter()
            .filter(|c| c.resolution == Resolution::Unresolved && c.kind == ChunkKind::Conflict)
            .count()
    }

    pub fn output(&self) -> String {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        let lines: Vec<&str> = self
            .chunks
            .iter()
            .flat_map(|c| c.result().into_iter().map(|(l, _)| l))
            .collect();
        let mut s = lines.join(eol);
        if self.trailing_newline && !lines.is_empty() {
            s.push_str(eol);
        }
        s
    }

    /// Reconstruct one side as a document (for syntax highlighting).
    pub fn side_doc(&self, f: impl Fn(&Chunk) -> &Vec<String>) -> TextDoc {
        let lines: Vec<&str> = self
            .chunks
            .iter()
            .flat_map(|c| f(c).iter().map(String::as_str))
            .collect();
        TextDoc::from_lines(&lines, self.crlf, self.trailing_newline)
    }
}

/// Move lines that are identical in base, local and remote from the edges of
/// change chunks into stable chunks, and merge adjacent stable chunks.
fn trim_chunks(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::with_capacity(chunks.len());
    let push_stable = |out: &mut Vec<Chunk>, lines: Vec<String>| {
        if lines.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut().filter(|c| c.kind == ChunkKind::Stable) {
            last.base.extend(lines.iter().cloned());
            last.local.extend(lines.iter().cloned());
            last.remote.extend(lines);
        } else {
            out.push(Chunk::new(ChunkKind::Stable, lines.clone(), lines.clone(), lines));
        }
    };
    for mut c in chunks {
        if c.kind == ChunkKind::Stable {
            push_stable(&mut out, c.base);
            continue;
        }
        let same = |c: &Chunk, i: usize, j: usize, k: usize| c.base[i] == c.local[j] && c.base[i] == c.remote[k];
        let mut head = 0;
        while head < c.base.len().min(c.local.len()).min(c.remote.len()) && same(&c, head, head, head) {
            head += 1;
        }
        let mut tail = 0;
        while tail < (c.base.len() - head).min(c.local.len() - head).min(c.remote.len() - head)
            && same(&c, c.base.len() - 1 - tail, c.local.len() - 1 - tail, c.remote.len() - 1 - tail)
        {
            tail += 1;
        }
        let prefix: Vec<String> = c.base[..head].to_vec();
        let suffix: Vec<String> = c.base[c.base.len() - tail..].to_vec();
        for v in [&mut c.base, &mut c.local, &mut c.remote] {
            v.truncate(v.len() - tail);
            v.drain(..head);
        }
        push_stable(&mut out, prefix);
        out.push(c);
        push_stable(&mut out, suffix);
    }
    out
}

pub fn has_conflict_markers(doc: &TextDoc) -> bool {
    (0..doc.len()).any(|i| doc.line(i).starts_with("<<<<<<<"))
        && (0..doc.len()).any(|i| doc.line(i).starts_with(">>>>>>>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> TextDoc {
        TextDoc::from_string(s.to_owned())
    }

    #[test]
    fn auto_merges_disjoint_changes() {
        let base = doc("a\nb\nc\nd\ne\n");
        let local = doc("A\nb\nc\nd\ne\n");
        let remote = doc("a\nb\nc\nd\nE\n");
        let m = MergeDoc::three_way(&base, &local, &remote);
        assert_eq!(m.conflict_count(), 0);
        assert_eq!(m.output(), "A\nb\nc\nd\nE\n");
    }

    #[test]
    fn detects_conflict() {
        let base = doc("a\nb\nc\n");
        let local = doc("a\nX\nc\n");
        let remote = doc("a\nY\nc\n");
        let mut m = MergeDoc::three_way(&base, &local, &remote);
        assert_eq!(m.conflict_count(), 1);
        assert_eq!(
            m.output(),
            "a\n<<<<<<< LOCAL\nX\n=======\nY\n>>>>>>> REMOTE\nc\n"
        );
        let c = m
            .chunks
            .iter_mut()
            .find(|c| c.kind == ChunkKind::Conflict)
            .unwrap();
        c.resolution = Resolution::RemoteThenLocal;
        assert_eq!(m.output(), "a\nY\nX\nc\n");
    }

    #[test]
    fn identical_change_is_not_conflict() {
        let m = MergeDoc::three_way(&doc("a\nb\n"), &doc("a\nZ\n"), &doc("a\nZ\n"));
        assert_eq!(m.conflict_count(), 0);
        assert_eq!(m.output(), "a\nZ\n");
    }

    #[test]
    fn trims_shared_lines() {
        let base = doc("fn a() {\n    x\n}\n");
        let local = doc("fn a() {\n    y\n}\n");
        let remote = doc("/// doc\nfn a() {\n    x\n}\n");
        let m = MergeDoc::three_way(&base, &local, &remote);
        assert!(m.chunks.iter().all(|c| c.kind == ChunkKind::Stable || c.base.len() <= 1));
        assert_eq!(m.output(), "/// doc\nfn a() {\n    y\n}\n");
    }

    #[test]
    fn two_way_conflicts() {
        let m = MergeDoc::two_way(&doc("a\nb\nc\n"), &doc("a\nX\nc\n"));
        assert_eq!(m.conflict_count(), 1);
        assert_eq!(m.chunks[1].local, vec!["b"]);
        assert_eq!(m.chunks[1].remote, vec!["X"]);
    }

    #[test]
    fn parses_markers() {
        let d = doc("x\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> b\ny\n");
        let m = MergeDoc::from_conflict_markers(&d).unwrap();
        assert!(m.has_base);
        assert_eq!(m.chunks.len(), 3);
        assert_eq!(m.chunks[1].local, vec!["ours"]);
        assert_eq!(m.chunks[1].base, vec!["orig"]);
        assert_eq!(m.chunks[1].remote, vec!["theirs"]);
    }
}

