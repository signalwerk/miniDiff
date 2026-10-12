//! Three-way merge / conflict resolution view.
//!
//! Columns: A (local, ours) · [Base] · Result · B (remote, theirs).
//! Every change gets an action bar to pick A, B, both, base or a custom edit.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;

use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Key, Layout, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::code::{CodeStyle, line_job, paint_filler, paint_job, paint_line_number};
use super::{ViewSettings, icon_button, keys_free, toggle, toolbar_frame, dialog_modal, dialog_footer};
use crate::highlight::{Highlights, highlight};
use crate::diff::{DiffOptions, diff_docs};
use crate::merge::{ChunkKind, MergeDoc, Origin, Resolution, default_resolution};
use crate::text::TextDoc;
use crate::theme::Palette;

type LineEmphasis = Vec<Vec<Range<usize>>>;

fn pair_emphasis(a: &[String], b: &[String]) -> [LineEmphasis; 2] {
    let doc = |lines: &[String]| TextDoc::from_lines(&lines.iter().map(String::as_str).collect::<Vec<_>>(), false, true);
    let diff = diff_docs(&doc(a), &doc(b), DiffOptions::default());
    let mut left = vec![Vec::new(); a.len()];
    let mut right = vec![Vec::new(); b.len()];
    for row in diff.rows {
        if let Some(i) = row.left { left[i] = row.left_emph; }
        if let Some(i) = row.right { right[i] = row.right_emph; }
    }
    [left, right]
}

fn input_emphasis(doc: &MergeDoc) -> Vec<[LineEmphasis; 3]> {
    doc.chunks.iter().map(|c| {
        if !doc.has_base {
            let [local, remote] = pair_emphasis(&c.local, &c.remote);
            return [local, vec![Vec::new(); c.base.len()], remote];
        }
        let [mut base, local] = pair_emphasis(&c.base, &c.local);
        let [other_base, remote] = pair_emphasis(&c.base, &c.remote);
        for (ranges, other) in base.iter_mut().zip(other_base) {
            ranges.extend(other);
            ranges.sort_by_key(|r| r.start);
            let mut merged: Vec<Range<usize>> = Vec::new();
            for r in ranges.drain(..) {
                if let Some(last) = merged.last_mut() && r.start <= last.end {
                    last.end = last.end.max(r.end);
                } else {
                    merged.push(r);
                }
            }
            *ranges = merged;
        }
        [local, base, remote]
    }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MItem {
    Action(usize),
    Line { chunk: usize, k: usize },
    Fold { chunk: usize, from: usize, to: usize },
}

impl MItem {
    fn chunk(self) -> usize {
        match self {
            Self::Action(c) | Self::Line { chunk: c, .. } | Self::Fold { chunk: c, .. } => c,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Col {
    Local,
    Base,
    Result,
    Remote,
}

fn includes(resolution: &Resolution, col: Col) -> bool {
    matches!((resolution, col),
        (Resolution::Local, Col::Local)
        | (Resolution::Remote, Col::Remote)
        | (Resolution::LocalThenRemote | Resolution::RemoteThenLocal, Col::Local | Col::Remote)
        | (Resolution::Base, Col::Base))
}

fn dim_color(pal: &Palette, alpha: u8) -> Color32 {
    let [r, g, b, _] = pal.bg.to_array();
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeAction {
    None,
    /// Saved; close the window if launched as a merge tool.
    SavedAndClose,
    /// Close without saving (merge tool reports failure).
    Cancel,
}

pub struct MergeView {
    id: u64,
    pub doc: MergeDoc,
    pub file_name: String,
    pub output: Option<PathBuf>,
    pub labels: [String; 3],
    /// Launched as a merge tool (git/Tower): offer Save & Close / Cancel.
    pub tool_mode: bool,
    /// Built-in practice merge: changes are disposable and cannot be saved.
    pub demo: bool,
    pub unsaved: bool,
    /// At least one successful save, and nothing changed since.
    pub saved: bool,

    local: TextDoc,
    base: TextDoc,
    remote: TextDoc,
    local_hl: Highlights,
    base_hl: Highlights,
    remote_hl: Highlights,
    starts: Vec<[usize; 3]>,
    input_emph: Vec<[LineEmphasis; 3]>,

    result: TextDoc,
    result_hl: Highlights,
    result_origins: Vec<Origin>,
    result_starts: Vec<usize>,
    result_lens: Vec<usize>,
    result_emph: LineEmphasis,

    items: Vec<MItem>,
    /// Overview markers: item range and the chunk they belong to.
    markers: Vec<(usize, usize, usize)>,
    built_for: Option<(bool, bool, usize, usize)>,
    expanded: HashSet<usize>,
    current: Option<usize>,
    scroll_to: Option<usize>,
    pending_offset: Option<f32>,
    last_offset: f32,
    last_view_h: f32,
    h_off: f32,
    h_max: f32,
    editing: Option<(usize, String)>,
    confirm_save: Option<bool>,
    close_flag: bool,
    pub message: Option<(String, bool)>,
}

impl MergeView {
    pub fn new(doc: MergeDoc, file_name: String, output: Option<PathBuf>, labels: [String; 3], tool_mode: bool) -> Self {
        let local = doc.side_doc(|c| &c.local);
        let base = doc.side_doc(|c| &c.base);
        let remote = doc.side_doc(|c| &c.remote);
        let mut starts = Vec::with_capacity(doc.chunks.len());
        let mut acc = [0usize; 3];
        for c in &doc.chunks {
            starts.push(acc);
            acc[0] += c.local.len();
            acc[1] += c.base.len();
            acc[2] += c.remote.len();
        }
        let mut view = Self {
            id: super::next_id(),
            local_hl: highlight(&local, &file_name),
            base_hl: highlight(&base, &file_name),
            remote_hl: highlight(&remote, &file_name),
            local,
            base,
            remote,
            starts,
            input_emph: input_emphasis(&doc),
            doc,
            file_name,
            output,
            labels,
            tool_mode,
            demo: false,
            unsaved: false,
            saved: false,
            result: TextDoc::default(),
            result_hl: Highlights::default(),
            result_origins: Vec::new(),
            result_starts: Vec::new(),
            result_lens: Vec::new(),
            result_emph: Vec::new(),
            items: Vec::new(),
            markers: Vec::new(),
            built_for: None,
            expanded: HashSet::new(),
            current: None,
            scroll_to: None,
            pending_offset: None,
            last_offset: 0.0,
            last_view_h: 600.0,
            h_off: 0.0,
            h_max: 0.0,
            editing: None,
            confirm_save: None,
            close_flag: false,
            message: None,
        };
        view.refresh_result();
        if let Some(first) = view.next_conflict(None, true).or_else(|| view.next_change(None)) {
            view.current = Some(first);
            view.scroll_to = Some(first);
        }
        view
    }

    /// Exchange A and B while keeping resolved output and in-progress edits.
    pub fn swap(&mut self) {
        let previous_result = self.result.text.clone();
        for c in &mut self.doc.chunks {
            std::mem::swap(&mut c.local, &mut c.remote);
            c.kind = match c.kind {
                ChunkKind::Local => ChunkKind::Remote,
                ChunkKind::Remote => ChunkKind::Local,
                kind => kind,
            };
            c.resolution = match &c.resolution {
                Resolution::Local => Resolution::Remote,
                Resolution::Remote => Resolution::Local,
                Resolution::LocalThenRemote => Resolution::RemoteThenLocal,
                Resolution::RemoteThenLocal => Resolution::LocalThenRemote,
                res => res.clone(),
            };
        }
        self.labels.swap(0, 2);
        std::mem::swap(&mut self.local, &mut self.remote);
        std::mem::swap(&mut self.local_hl, &mut self.remote_hl);
        for start in &mut self.starts {
            start.swap(0, 2);
        }
        for emph in &mut self.input_emph {
            emph.swap(0, 2);
        }
        self.refresh_result();
        // Unresolved marker blocks exchange their sides too, changing saved text.
        if self.result.text != previous_result {
            self.unsaved = true;
            self.saved = false;
        }
    }

    fn refresh_result(&mut self) {
        let mut lines: Vec<&str> = Vec::new();
        self.result_origins.clear();
        self.result_starts.clear();
        self.result_lens.clear();
        self.result_emph.clear();
        for (ci, c) in self.doc.chunks.iter().enumerate() {
            let r = c.result();
            let custom = if let Resolution::Custom(lines) = &c.resolution {
                let [_, ranges] = pair_emphasis(if self.doc.has_base { &c.base } else { &c.local }, lines);
                ranges
            } else { Vec::new() };
            let mut offsets = [0; 3];
            self.result_starts.push(lines.len());
            self.result_lens.push(r.len());
            for (l, o) in r {
                lines.push(l);
                self.result_origins.push(o);
                let side = match o {
                    Origin::Local => Some(0),
                    Origin::Base | Origin::Stable => Some(1),
                    Origin::Remote => Some(2),
                    _ => None,
                };
                let emph = if let Some(side) = side {
                    let ranges = self.input_emph[ci][side].get(offsets[side]).cloned().unwrap_or_default();
                    offsets[side] += 1;
                    ranges
                } else if o == Origin::Custom {
                    custom.get(lines.len() - 1 - self.result_starts[ci]).cloned().unwrap_or_default()
                } else { Vec::new() };
                self.result_emph.push(emph);
            }
        }
        self.result = TextDoc::from_lines(&lines, self.doc.crlf, self.doc.trailing_newline);
        self.result_hl = highlight(&self.result, &self.file_name);
        self.built_for = None;
    }

    fn set_resolution(&mut self, chunk: usize, res: Resolution) {
        if let Some(c) = self.doc.chunks.get_mut(chunk)
            && c.resolution != res
        {
            c.resolution = res;
            self.unsaved = true;
            self.saved = false;
            self.refresh_result();
        }
    }

    fn resolve_all(&mut self, res: Resolution) {
        for c in &mut self.doc.chunks {
            if c.kind == ChunkKind::Conflict {
                c.resolution = res.clone();
            }
        }
        self.unsaved = true;
        self.refresh_result();
    }

    fn next_conflict(&self, from: Option<usize>, unresolved_only: bool) -> Option<usize> {
        let start = from.map_or(0, |f| f + 1);
        (start..self.doc.chunks.len()).find(|&i| {
            let c = &self.doc.chunks[i];
            c.kind == ChunkKind::Conflict && (!unresolved_only || c.resolution == Resolution::Unresolved)
        })
    }

    fn next_change(&self, from: Option<usize>) -> Option<usize> {
        let start = from.map_or(0, |f| f + 1);
        (start..self.doc.chunks.len()).find(|&i| self.doc.chunks[i].is_change())
    }

    fn prev_change(&self, from: Option<usize>) -> Option<usize> {
        let end = from.unwrap_or(self.doc.chunks.len());
        (0..end).rev().find(|&i| self.doc.chunks[i].is_change())
    }

    fn goto(&mut self, chunk: Option<usize>) {
        if let Some(c) = chunk {
            self.current = Some(c);
            self.scroll_to = Some(c);
        }
    }

    /// After resolving, jump to the next unresolved conflict (wrapping).
    fn advance(&mut self) {
        let next = self
            .next_conflict(self.current, true)
            .or_else(|| self.next_conflict(None, true));
        if next.is_some() {
            self.goto(next);
        }
    }

    // ------------------------------------------------------------------

    fn rebuild_items(&mut self, show_base: bool, collapse: bool, ctx: usize) {
        let key = (show_base, collapse, ctx, self.expanded.len());
        if self.built_for == Some(key) && !self.items.is_empty() {
            return;
        }
        self.built_for = Some(key);
        let last = self.doc.chunks.len().saturating_sub(1);
        let mut items = Vec::new();
        for (ci, c) in self.doc.chunks.iter().enumerate() {
            if c.kind == ChunkKind::Stable {
                let n = c.base.len();
                let head = if ci == 0 { 0 } else { ctx };
                let tail = if ci == last { 0 } else { ctx };
                if collapse && n > head + tail + 2 && !self.expanded.contains(&ci) {
                    items.extend((0..head).map(|k| MItem::Line { chunk: ci, k }));
                    items.push(MItem::Fold { chunk: ci, from: head, to: n - tail });
                    items.extend((n - tail..n).map(|k| MItem::Line { chunk: ci, k }));
                } else {
                    items.extend((0..n).map(|k| MItem::Line { chunk: ci, k }));
                }
                continue;
            }
            items.push(MItem::Action(ci));
            let mut h = c.local.len().max(c.remote.len()).max(self.result_lens[ci]);
            if show_base {
                h = h.max(c.base.len());
            }
            items.extend((0..h.max(1)).map(|k| MItem::Line { chunk: ci, k }));
        }
        self.items = items;
        self.rebuild_markers();
    }

    fn chunk_color(&self, ci: usize, pal: &Palette) -> Option<Color32> {
        let c = &self.doc.chunks[ci];
        match c.kind {
            ChunkKind::Stable => None,
            ChunkKind::Conflict if c.resolution == Resolution::Unresolved => Some(pal.conflict),
            ChunkKind::Conflict => Some(pal.add_fg),
            ChunkKind::Local | ChunkKind::Both => Some(pal.local),
            ChunkKind::Remote => Some(pal.remote),
        }
    }

    fn rebuild_markers(&mut self) {
        // Colours depend on theme and resolution, so they are picked at paint time.
        let mut markers: Vec<(usize, usize, usize)> = Vec::new();
        for (idx, it) in self.items.iter().enumerate() {
            let ci = it.chunk();
            if !self.doc.chunks[ci].is_change() {
                continue;
            }
            match markers.last_mut() {
                Some(m) if m.1 == idx && m.2 == ci => m.1 = idx + 1,
                _ => markers.push((idx, idx + 1, ci)),
            }
        }
        self.markers = markers;
    }

    fn item_of_chunk(&self, ci: usize) -> Option<usize> {
        self.items.iter().position(|it| it.chunk() == ci)
    }

    // ------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut ViewSettings) -> MergeAction {
        let st = CodeStyle::new(ui.ctx(), settings.font_size);
        let pal = st.pal;
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut action = MergeAction::None;
        let show_base = settings.show_base && self.doc.has_base;

        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.request_save(false);
        }
        if keys_free(ui.ctx()) && self.editing.is_none() && self.confirm_save.is_none() {
            self.handle_keys(ui, settings, st.row_h);
        }

        self.toolbar(ui, settings, pal, &mut action);
        self.rebuild_items(show_base, settings.collapse, settings.context);
        self.headers(ui, &st, show_base);
        self.body(ui, &st, show_base);
        self.modals(ui, &mut action);
        action
    }

    fn handle_keys(&mut self, ui: &mut Ui, settings: &mut ViewSettings, row_h: f32) {
        let k = |ui: &mut Ui, m: Modifiers, key: Key| ui.input_mut(|i| i.consume_key(m, key));
        if k(ui, Modifiers::NONE, Key::N) || k(ui, Modifiers::ALT, Key::ArrowDown) || k(ui, Modifiers::NONE, Key::J) {
            let n = self.next_change(self.current);
            self.goto(n);
        }
        if k(ui, Modifiers::NONE, Key::P) || k(ui, Modifiers::ALT, Key::ArrowUp) || k(ui, Modifiers::NONE, Key::K) {
            let p = self.prev_change(self.current);
            self.goto(p);
        }
        if k(ui, Modifiers::NONE, Key::C) {
            settings.collapse = !settings.collapse;
        }
        if let Some(cur) = self.current {
            let choice = if k(ui, Modifiers::NONE, Key::A) || k(ui, Modifiers::NONE, Key::Num1) {
                Some(Resolution::Local)
            } else if k(ui, Modifiers::NONE, Key::B) || k(ui, Modifiers::NONE, Key::Num2) {
                Some(Resolution::Remote)
            } else if k(ui, Modifiers::NONE, Key::Num3) {
                Some(Resolution::LocalThenRemote)
            } else if k(ui, Modifiers::NONE, Key::Num4) {
                Some(Resolution::RemoteThenLocal)
            } else if k(ui, Modifiers::NONE, Key::Num0) && self.doc.has_base {
                Some(Resolution::Base)
            } else if k(ui, Modifiers::NONE, Key::R) {
                Some(default_resolution(self.doc.chunks[cur].kind))
            } else {
                None
            };
            if let Some(res) = choice {
                self.set_resolution(cur, res);
                self.advance();
            }
            if k(ui, Modifiers::NONE, Key::E) {
                self.start_edit(cur);
            }
        }
        let page = (self.last_view_h - 3.0 * row_h).max(row_h);
        let mut d = 0.0;
        if k(ui, Modifiers::NONE, Key::ArrowDown) {
            d += 3.0 * row_h;
        }
        if k(ui, Modifiers::NONE, Key::ArrowUp) {
            d -= 3.0 * row_h;
        }
        if k(ui, Modifiers::NONE, Key::PageDown) || k(ui, Modifiers::NONE, Key::Space) {
            d += page;
        }
        if k(ui, Modifiers::NONE, Key::PageUp) {
            d -= page;
        }
        if d != 0.0 {
            self.pending_offset = Some(self.last_offset + d);
        }
    }

    fn start_edit(&mut self, ci: usize) {
        let lines: Vec<&str> = self.doc.chunks[ci].result().into_iter().map(|(l, _)| l).collect();
        self.editing = Some((ci, lines.join("\n")));
    }

    pub fn needs_save(&self) -> bool {
        self.unsaved && !self.demo
    }

    fn request_save(&mut self, and_close: bool) {
        if self.demo { return; }
        if self.doc.unresolved_count() > 0 {
            self.confirm_save = Some(and_close);
        } else if self.save(and_close) && and_close {
            self.close_flag = true;
        }
    }

    /// Returns true on success.
    pub fn save(&mut self, _and_close: bool) -> bool {
        if self.demo { return false; }
        let out = self.doc.output();
        let Some(path) = &self.output else {
            self.message = Some(("No output file to save to".into(), true));
            return false;
        };
        match std::fs::write(path, out.as_bytes()) {
            Ok(()) => {
                self.unsaved = false;
                self.saved = true;
                self.message = Some((format!("Saved {}", path.display()), false));
                true
            }
            Err(e) => {
                self.message = Some((format!("Could not save: {e}"), true));
                false
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, settings: &mut ViewSettings, pal: &Palette, action: &mut MergeAction) {
        toolbar_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if icon_button(ui, "⏶", &format!("Previous change  (p · {}↑)", super::alt()), true).clicked() {
                    let p = self.prev_change(self.current);
                    self.goto(p);
                }
                if icon_button(ui, "⏷", &format!("Next change  (n · {}↓)", super::alt()), true).clicked() {
                    let n = self.next_change(self.current);
                    self.goto(n);
                }
                let changes = self.doc.chunks.iter().filter(|c| c.is_change()).count();
                if let Some(ci) = self.current.filter(|&ci| self.doc.chunks[ci].is_change()) {
                    let step = self.doc.chunks[..=ci].iter().filter(|c| c.is_change()).count();
                    ui.label(RichText::new(format!("Change {step} / {changes}")).color(pal.accent).strong());
                    ui.separator();
                }
                let total = self.doc.conflict_count();
                let open = self.doc.unresolved_count();
                if total == 0 {
                    ui.label(RichText::new("✔ Merged automatically — no conflicts").color(pal.add_fg).strong());
                } else if open == 0 {
                    ui.label(RichText::new(format!("✔ All {total} conflicts resolved")).color(pal.add_fg).strong());
                } else {
                    if ui.add(egui::Label::new(
                        RichText::new(format!("⚠ {open} of {total} conflict{} unresolved", if total == 1 { "" } else { "s" }))
                            .color(pal.conflict)
                            .strong(),
                    ).sense(Sense::click()))
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text("Go to the next unresolved conflict")
                        .clicked()
                    {
                        self.advance();
                    }
                }
                if total > 0 {
                    ui.separator();
                    if ui.button(RichText::new("All A").color(pal.local)).on_hover_text("Resolve every conflict with A (local)").clicked() {
                        self.resolve_all(Resolution::Local);
                    }
                    if ui.button(RichText::new("All B").color(pal.remote)).on_hover_text("Resolve every conflict with B (remote)").clicked() {
                        self.resolve_all(Resolution::Remote);
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let save_label = "Save";
                    if self.tool_mode && !self.demo {
                        let b = egui::Button::new(RichText::new("Save & Close").color(Color32::WHITE).strong()).fill(pal.accent);
                        if ui.add(b).on_hover_text(format!("Write the result and return to git ({}S saves only)", super::cmd())).clicked() {
                            self.request_save(true);
                        }
                        if ui.button("Cancel").on_hover_text("Close without saving — the merge stays unresolved").clicked() {
                            *action = MergeAction::Cancel;
                        }
                    } else if !self.demo {
                        let b = egui::Button::new(RichText::new(save_label).color(Color32::WHITE).strong()).fill(pal.accent);
                        if ui.add(b).on_hover_text(format!("{}S", super::cmd())).clicked() {
                            self.request_save(false);
                        }
                    }
                    if !self.demo { ui.separator(); }
                    toggle(ui, &mut settings.collapse, "↕ Collapse", "Collapse unchanged lines  (c)");
                    if self.doc.has_base {
                        toggle(ui, &mut settings.show_base, "Base", "Show the common ancestor");
                    }
                    if let Some((msg, err)) = &self.message {
                        ui.label(RichText::new(msg).color(if *err { pal.del_fg } else { pal.text_dim }).small());
                    }
                });
            });
        });
    }

    fn columns(&self, rect: Rect, show_base: bool) -> Vec<(Col, Rect)> {
        let cols: &[Col] = if show_base {
            &[Col::Local, Col::Base, Col::Result, Col::Remote]
        } else {
            &[Col::Local, Col::Result, Col::Remote]
        };
        let w = rect.width() / cols.len() as f32;
        cols.iter()
            .enumerate()
            .map(|(i, c)| (*c, Rect::from_min_size(pos2(rect.left() + i as f32 * w, rect.top()), vec2(w, rect.height()))))
            .collect()
    }

    fn headers(&self, ui: &mut Ui, st: &CodeStyle, show_base: bool) {
        let pal = st.pal;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let body = rect.with_max_x(rect.max.x - 12.0);
        let p = ui.painter();
        p.rect_filled(rect, 0.0, pal.panel);
        p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, pal.border));
        for (col, r) in self.columns(body, show_base) {
            let (tag, color, text) = match col {
                Col::Local => ("A", pal.local, self.labels[0].as_str()),
                Col::Base => ("◇", pal.text_dim, self.labels[1].as_str()),
                Col::Result => ("=", pal.add_fg, "Result"),
                Col::Remote => ("B", pal.remote, self.labels[2].as_str()),
            };
            let clip = p.with_clip_rect(r.shrink2(vec2(2.0, 0.0)));
            let badge = Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(18.0, 16.0));
            clip.rect_filled(badge, 4.0, color);
            clip.text(badge.center(), Align2::CENTER_CENTER, tag, FontId::proportional(11.0), Color32::WHITE);
            clip.text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), pal.text);
            if r.left() > rect.left() {
                p.vline(r.left(), r.y_range(), Stroke::new(1.0, pal.border));
            }
        }
    }

    fn body(&mut self, ui: &mut Ui, st: &CodeStyle, show_base: bool) {
        let pal = st.pal;
        let full = ui.available_rect_before_wrap();
        let strip_w = 12.0;
        let body_rect = full.with_max_x(full.max.x - strip_w);
        let strip_rect = full.with_min_x(full.max.x - strip_w);
        ui.painter().rect_filled(body_rect, 0.0, pal.bg);

        if let Some(ci) = self.scroll_to.take()
            && let Some(idx) = self.item_of_chunk(ci)
        {
            self.pending_offset = Some(idx as f32 * st.row_h - body_rect.height() / 3.0);
        }
        let mut sa = ScrollArea::vertical()
            .id_salt(("merge-body", self.id))
            .auto_shrink(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(off) = self.pending_offset.take() {
            let max = (self.items.len() as f32 * st.row_h - body_rect.height()).max(0.0);
            sa = sa.vertical_scroll_offset(off.clamp(0.0, max));
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(body_rect));
        child.spacing_mut().item_spacing = Vec2::ZERO;
        let total = self.items.len();
        let out = sa.show_rows(&mut child, st.row_h, total, |ui, range| {
            self.paint_rows(ui, range, st, show_base);
        });
        ui.advance_cursor_after_rect(full);
        self.last_offset = out.state.offset.y;
        self.last_view_h = out.inner_rect.height();
        self.overview(ui, strip_rect, out.content_size.y.max(1.0), pal, st.row_h);
    }

    fn paint_rows(&mut self, ui: &mut Ui, range: Range<usize>, st: &CodeStyle, show_base: bool) {
        let pal = st.pal;
        let max = ui.max_rect();
        let width = max.width();
        let area = Rect::from_min_size(max.min, vec2(width, range.len() as f32 * st.row_h));
        let resp = ui.interact(area, ui.id().with(("merge-rows", self.id)), Sense::click());
        let p = ui.painter().clone();
        let hovered = resp
            .hover_pos()
            .map(|pos| range.start + ((pos.y - max.top()) / st.row_h).max(0.0) as usize)
            .filter(|&i| i < range.end);
        let max_line = self.local.len().max(self.remote.len()).max(self.result.len()).max(self.base.len());
        let gw = st.gutter_width(max_line);
        let cols = self.columns(area, show_base);
        let col_w = cols[0].1.width();
        let max_chars = self
            .local
            .max_line_chars()
            .max(self.remote.max_line_chars())
            .max(self.result.max_line_chars());
        self.h_max = (max_chars as f32 * st.char_w + 40.0 - (col_w - gw - 8.0)).max(0.0);
        self.h_off = self.h_off.min(self.h_max);

        let mut clicked_action: Option<(usize, Resolution)> = None;
        let mut edit: Option<usize> = None;
        for (k, idx) in range.clone().enumerate() {
            let r = Rect::from_min_size(pos2(max.left(), max.top() + k as f32 * st.row_h), vec2(width, st.row_h));
            let item = self.items[idx];
            let ci = item.chunk();
            match item {
                MItem::Fold { from, to, .. } => {
                    p.rect_filled(r, 0.0, pal.fold_bg);
                    let hov = hovered == Some(idx);
                    p.text(
                        pos2(r.left() + gw + 8.0, r.center().y),
                        Align2::LEFT_CENTER,
                        if hov {
                            format!("⋯  {} unchanged lines — click to expand", to - from)
                        } else {
                            format!("⋯  {} unchanged lines", to - from)
                        },
                        st.small.clone(),
                        if hov { pal.accent } else { pal.text_dim },
                    );
                    if hov && resp.clicked() {
                        self.expanded.insert(ci);
                        self.built_for = None;
                    }
                }
                MItem::Action(_) => {
                    self.paint_action_row(ui, &p, r, &cols, ci, st, &mut clicked_action, &mut edit);
                }
                MItem::Line { k, .. } => {
                    for (col, cr) in &cols {
                        let cell = Rect::from_x_y_ranges(cr.x_range(), r.y_range());
                        self.paint_cell(&p, cell, *col, ci, k, gw, st);
                    }
                    for (_, cr) in cols.iter().skip(1) {
                        p.vline(cr.left(), r.y_range(), Stroke::new(1.0, pal.border));
                    }
                }
            }
            let c = &self.doc.chunks[ci];
            let focused = self.current == Some(ci) && c.is_change();
            let has_focus = self.current.is_some();
            if c.is_change() {
                for (col, cr) in &cols {
                    let color = match col {
                        Col::Local => pal.local,
                        Col::Remote => pal.remote,
                        Col::Base => pal.text_dim,
                        Col::Result => continue,
                    };
                    let cell = Rect::from_x_y_ranges(cr.x_range(), r.y_range());
                    if includes(&c.resolution, *col) {
                        p.vline(cell.left() + 1.0, cell.y_range(), Stroke::new(2.0, color));
                        p.vline(cell.right() - 1.0, cell.y_range(), Stroke::new(2.0, color));
                        if idx == 0 || self.items[idx - 1].chunk() != ci {
                            p.hline(cell.x_range(), cell.top() + 1.0, Stroke::new(2.0, color));
                        }
                        if idx + 1 == self.items.len() || self.items[idx + 1].chunk() != ci {
                            p.hline(cell.x_range(), cell.bottom() - 1.0, Stroke::new(2.0, color));
                        }
                    } else if c.resolution != Resolution::Unresolved && (focused || !has_focus) {
                        // Avoid stacking this with the spotlight dim on other chunks.
                        p.rect_filled(cell, 0.0, dim_color(pal, 50));
                    }
                }
            }
            if focused {
                let [red, green, blue, _] = pal.accent.to_array();
                p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(red, green, blue, 14));
            } else if has_focus {
                // Gentle spotlight keeps surrounding code readable.
                p.rect_filled(r, 0.0, dim_color(pal, 58));
            }
            if focused {
                p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, st.row_h)), 0.0, pal.accent);
            }
            if hovered == Some(idx) && !matches!(item, MItem::Action(_)) {
                p.rect_filled(r, 0.0, pal.hover);
            }
        }

        if resp.clicked()
            && let Some(h) = hovered
        {
            let ci = self.items[h].chunk();
            if self.doc.chunks[ci].is_change() {
                self.current = Some(ci);
            }
        }
        if resp.double_clicked()
            && let Some(h) = hovered
        {
            let ci = self.items[h].chunk();
            if self.doc.chunks[ci].is_change() {
                edit = Some(ci);
            }
        }
        if let Some((ci, res)) = clicked_action {
            self.current = Some(ci);
            self.set_resolution(ci, res);
        }
        if let Some(ci) = edit {
            self.current = Some(ci);
            self.start_edit(ci);
        }
        if resp.contains_pointer() {
            let dx = ui.input(|i| i.smooth_scroll_delta.x);
            if dx != 0.0 {
                self.h_off = (self.h_off - dx).clamp(0.0, self.h_max);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_action_row(
        &self,
        ui: &mut Ui,
        p: &egui::Painter,
        r: Rect,
        cols: &[(Col, Rect)],
        ci: usize,
        st: &CodeStyle,
        clicked: &mut Option<(usize, Resolution)>,
        edit: &mut Option<usize>,
    ) {
        let pal = st.pal;
        let c = &self.doc.chunks[ci];
        let color = self.chunk_color(ci, pal).unwrap_or(pal.border);
        p.rect_filled(r, 0.0, pal.toolbar);
        p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, color.gamma_multiply(0.7)));
        let font = FontId::proportional(11.5);

        let chip = |x: f32, text: &str, selected: bool, color: Color32, tip: &str, salt: usize| -> (f32, bool) {
            let galley = p.layout_no_wrap(text.to_owned(), font.clone(), color);
            let labels = match salt {
                1 => Some(["Use A  ▶", "✔ Included A"]),
                2 => Some(["◀  Use B", "✔ Included B"]),
                3 => Some(["Use base", "✔ Included base"]),
                _ => None,
            };
            // Reserve both label widths and the same inset in either state.
            let width = labels.map_or(galley.size().x, |labels| labels.into_iter()
                .map(|label| p.layout_no_wrap(label.into(), font.clone(), color).size().x)
                .fold(0.0, f32::max));
            let w = width + 14.0;
            // The selected outline occupies the first two points of the row.
            let top = if labels.is_some() { 4.0 } else { 2.0 };
            let rect = Rect::from_min_size(pos2(x, r.top() + top), vec2(w, r.height() - top - 2.0));
            let resp = ui
                .interact(rect, ui.id().with(("chip", self.id, ci, salt)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tip);
            let fill = if selected {
                color.gamma_multiply(0.25)
            } else if resp.hovered() {
                pal.hover
            } else {
                Color32::TRANSPARENT
            };
            p.rect_filled(rect, 4.0, fill);
            p.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, if selected { color } else { pal.border }),
                egui::StrokeKind::Inside,
            );
            p.galley(pos2(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0), galley, color);
            (x + w + 4.0, resp.clicked())
        };

        let res = &c.resolution;
        let uses_local = includes(res, Col::Local);
        let uses_remote = includes(res, Col::Remote);
        for (col, cr) in cols {
            let x0 = cr.left() + 8.0;
            match col {
                Col::Local => {
                    let (_, hit) = chip(x0, if uses_local { "✔ Included A" } else { "Use A  ▶" }, uses_local, pal.local, "Take A (local)  — key a / 1", 1);
                    if hit {
                        *clicked = Some((ci, Resolution::Local));
                    }
                }
                Col::Remote => {
                    let (_, hit) = chip(x0, if uses_remote { "✔ Included B" } else { "◀  Use B" }, uses_remote, pal.remote, "Take B (remote)  — key b / 2", 2);
                    if hit {
                        *clicked = Some((ci, Resolution::Remote));
                    }
                }
                Col::Base => {
                    let (_, hit) = chip(x0, if *res == Resolution::Base { "✔ Included base" } else { "Use base" }, *res == Resolution::Base, pal.text_dim, "Take the common ancestor  — key 0", 3);
                    if hit {
                        *clicked = Some((ci, Resolution::Base));
                    }
                }
                Col::Result => {
                    let (label, lc) = match (c.kind, res) {
                        (ChunkKind::Conflict, Resolution::Unresolved) => ("⚠ Conflict".to_owned(), pal.conflict),
                        (ChunkKind::Conflict, r) => (format!("✔ {}", describe(r)), pal.add_fg),
                        (_, r) if *r == default_resolution(c.kind) => (format!("Auto · {}", describe(r)), pal.text_dim),
                        (_, r) => (format!("✔ {}", describe(r)), pal.add_fg),
                    };
                    let galley = p.layout_no_wrap(label, FontId::proportional(11.5), lc);
                    let gw = galley.size().x;
                    p.galley(pos2(x0, r.center().y - galley.size().y / 2.0), galley, lc);
                    let mut x = x0 + gw + 10.0;
                    let (nx, hit) = chip(x, "A+B", *res == Resolution::LocalThenRemote, pal.accent, "A then B  — key 3", 4);
                    x = nx;
                    if hit {
                        *clicked = Some((ci, Resolution::LocalThenRemote));
                    }
                    let (nx, hit) = chip(x, "B+A", *res == Resolution::RemoteThenLocal, pal.accent, "B then A  — key 4", 5);
                    x = nx;
                    if hit {
                        *clicked = Some((ci, Resolution::RemoteThenLocal));
                    }
                    if self.doc.has_base {
                        let (nx, hit) = chip(x, "Base", *res == Resolution::Base, pal.text_dim, "Common ancestor  — key 0", 6);
                        x = nx;
                        if hit {
                            *clicked = Some((ci, Resolution::Base));
                        }
                    }
                    let (nx, hit) = chip(x, "Edit…", matches!(res, Resolution::Custom(_)), pal.accent, "Edit the result by hand  — key e / double-click", 7);
                    x = nx;
                    if hit {
                        *edit = Some(ci);
                    }
                    if *res != default_resolution(c.kind) {
                        let (_, hit) = chip(x, "↺", false, pal.text_dim, "Reset  — key r", 8);
                        if hit {
                            *clicked = Some((ci, default_resolution(c.kind)));
                        }
                    }
                }
            }
        }
        for (_, cr) in cols.iter().skip(1) {
            p.vline(cr.left(), r.y_range(), Stroke::new(1.0, pal.border));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_cell(&self, p: &egui::Painter, rect: Rect, col: Col, ci: usize, k: usize, gw: f32, st: &CodeStyle) {
        let pal = st.pal;
        let c = &self.doc.chunks[ci];
        let change = c.is_change();
        let (line_idx, len, doc, hl, bg, num_color): (usize, usize, &TextDoc, &Highlights, Option<Color32>, Color32) = match col {
            Col::Local => {
                let touched = matches!(c.kind, ChunkKind::Local | ChunkKind::Both | ChunkKind::Conflict);
                (
                    self.starts[ci][0] + k,
                    c.local.len(),
                    &self.local,
                    &self.local_hl,
                    touched.then_some(pal.local_bg),
                    if touched { pal.local } else { pal.gutter_fg },
                )
            }
            Col::Remote => {
                let touched = matches!(c.kind, ChunkKind::Remote | ChunkKind::Both | ChunkKind::Conflict);
                (
                    self.starts[ci][2] + k,
                    c.remote.len(),
                    &self.remote,
                    &self.remote_hl,
                    touched.then_some(pal.remote_bg),
                    if touched { pal.remote } else { pal.gutter_fg },
                )
            }
            Col::Base => (
                self.starts[ci][1] + k,
                c.base.len(),
                &self.base,
                &self.base_hl,
                change.then_some(pal.base_bg),
                pal.gutter_fg,
            ),
            Col::Result => {
                let idx = self.result_starts[ci] + k;
                let bg = if k < self.result_lens[ci] {
                    match self.result_origins[idx] {
                        Origin::Stable => None,
                        Origin::Local => Some(pal.local_bg),
                        Origin::Remote => Some(pal.remote_bg),
                        Origin::Base => Some(pal.base_bg),
                        Origin::Custom => Some(pal.custom_bg),
                        Origin::Marker => Some(pal.conflict_bg),
                    }
                } else {
                    None
                };
                let nc = if change { pal.add_fg } else { pal.gutter_fg };
                (idx, self.result_lens[ci], &self.result, &self.result_hl, bg, nc)
            }
        };
        if k >= len {
            if change {
                paint_filler(p, rect, st);
            }
            return;
        }
        if let Some(bg) = bg {
            p.rect_filled(rect, 0.0, bg);
        }
        let g = Rect::from_min_size(rect.min, vec2(gw, rect.height()));
        if bg.is_none() {
            p.rect_filled(g, 0.0, pal.gutter_bg);
        }
        paint_line_number(p, g, Some(line_idx + 1), num_color, st);
        let is_marker = col == Col::Result && self.result_origins.get(line_idx) == Some(&Origin::Marker);
        let job = if is_marker {
            let mut j = line_job(doc.line(line_idx), &[], &[], Color32::TRANSPARENT, st);
            for s in &mut j.sections {
                s.format.color = pal.conflict;
            }
            j
        } else {
            let (emph, color) = match col {
                Col::Local => (&self.input_emph[ci][0][k], pal.local_emph),
                Col::Base => (&self.input_emph[ci][1][k], pal.base_emph),
                Col::Remote => (&self.input_emph[ci][2][k], pal.remote_emph),
                Col::Result => (&self.result_emph[line_idx], match self.result_origins[line_idx] {
                    Origin::Local => pal.local_emph,
                    Origin::Remote => pal.remote_emph,
                    Origin::Base => pal.base_emph,
                    Origin::Custom => pal.custom_emph,
                    _ => Color32::TRANSPARENT,
                }),
            };
            line_job(doc.line(line_idx), hl.line(line_idx), emph, color, st)
        };
        let text_rect = Rect::from_min_max(pos2(g.right() + 8.0, rect.top()), rect.max);
        paint_job(p, text_rect, job, self.h_off, st);
    }

    fn overview(&mut self, ui: &mut Ui, rect: Rect, content_h: f32, pal: &Palette, row_h: f32) {
        let p = ui.painter();
        p.rect_filled(rect, 0.0, pal.gutter_bg);
        p.vline(rect.left(), rect.y_range(), Stroke::new(1.0, pal.border));
        // Map items onto the strip; short files only use the top part of it.
        let total = (self.items.len() as f32).max(self.last_view_h / row_h).max(1.0);
        let h = rect.height();
        for &(s, e, ci) in &self.markers {
            let Some(color) = self.chunk_color(ci, pal) else { continue };
            let y0 = rect.top() + s as f32 / total * h;
            let y1 = (rect.top() + e as f32 / total * h).max(y0 + 3.0);
            p.rect_filled(Rect::from_x_y_ranges(rect.left() + 3.0..=rect.right() - 2.0, y0..=y1), 1.0, color);
        }
        if content_h > self.last_view_h {
            let t0 = rect.top() + self.last_offset / content_h * h;
            let t1 = rect.top() + (self.last_offset + self.last_view_h) / content_h * h;
            let thumb = Rect::from_x_y_ranges(rect.left() + 1.0..=rect.right(), t0..=t1.max(t0 + 8.0));
            p.rect_filled(thumb, 2.0, pal.hover);
            p.rect_stroke(thumb, 2.0, Stroke::new(1.0, pal.text_dim.gamma_multiply(0.5)), egui::StrokeKind::Inside);
        }
        let resp = ui
            .interact(rect, ui.id().with(("merge-overview", self.id)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand);
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let frac = ((pos.y - rect.top()) / h).clamp(0.0, 1.0);
            self.pending_offset = Some(frac * content_h - self.last_view_h / 2.0);
            ui.ctx().request_repaint();
        }
    }

    fn modals(&mut self, ui: &mut Ui, action: &mut MergeAction) {
        let ctx = ui.ctx().clone();
        if let Some((ci, text)) = &mut self.editing {
            let ci = *ci;
            let mut apply = false;
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new(("edit-chunk", self.id))).show(&ctx, |ui| {
                ui.set_width(760.0_f32.min(ctx.content_rect().width() - 80.0));
                ui.heading("Edit result");
                ui.label(RichText::new("The text below replaces this change in the merged result.").weak());
                ui.add_space(6.0);
                ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.6).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .code_editor()
                            .desired_rows(14)
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Apply").strong()).clicked()
                        || ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter))
                    {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.label(RichText::new(format!("{}↩ to apply", super::cmd())).weak().small());
                });
            });
            if modal.should_close() {
                cancel = true;
            }
            if apply {
                let lines = text.split('\n').map(|l| l.trim_end_matches('\r').to_owned()).collect();
                self.editing = None;
                self.set_resolution(ci, Resolution::Custom(lines));
            } else if cancel {
                self.editing = None;
            }
        }

        if let Some(and_close) = self.confirm_save {
            let open = self.doc.unresolved_count();
            let error = self.message.as_ref().filter(|(_, error)| *error).map(|(text, _)| text.as_str());
            let modal = dialog_modal(&ctx, egui::Id::new(("confirm-save", self.id))).show(&ctx, |ui| {
                unresolved_conflicts_contents(ui, &self.file_name, open, error)
            });
            let mut decision = modal.inner;
            if modal.should_close() {
                decision = Some(false);
            }
            if let Some(d) = decision {
                if !d {
                    self.confirm_save = None;
                } else if self.save(and_close) {
                    self.confirm_save = None;
                    if and_close { *action = MergeAction::SavedAndClose; }
                }
            }
        } else if std::mem::take(&mut self.close_flag) {
            *action = MergeAction::SavedAndClose;
        }
    }
}

fn unresolved_conflicts_contents(ui: &mut Ui, file_name: &str, open: usize, error: Option<&str>) -> Option<bool> {
    let pal = crate::theme::palette(ui.ctx());
    ui.set_width(380.0_f32.min(ui.ctx().content_rect().width() - 80.0));
    ui.label(RichText::new("Unresolved conflicts").size(22.0).strong());
    ui.add_space(12.0);
    ui.add(egui::Label::new(RichText::new(file_name).size(14.0).strong()).truncate()).on_hover_text(file_name);
    ui.add_space(4.0);
    ui.label(RichText::new(format!("{open} conflict{} still unresolved.", if open == 1 { " is" } else { "s are" }))
        .size(13.0).color(pal.conflict));
    ui.add_space(4.0);
    ui.label(RichText::new("Saving now writes conflict markers into the result. Keep resolving to finish the merge first.")
        .size(13.0).color(pal.text_dim));
    if let Some(error) = error {
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(error).size(13.0).color(pal.del_fg)).truncate()).on_hover_text(error);
    }
    dialog_footer(ui, |ui| {
        let keep = egui::Button::new(RichText::new("Keep resolving").strong().color(Color32::WHITE)).fill(pal.accent);
        if ui.add_sized([120.0, 28.0], keep).clicked() { return Some(false); }
        if ui.add_sized([110.0, 28.0], egui::Button::new("Save anyway")).clicked() { return Some(true); }
        None
    })
}

fn describe(r: &Resolution) -> &'static str {
    match r {
        Resolution::Unresolved => "Unresolved",
        Resolution::Local => "A",
        Resolution::Remote => "B",
        Resolution::LocalThenRemote => "A then B",
        Resolution::RemoteThenLocal => "B then A",
        Resolution::Base => "Base",
        Resolution::Custom(_) => "Edited",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::Chunk;

    #[test]
    fn clicking_unresolved_count_skips_resolved_changes_and_wraps() {
        fn center(shape: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text().contains("2 of 3 conflicts unresolved") => Some(t.pos + t.galley.size() * 0.5),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(center),
                _ => None,
            }
        }
        let mut doc = view(Resolution::Unresolved).doc;
        let mut resolved = doc.chunks[0].clone();
        resolved.resolution = Resolution::Remote;
        doc.chunks.push(resolved);
        doc.chunks.push(doc.chunks[0].clone());
        let mut view = MergeView::new(doc, "merged.rs".into(), None,
            ["Local".into(), "Base".into(), "Remote".into()], false);
        let ctx = egui::Context::default();
        let pal = crate::theme::palette(&ctx);
        let mut settings = ViewSettings::default();
        let mut action = MergeAction::None;
        let mut pos = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| view.toolbar(ui, &mut settings, pal, &mut action));
            pos = output.shapes.iter().find_map(|s| center(&s.shape));
            output.textures_delta.clear();
        }
        let pos = pos.expect("unresolved conflict count");
        for expected in [3, 0] {
            view.scroll_to = None;
            for pressed in [true, false] {
                let mut output = ctx.run_ui(egui::RawInput {
                    events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                    }], ..Default::default()
                }, |ui| view.toolbar(ui, &mut settings, pal, &mut action));
                output.textures_delta.clear();
            }
            assert_eq!(view.current, Some(expected));
            assert_eq!(view.scroll_to, Some(expected));
            assert_eq!(view.doc.unresolved_count(), 2);
            assert!(!view.unsaved);
        }
    }

    #[test]
    fn unresolved_warning_fits_small_windows_in_both_themes() {
        for dark in [false, true] {
            for size in [vec2(640.0, 400.0), vec2(880.0, 520.0)] {
                let ctx = egui::Context::default();
                crate::theme::install(&ctx);
                ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                let screen = Rect::from_min_size(egui::Pos2::ZERO, size);
                let mut bounds = Rect::NOTHING;
                for _ in 0..4 {
                    let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                        bounds = dialog_modal(ui.ctx(), egui::Id::new("warning-fit")).show(ui.ctx(), |ui| {
                            unresolved_conflicts_contents(ui, "merged.rs", 3, Some("No output file to save to"));
                        }).response.rect;
                    });
                    output.textures_delta.clear();
                }
                assert!(screen.shrink(12.0).contains_rect(bounds));
            }
        }
    }

    #[test]
    fn unresolved_warning_keeps_failed_saves_open_and_cancels_safely() {
        fn center(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| center(s, label)),
                _ => None,
            }
        }
        for label in ["Keep resolving", "Save anyway"] {
            let ctx = egui::Context::default();
            let mut view = view(Resolution::Unresolved);
            view.output = None;
            view.unsaved = true;
            view.confirm_save = Some(true);
            let mut action = MergeAction::None;
            let mut position = None;
            for _ in 0..3 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| view.modals(ui, &mut action));
                position = output.shapes.iter().find_map(|s| center(&s.shape, label));
                output.textures_delta.clear();
            }
            let pos = position.expect("confirmation action");
            for pressed in [true, false] {
                let mut output = ctx.run_ui(egui::RawInput {
                    events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                    }], ..Default::default()
                }, |ui| view.modals(ui, &mut action));
                output.textures_delta.clear();
            }
            assert_eq!(action, MergeAction::None);
            assert!(view.unsaved);
            if label == "Keep resolving" {
                assert!(view.confirm_save.is_none() && view.message.is_none());
            } else {
                assert_eq!(view.confirm_save, Some(true));
                assert!(view.message.as_ref().is_some_and(|(_, error)| *error));
            }
        }
    }

    #[test]
    fn merge_demo_has_no_save_controls_or_keyboard_save_prompt() {
        fn texts(shape: &egui::epaint::Shape, labels: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => labels.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Vec(shapes) => for s in shapes { texts(s, labels); },
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut view = view(Resolution::Unresolved);
        view.demo = true;
        let mut settings = ViewSettings::default();
        let mut labels = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1100.0, 700.0))),
                events: vec![egui::Event::Key { key: Key::S, physical_key: None, pressed: true,
                    repeat: false, modifiers: Modifiers::COMMAND }],
                ..Default::default()
            }, |ui| { view.ui(ui, &mut settings); });
            labels.clear();
            for s in &output.shapes { texts(&s.shape, &mut labels); }
            output.textures_delta.clear();
        }
        assert!(!labels.iter().any(|s| s == "Save" || s == "Save & Close" || s == "Save anyway"));
        assert!(view.confirm_save.is_none());
        assert!(view.message.is_none());
    }

    #[test]
    fn chosen_inputs_match_each_resolution() {
        for (resolution, expected) in [
            (Resolution::Unresolved, [false, false, false]),
            (Resolution::Local, [true, false, false]),
            (Resolution::Remote, [false, false, true]),
            (Resolution::LocalThenRemote, [true, false, true]),
            (Resolution::RemoteThenLocal, [true, false, true]),
            (Resolution::Base, [false, true, false]),
            (Resolution::Custom(vec!["edited".into()]), [false, false, false]),
        ] {
            assert_eq!([Col::Local, Col::Base, Col::Remote].map(|col| includes(&resolution, col)), expected);
            assert!(!includes(&resolution, Col::Result));
        }
    }

    fn marked(text: &str, ranges: &[Range<usize>]) -> Vec<String> {
        ranges.iter().map(|r| text[r.clone()].to_owned()).collect()
    }

    #[test]
    fn inline_emphasis_follows_resolutions_custom_edits_and_swaps() {
        let text = |s: &str| TextDoc::from_string(format!("let value = {s};\n"));
        let doc = MergeDoc::three_way(&text("1"), &text("2"), &text("3"));
        let mut view = MergeView::new(doc, "value.rs".into(), None,
            ["Local".into(), "Base".into(), "Remote".into()], false);
        for (side, expected) in ["2", "1", "3"].into_iter().enumerate() {
            let doc = [&view.local, &view.base, &view.remote][side];
            assert_eq!(marked(doc.line(0), &view.input_emph[0][side][0]), [expected]);
        }
        // Conflict markers stay plain; each input's emphasis follows its result line.
        assert!(view.result_emph[0].is_empty());
        assert_eq!(marked(view.result.line(1), &view.result_emph[1]), ["2"]);
        assert_eq!(marked(view.result.line(3), &view.result_emph[3]), ["3"]);
        view.set_resolution(0, Resolution::LocalThenRemote);
        assert_eq!(marked(view.result.line(0), &view.result_emph[0]), ["2"]);
        assert_eq!(marked(view.result.line(1), &view.result_emph[1]), ["3"]);
        view.swap();
        assert_eq!(marked(view.local.line(0), &view.input_emph[0][0][0]), ["3"]);
        assert_eq!(marked(view.result.line(0), &view.result_emph[0]), ["2"]);
        assert_eq!(marked(view.result.line(1), &view.result_emph[1]), ["3"]);
        view.set_resolution(0, Resolution::Custom(vec!["let value = 4;".into()]));
        assert_eq!(marked(view.result.line(0), &view.result_emph[0]), ["4"]);
    }

    #[test]
    fn inline_emphasis_uses_line_pairing_and_utf8_ranges_without_base() {
        let local = vec!["unchanged".into(), "let café = 1;".into()];
        let remote = vec!["inserted".into(), "unchanged".into(), "let café = 2;".into()];
        let doc = MergeDoc {
            chunks: vec![Chunk {
                kind: ChunkKind::Conflict, base: Vec::new(), local, remote,
                resolution: Resolution::Unresolved,
            }],
            ..Default::default()
        };
        let emph = input_emphasis(&doc);
        assert!(emph[0][0][0].is_empty());
        assert!(emph[0][2][0].is_empty());
        assert!(emph[0][2][1].is_empty());
        assert_eq!(marked(&doc.chunks[0].local[1], &emph[0][0][1]), ["1"]);
        assert_eq!(marked(&doc.chunks[0].remote[2], &emph[0][2][2]), ["2"]);
        assert!(emph[0][1].is_empty());
    }

    #[test]
    fn base_emphasis_combines_both_sides_and_unchanged_inputs_stay_plain() {
        let text = |s: &str| TextDoc::from_string(format!("let value = {s};\n"));
        let base = text("1 + 2");
        for remote in [text("1 + 4"), base.clone()] {
            let doc = MergeDoc::three_way(&base, &text("3 + 2"), &remote);
            let emph = input_emphasis(&doc);
            assert_eq!(marked(&doc.chunks[0].local[0], &emph[0][0][0]), ["3"]);
            if remote.text == base.text {
                assert!(emph[0][2][0].is_empty());
                assert_eq!(marked(&doc.chunks[0].base[0], &emph[0][1][0]), ["1"]);
            } else {
                assert_eq!(marked(&doc.chunks[0].base[0], &emph[0][1][0]), ["1", "2"]);
            }
        }
    }

    fn view(resolution: Resolution) -> MergeView {
        let doc = MergeDoc {
            chunks: vec![
                Chunk {
                    kind: ChunkKind::Conflict,
                    base: vec!["base".into()],
                    local: vec!["local".into(), "extra".into()],
                    remote: vec!["remote".into()],
                    resolution,
                },
                Chunk {
                    kind: ChunkKind::Local,
                    base: vec!["old".into()],
                    local: vec!["new".into()],
                    remote: vec!["old".into()],
                    resolution: Resolution::Local,
                },
            ],
            has_base: true,
            trailing_newline: true,
            ..Default::default()
        };
        MergeView::new(doc, "merged.rs".into(), Some(PathBuf::from("merged.rs")),
            ["Local".into(), "Base".into(), "Remote".into()], true)
    }

    #[test]
    fn swap_preserves_resolved_output_edits_and_save_state() {
        for resolution in [Resolution::Local, Resolution::Remote, Resolution::LocalThenRemote,
            Resolution::RemoteThenLocal, Resolution::Base, Resolution::Custom(vec!["edited".into()])] {
            let mut view = view(resolution.clone());
            let output = view.result.text.clone();
            view.saved = true;
            view.editing = Some((0, "pending edit".into()));
            view.swap();
            assert_eq!(view.result.text, output);
            assert!(view.saved && !view.unsaved);
            assert_eq!(view.labels, ["Remote", "Base", "Local"]);
            assert_eq!(view.local.line(0), "remote");
            assert_eq!(view.remote.line(0), "local");
            assert_eq!(view.starts[1], [1, 1, 2]);
            assert_eq!(view.doc.chunks[1].kind, ChunkKind::Remote);
            assert_eq!(view.doc.chunks[1].resolution, Resolution::Remote);
            assert_eq!(view.editing, Some((0, "pending edit".into())));
            assert_eq!(view.output, Some(PathBuf::from("merged.rs")));
            assert!(view.tool_mode);
            assert!(view.built_for.is_none());
            view.swap();
            assert_eq!(view.result.text, output);
            assert_eq!(view.doc.chunks[0].resolution, resolution);
            assert_eq!(view.starts[1], [2, 1, 1]);
            assert_eq!(view.doc.chunks[1].kind, ChunkKind::Local);
        }
    }

    #[test]
    fn swap_keeps_conflicts_unresolved_and_marks_changed_markers_unsaved() {
        let mut view = view(Resolution::Unresolved);
        let output = view.result.text.clone();
        view.saved = true;
        view.swap();
        assert_eq!(view.doc.chunks[0].resolution, Resolution::Unresolved);
        assert!(view.result.text.starts_with("<<<<<<< LOCAL\nremote\n=======\nlocal\nextra\n"));
        assert!(view.unsaved && !view.saved);
        view.swap();
        assert_eq!(view.result.text, output);
        assert!(view.unsaved && !view.saved);
    }
}
