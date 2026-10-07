//! Tree-sitter based syntax highlighting.
//!
//! The whole file is highlighted at once (so multi-line constructs such as
//! block comments and template strings are coloured correctly) and the result
//! is split into per-line spans.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::text::TextDoc;

/// Highlight classes the UI knows colours for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hl {
    Plain,
    Comment,
    Keyword,
    String,
    Escape,
    Number,
    Constant,
    Function,
    Macro,
    Type,
    VariableBuiltin,
    Parameter,
    Property,
    Operator,
    Punctuation,
    Tag,
    Attribute,
    Module,
    Label,
    Heading,
    Link,
}

/// Capture names we ask tree-sitter-highlight to resolve, and their class.
/// tree-sitter picks the longest matching prefix, e.g. `function.method.call`
/// resolves to `function.method`.
const NAMES: &[(&str, Hl)] = &[
    ("attribute", Hl::Attribute),
    ("boolean", Hl::Constant),
    ("character", Hl::String),
    ("comment", Hl::Comment),
    ("constant", Hl::Constant),
    ("constant.builtin", Hl::Constant),
    ("constant.numeric", Hl::Number),
    ("constructor", Hl::Type),
    ("delimiter", Hl::Punctuation),
    ("embedded", Hl::Plain),
    ("escape", Hl::Escape),
    ("float", Hl::Number),
    ("function", Hl::Function),
    ("function.builtin", Hl::Function),
    ("function.macro", Hl::Macro),
    ("function.method", Hl::Function),
    ("keyword", Hl::Keyword),
    ("label", Hl::Label),
    ("markup.heading", Hl::Heading),
    ("markup.link", Hl::Link),
    ("markup.raw", Hl::String),
    ("module", Hl::Module),
    ("namespace", Hl::Module),
    ("number", Hl::Number),
    ("operator", Hl::Operator),
    ("property", Hl::Property),
    ("punctuation", Hl::Punctuation),
    ("punctuation.bracket", Hl::Punctuation),
    ("punctuation.delimiter", Hl::Punctuation),
    ("punctuation.special", Hl::Operator),
    ("string", Hl::String),
    ("string.escape", Hl::Escape),
    ("string.special", Hl::Escape),
    ("string.special.key", Hl::Property),
    ("tag", Hl::Tag),
    ("text.literal", Hl::String),
    ("text.reference", Hl::Link),
    ("text.title", Hl::Heading),
    ("text.uri", Hl::Link),
    ("type", Hl::Type),
    ("type.builtin", Hl::Type),
    ("variable", Hl::Plain),
    ("variable.builtin", Hl::VariableBuiltin),
    ("variable.member", Hl::Property),
    ("variable.parameter", Hl::Parameter),
];

struct LangDef {
    name: &'static str,
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    load: fn() -> (Language, String),
}

fn q(parts: &[&str]) -> String {
    parts.join("\n")
}

const LANGS: &[LangDef] = &[
    LangDef {
        name: "Rust",
        extensions: &["rs"],
        file_names: &[],
        load: || (tree_sitter_rust::LANGUAGE.into(), q(&[tree_sitter_rust::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        load: || {
            (
                tree_sitter_javascript::LANGUAGE.into(),
                q(&[
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ]),
            )
        },
    },
    LangDef {
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        load: || {
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                q(&[
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                ]),
            )
        },
    },
    LangDef {
        name: "TSX",
        extensions: &["tsx"],
        file_names: &[],
        load: || {
            (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                q(&[
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ]),
            )
        },
    },
    LangDef {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        load: || (tree_sitter_python::LANGUAGE.into(), q(&[tree_sitter_python::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "JSON",
        extensions: &["json", "jsonc", "json5", "webmanifest"],
        file_names: &[".prettierrc", ".babelrc", ".eslintrc"],
        load: || (tree_sitter_json::LANGUAGE.into(), q(&[tree_sitter_json::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "HTML",
        extensions: &["html", "htm", "xhtml", "vue", "svelte"],
        file_names: &[],
        load: || (tree_sitter_html::LANGUAGE.into(), q(&[tree_sitter_html::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "CSS",
        extensions: &["css", "scss", "less"],
        file_names: &[],
        load: || (tree_sitter_css::LANGUAGE.into(), q(&[tree_sitter_css::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "Go",
        extensions: &["go"],
        file_names: &[],
        load: || (tree_sitter_go::LANGUAGE.into(), q(&[tree_sitter_go::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "Shell",
        extensions: &["sh", "bash", "zsh", "command"],
        file_names: &[".bashrc", ".zshrc", ".profile", ".bash_profile", ".zprofile", ".envrc"],
        load: || (tree_sitter_bash::LANGUAGE.into(), q(&[tree_sitter_bash::HIGHLIGHT_QUERY])),
    },
    LangDef {
        name: "C",
        extensions: &["c", "h"],
        file_names: &[],
        load: || (tree_sitter_c::LANGUAGE.into(), q(&[tree_sitter_c::HIGHLIGHT_QUERY])),
    },
    LangDef {
        name: "C++",
        extensions: &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "ipp", "m", "mm"],
        file_names: &[],
        load: || {
            (
                tree_sitter_cpp::LANGUAGE.into(),
                q(&[tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY]),
            )
        },
    },
    LangDef {
        name: "Java",
        extensions: &["java"],
        file_names: &[],
        load: || (tree_sitter_java::LANGUAGE.into(), q(&[tree_sitter_java::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "TOML",
        extensions: &["toml"],
        file_names: &["Cargo.lock", "Pipfile"],
        load: || (tree_sitter_toml_ng::LANGUAGE.into(), q(&[tree_sitter_toml_ng::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "YAML",
        extensions: &["yml", "yaml"],
        file_names: &[],
        load: || (tree_sitter_yaml::LANGUAGE.into(), q(&[tree_sitter_yaml::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec", "ru"],
        file_names: &["Gemfile", "Rakefile", "Podfile", "Fastfile"],
        load: || (tree_sitter_ruby::LANGUAGE.into(), q(&[tree_sitter_ruby::HIGHLIGHTS_QUERY])),
    },
    LangDef {
        name: "Markdown",
        extensions: &["md", "markdown", "mdx"],
        file_names: &[],
        load: || (tree_sitter_md::LANGUAGE.into(), q(&[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK])),
    },
];

fn lang_for(file_name: &str) -> Option<&'static LangDef> {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    if let Some(l) = LANGS.iter().find(|l| l.file_names.contains(&base)) {
        return Some(l);
    }
    let ext = base.rsplit_once('.')?.1.to_ascii_lowercase();
    LANGS.iter().find(|l| l.extensions.contains(&ext.as_str()))
}

/// Human readable language name for a file, if we can highlight it.
pub fn language_name(file_name: &str) -> Option<&'static str> {
    lang_for(file_name).map(|l| l.name)
}

thread_local! {
    static CONFIGS: RefCell<HashMap<&'static str, Option<Rc<HighlightConfiguration>>>> =
        RefCell::new(HashMap::new());
}

fn config_for(lang: &'static LangDef) -> Option<Rc<HighlightConfiguration>> {
    CONFIGS.with(|c| {
        c.borrow_mut()
            .entry(lang.name)
            .or_insert_with(|| {
                let (language, query) = (lang.load)();
                match HighlightConfiguration::new(language, lang.name, &query, "", "") {
                    Ok(mut cfg) => {
                        let names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
                        cfg.configure(&names);
                        Some(Rc::new(cfg))
                    }
                    Err(e) => {
                        log::warn!("highlight query for {} failed: {e:?}", lang.name);
                        None
                    }
                }
            })
            .clone()
    })
}

/// A highlighted span within a line: byte range relative to the line start.
pub type Span = (Range<usize>, Hl);

/// Per-line highlight spans for a whole document.
#[derive(Clone, Debug, Default)]
pub struct Highlights {
    lines: Vec<Vec<Span>>,
}

impl Highlights {
    pub fn line(&self, i: usize) -> &[Span] {
        self.lines.get(i).map_or(&[], Vec::as_slice)
    }
}

/// Above this size we skip highlighting to keep the UI responsive.
const MAX_HIGHLIGHT_BYTES: usize = 4 * 1024 * 1024;

pub fn highlight(doc: &TextDoc, file_name: &str) -> Highlights {
    if doc.binary || doc.text.len() > MAX_HIGHLIGHT_BYTES {
        return Highlights::default();
    }
    let Some(cfg) = lang_for(file_name).and_then(config_for) else {
        return Highlights::default();
    };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(&cfg, doc.text.as_bytes(), None, None, |_| None) else {
        return Highlights::default();
    };

    let mut lines: Vec<Vec<Span>> = vec![Vec::new(); doc.len()];
    let mut stack: Vec<Hl> = Vec::new();
    // Line index containing byte offset `pos`, found by walking forward.
    let mut line = 0usize;
    for event in events {
        let Ok(event) = event else { break };
        match event {
            HighlightEvent::HighlightStart(h) => {
                stack.push(NAMES.get(h.0).map_or(Hl::Plain, |(_, hl)| *hl));
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(&hl) = stack.last() else { continue };
                if hl == Hl::Plain {
                    continue;
                }
                // A source range may span several lines.
                while line < doc.lines.len() && doc.lines[line].end < start {
                    line += 1;
                }
                let mut l = line;
                while l < doc.lines.len() && doc.lines[l].start < end {
                    let lr = &doc.lines[l];
                    let s = start.max(lr.start);
                    let e = end.min(lr.end);
                    if s < e {
                        lines[l].push((s - lr.start..e - lr.start, hl));
                    }
                    l += 1;
                }
            }
        }
    }
    Highlights { lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_queries_compile() {
        for lang in LANGS {
            assert!(config_for(lang).is_some(), "{} query failed", lang.name);
        }
    }

    #[test]
    fn highlights_rust() {
        let doc = TextDoc::from_string("/* a\n b */\nfn main() { let x = 42; }\n".into());
        let h = highlight(&doc, "x.rs");
        assert_eq!(h.line(0), &[(0..4, Hl::Comment)]);
        assert_eq!(h.line(1)[0], (0..5, Hl::Comment));
        assert!(h.line(2).iter().any(|(_, hl)| *hl == Hl::Keyword));
        assert!(h.line(2).iter().any(|(_, hl)| *hl == Hl::Constant));
    }
}
