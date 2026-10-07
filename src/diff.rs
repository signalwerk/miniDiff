//! Line based side-by-side diff with word-level emphasis.
//!
//! Consecutive deletions are paired with the following insertions (GitHub
//! style), and paired lines get a word-level diff when they are similar enough
//! to make that useful.

use std::borrow::Cow;
use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::text::TextDoc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Delete,
    Insert,
    Modified,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub kind: RowKind,
    /// Line index in the left document.
    pub left: Option<usize>,
    /// Line index in the right document.
    pub right: Option<usize>,
    /// Emphasised (changed) byte ranges within the left line.
    pub left_emph: Vec<Range<usize>>,
    pub right_emph: Vec<Range<usize>>,
}

/// A run of consecutive changed rows: `rows[start..end]`.
#[derive(Clone, Copy, Debug)]
pub struct Hunk {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiffOptions {
    pub ignore_whitespace: bool,
    pub patience: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
            patience: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FileDiff {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub removed: usize,
}

impl FileDiff {
    pub fn is_identical(&self) -> bool {
        self.hunks.is_empty()
    }

    pub fn hunk_of_row(&self, row: usize) -> Option<usize> {
        self.hunks.iter().position(|h| h.start <= row && row < h.end)
    }
}

fn line_keys(doc: &TextDoc, ignore_ws: bool) -> Vec<Cow<'_, str>> {
    (0..doc.len())
        .map(|i| {
            let l = doc.line(i);
            if ignore_ws {
                Cow::Owned(l.split_whitespace().collect::<Vec<_>>().join(" "))
            } else {
                Cow::Borrowed(l)
            }
        })
        .collect()
}

pub fn line_ops(a: &[Cow<'_, str>], b: &[Cow<'_, str>], patience: bool) -> Vec<DiffOp> {
    let alg = if patience {
        Algorithm::Patience
    } else {
        Algorithm::Myers
    };
    capture_diff_slices(alg, a, b)
}

pub fn diff_docs(a: &TextDoc, b: &TextDoc, opts: DiffOptions) -> FileDiff {
    let ka = line_keys(a, opts.ignore_whitespace);
    let kb = line_keys(b, opts.ignore_whitespace);
    let ops = line_ops(&ka, &kb, opts.patience);

    let mut rows = Vec::with_capacity(a.len().max(b.len()));
    let mut dels: Vec<usize> = Vec::new();
    let mut ins: Vec<usize> = Vec::new();

    let flush = |rows: &mut Vec<Row>, dels: &mut Vec<usize>, ins: &mut Vec<usize>| {
        let n = dels.len().max(ins.len());
        for j in 0..n {
            let l = dels.get(j).copied();
            let r = ins.get(j).copied();
            let mut row = Row {
                kind: match (l, r) {
                    (Some(_), Some(_)) => RowKind::Modified,
                    (Some(_), None) => RowKind::Delete,
                    _ => RowKind::Insert,
                },
                left: l,
                right: r,
                left_emph: Vec::new(),
                right_emph: Vec::new(),
            };
            if let (Some(l), Some(r)) = (l, r)
                && let Some((le, re)) = word_diff(a.line(l), b.line(r)) {
                    row.left_emph = le;
                    row.right_emph = re;
                }
            rows.push(row);
        }
        dels.clear();
        ins.clear();
    };

    for op in ops {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                flush(&mut rows, &mut dels, &mut ins);
                for k in 0..len {
                    rows.push(Row {
                        kind: RowKind::Equal,
                        left: Some(old_index + k),
                        right: Some(new_index + k),
                        left_emph: Vec::new(),
                        right_emph: Vec::new(),
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                if !ins.is_empty() {
                    flush(&mut rows, &mut dels, &mut ins);
                }
                dels.extend(old_index..old_index + old_len);
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => ins.extend(new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                if !ins.is_empty() {
                    flush(&mut rows, &mut dels, &mut ins);
                }
                dels.extend(old_index..old_index + old_len);
                ins.extend(new_index..new_index + new_len);
            }
        }
    }
    flush(&mut rows, &mut dels, &mut ins);

    let mut hunks = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    let mut start = None;
    for (i, row) in rows.iter().enumerate() {
        if row.kind == RowKind::Equal {
            if let Some(s) = start.take() {
                hunks.push(Hunk { start: s, end: i });
            }
        } else {
            start.get_or_insert(i);
            added += row.right.is_some() as usize;
            removed += row.left.is_some() as usize;
        }
    }
    if let Some(s) = start {
        hunks.push(Hunk {
            start: s,
            end: rows.len(),
        });
    }

    FileDiff {
        rows,
        hunks,
        added,
        removed,
    }
}

/// Split a line into word, whitespace and punctuation tokens (byte ranges).
fn tokenize(s: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut iter = s.char_indices().peekable();
    while let Some((start, c)) = iter.next() {
        let cls = class(c);
        let mut end = start + c.len_utf8();
        if cls != Class::Other {
            while let Some(&(i, n)) = iter.peek() {
                if class(n) != cls {
                    break;
                }
                end = i + n.len_utf8();
                iter.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// Word-level diff of two lines. Returns `None` when the lines share too little
/// content for word highlighting to be helpful (like git's diff-highlight).
/// Emphasised byte ranges for the old and the new line.
pub type WordDiff = (Vec<Range<usize>>, Vec<Range<usize>>);

pub fn word_diff(old: &str, new: &str) -> Option<WordDiff> {
    let ta = tokenize(old);
    let tb = tokenize(new);
    let wa: Vec<&str> = ta.iter().map(|r| &old[r.clone()]).collect();
    let wb: Vec<&str> = tb.iter().map(|r| &new[r.clone()]).collect();
    let ops = capture_diff_slices(Algorithm::Myers, &wa, &wb);

    let mut le: Vec<Range<usize>> = Vec::new();
    let mut re: Vec<Range<usize>> = Vec::new();
    let mut unchanged = 0usize;
    let push = |v: &mut Vec<Range<usize>>, r: Range<usize>| {
        if r.is_empty() {
            return;
        }
        if let Some(last) = v.last_mut()
            && last.end == r.start
        {
            last.end = r.end;
            return;
        }
        v.push(r);
    };
    for op in ops {
        let (o, n) = (op.old_range(), op.new_range());
        match op {
            DiffOp::Equal { .. } => {
                unchanged += wa[o].iter().map(|w| w.trim().len()).sum::<usize>();
            }
            _ => {
                if !o.is_empty() {
                    push(&mut le, ta[o.start].start..ta[o.end - 1].end);
                }
                if !n.is_empty() {
                    push(&mut re, tb[n.start].start..tb[n.end - 1].end);
                }
            }
        }
    }
    let total = old.trim().len().max(new.trim().len());
    if total == 0 || (unchanged as f64) / (total as f64) < 0.2 {
        return None;
    }
    Some((le, re))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> TextDoc {
        TextDoc::from_string(s.to_owned())
    }

    #[test]
    fn pairs_modified_lines() {
        let d = diff_docs(
            &doc("a\nlet x = 1;\nc\n"),
            &doc("a\nlet x = 2;\nc\nd\n"),
            DiffOptions::default(),
        );
        let kinds: Vec<_> = d.rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RowKind::Equal,
                RowKind::Modified,
                RowKind::Equal,
                RowKind::Insert
            ]
        );
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.added, d.removed), (2, 1));
        assert_eq!(d.rows[1].left_emph, vec![8..9]);
        assert_eq!(d.rows[1].right_emph, vec![8..9]);
    }

    #[test]
    fn ignore_whitespace() {
        let opts = DiffOptions {
            ignore_whitespace: true,
            ..Default::default()
        };
        let d = diff_docs(&doc("a  b\n"), &doc("a b\n"), opts);
        assert!(d.is_identical());
    }
}
