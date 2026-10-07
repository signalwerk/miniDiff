//! Decoded text documents split into lines.

use std::ops::Range;

/// Files above this size are not diffed line by line.
pub const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct TextDoc {
    pub text: String,
    /// Byte range of every line, without the line terminator.
    pub lines: Vec<Range<usize>>,
    pub binary: bool,
    pub crlf: bool,
    pub trailing_newline: bool,
    pub size: usize,
}

impl TextDoc {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let size = bytes.len();
        let probe = &bytes[..bytes.len().min(8000)];
        if probe.contains(&0) || size > MAX_TEXT_BYTES {
            return Self {
                binary: true,
                size,
                ..Default::default()
            };
        }
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = String::from_utf8_lossy(bytes).into_owned();
        let mut doc = Self::from_string(text);
        doc.size = size;
        doc
    }

    pub fn from_string(text: String) -> Self {
        let mut lines = Vec::new();
        let mut start = 0;
        let mut crlf = false;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                let mut end = i;
                if end > start && text.as_bytes()[end - 1] == b'\r' {
                    end -= 1;
                    crlf = true;
                }
                lines.push(start..end);
                start = i + 1;
            }
        }
        let trailing_newline = start == text.len() && !text.is_empty();
        if start < text.len() {
            lines.push(start..text.len());
        }
        Self {
            size: text.len(),
            text,
            lines,
            binary: false,
            crlf,
            trailing_newline,
        }
    }

    pub fn from_lines(lines: &[&str], crlf: bool, trailing_newline: bool) -> Self {
        let eol = if crlf { "\r\n" } else { "\n" };
        let mut text = lines.join(eol);
        if trailing_newline && !lines.is_empty() {
            text.push_str(eol);
        }
        Self::from_string(text)
    }

    #[inline]
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", |r| &self.text[r.clone()])
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn max_line_chars(&self) -> usize {
        self.lines
            .iter()
            .map(|r| {
                let l = &self.text[r.clone()];
                l.chars().count() + l.matches('\t').count() * (TAB_WIDTH - 1)
            })
            .max()
            .unwrap_or(0)
    }
}

pub const TAB_WIDTH: usize = 4;

pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines() {
        let d = TextDoc::from_string("a\r\nb\nc".into());
        assert_eq!(d.len(), 3);
        assert_eq!(d.line(0), "a");
        assert_eq!(d.line(2), "c");
        assert!(d.crlf);
        assert!(!d.trailing_newline);
        let d = TextDoc::from_string("a\n".into());
        assert_eq!(d.len(), 1);
        assert!(d.trailing_newline);
        assert_eq!(TextDoc::from_string(String::new()).len(), 0);
    }
}
