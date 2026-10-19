//! Two-way file diff: side-by-side or unified, with syntax highlighting.

use std::collections::HashSet;
use std::ops::Range;

use egui::{
    Align, Align2, Color32, CursorIcon, Key, Layout, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::code::{CodeStyle, line_job, paint_filler, paint_job, paint_line_number, ChangeSides, paint_change_marker};
use super::{ViewSettings, icon_button, keys_free, segmented, toggle, toolbar_frame};
use crate::diff::{DiffOptions, FileDiff, RowKind, diff_docs};
use crate::highlight::{Highlights, highlight, language_name};
use crate::source::Entry;
use crate::text::{TextDoc, human_size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// Side by side row, or an unchanged row in unified mode.
    Row(usize),
    /// Unified: the left (deleted) half of a row.
    Left(usize),
    /// Unified: the right (inserted) half of a row.
    Right(usize),
    /// Collapsed unchanged rows `start..end`.
    Fold { start: usize, end: usize },
}

impl Item {
    fn row(self) -> usize {
        match self {
            Self::Row(i) | Self::Left(i) | Self::Right(i) => i,
            Self::Fold { start, .. } => start,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Add,
    Del,
    Mod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    None,
    NextFile,
    PrevFile,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

pub struct FileView {
    id: u64,
    pub left_label: String,
    pub right_label: String,
    left_entry: Option<Entry>,
    right_entry: Option<Entry>,
    left: TextDoc,
    right: TextDoc,
    left_hl: Highlights,
    right_hl: Highlights,
    pub diff: FileDiff,
    pub lang: Option<&'static str>,
    opts: DiffOptions,
    bytes_equal: bool,
    error: Option<String>,

    items: Vec<Item>,
    visual_offsets: Vec<usize>,
    layout_key: Option<(f32, f32, bool)>,
    wrap_width: Option<f32>,
    markers: Vec<(usize, usize, Mark)>,
    built_for: Option<(bool, bool, usize)>,
    expanded: HashSet<usize>,
    pub current_hunk: Option<usize>,
    scroll_to_hunk: Option<usize>,
    pending_offset: Option<f32>,
    last_offset: f32,
    last_view_h: f32,
    h_off: f32,
    h_max: f32,
    max_chars: usize,
    menu_item: Option<usize>,
}

fn load(entry: Option<&Entry>) -> (TextDoc, Option<std::sync::Arc<Vec<u8>>>, Option<String>) {
    match entry.map(Entry::read) {
        None => (TextDoc::default(), None, None),
        Some(Ok(bytes)) => (TextDoc::from_bytes(&bytes), Some(bytes), None),
        Some(Err(e)) => (TextDoc::default(), None, Some(e)),
    }
}

impl FileView {
    pub fn new(
        left: Option<Entry>,
        right: Option<Entry>,
        labels: Option<(String, String)>,
        opts: DiffOptions,
    ) -> Self {
        let (ld, lb, le) = load(left.as_ref());
        let (rd, rb, re) = load(right.as_ref());
        let file_name = right
            .as_ref()
            .or(left.as_ref())
            .map(Entry::name)
            .unwrap_or_default();
        let (left_label, right_label) = labels.unwrap_or_else(|| {
            (
                left.as_ref().map_or_else(String::new, Entry::display_path),
                right.as_ref().map_or_else(String::new, Entry::display_path),
            )
        });
        let lang = language_name(&file_name);
        let mut view = Self {
            id: super::next_id(),
            left_label,
            right_label,
            left_hl: highlight(&ld, &file_name),
            right_hl: highlight(&rd, &file_name),
            max_chars: ld.max_line_chars().max(rd.max_line_chars()),
            bytes_equal: lb == rb,
            error: le.or(re),
            diff: FileDiff::default(),
            left: ld,
            right: rd,
            left_entry: left,
            right_entry: right,
            lang,
            opts,
            items: Vec::new(),
            visual_offsets: vec![0],
            layout_key: None,
            wrap_width: None,
            markers: Vec::new(),
            built_for: None,
            expanded: HashSet::new(),
            current_hunk: None,
            scroll_to_hunk: None,
            pending_offset: None,
            last_offset: 0.0,
            last_view_h: 600.0,
            h_off: 0.0,
            h_max: 0.0,
            menu_item: None,
        };
        view.recompute();
        if !view.diff.hunks.is_empty() {
            view.current_hunk = Some(0);
            view.scroll_to_hunk = Some(0);
        }
        view
    }

    pub fn entries(&self) -> (Option<Entry>, Option<Entry>) {
        (self.left_entry.clone(), self.right_entry.clone())
    }

    fn recompute(&mut self) {
        self.diff = if self.is_binary() {
            FileDiff::default()
        } else {
            diff_docs(&self.left, &self.right, self.opts)
        };
        self.built_for = None;
        self.expanded.clear();
        if self.current_hunk.is_some_and(|h| h >= self.diff.hunks.len()) {
            self.current_hunk = None;
        }
    }

    fn is_binary(&self) -> bool {
        self.left.binary || self.right.binary
    }

    // ---------------------------------------------------------------------
    // Layout model

    fn rebuild_items(&mut self, s: &ViewSettings) {
        let key = (s.unified, s.collapse, s.context);
        if self.built_for == Some(key) {
            return;
        }
        self.built_for = Some(key);
        let rows = &self.diff.rows;
        let n = rows.len();
        let mut visible = vec![!s.collapse; n];
        if s.collapse {
            for h in &self.diff.hunks {
                for v in &mut visible[h.start.saturating_sub(s.context)..(h.end + s.context).min(n)] {
                    *v = true;
                }
            }
        }
        let mut items = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            if !visible[i] {
                let start = i;
                while i < n && !visible[i] {
                    i += 1;
                }
                if self.expanded.contains(&start) || i - start <= 2 {
                    items.extend((start..i).map(Item::Row));
                } else {
                    items.push(Item::Fold { start, end: i });
                }
                continue;
            }
            if s.unified && rows[i].kind != RowKind::Equal {
                let start = i;
                while i < n && rows[i].kind != RowKind::Equal {
                    i += 1;
                }
                items.extend((start..i).filter(|&k| rows[k].left.is_some()).map(Item::Left));
                items.extend((start..i).filter(|&k| rows[k].right.is_some()).map(Item::Right));
                continue;
            }
            items.push(Item::Row(i));
            i += 1;
        }

        let mut markers: Vec<(usize, usize, Mark)> = Vec::new();
        for (idx, item) in items.iter().enumerate() {
            let mark = match *item {
                Item::Row(r) => match rows[r].kind {
                    RowKind::Equal => None,
                    RowKind::Insert => Some(Mark::Add),
                    RowKind::Delete => Some(Mark::Del),
                    RowKind::Modified => Some(Mark::Mod),
                },
                Item::Left(_) => Some(Mark::Del),
                Item::Right(_) => Some(Mark::Add),
                Item::Fold { .. } => None,
            };
            if let Some(m) = mark {
                match markers.last_mut() {
                    Some(last) if last.1 == idx && last.2 == m => last.1 = idx + 1,
                    _ => markers.push((idx, idx + 1, m)),
                }
            }
        }
        self.layout_key = None;
        self.items = items;
        self.markers = markers;
    }

    fn item_of_hunk(&self, h: usize) -> Option<usize> {
        let start = self.diff.hunks.get(h)?.start;
        self.items
            .iter()
            .position(|it| !matches!(it, Item::Fold { .. }) && it.row() >= start)
    }

    fn goto_hunk(&mut self, h: usize) {
        self.current_hunk = Some(h);
        self.scroll_to_hunk = Some(h);
    }

    /// Returns false if there is no next change (caller may jump to the next file).
    pub fn next_hunk(&mut self) -> bool {
        let n = self.diff.hunks.len();
        let next = match self.current_hunk {
            Some(c) if c + 1 < n => c + 1,
            Some(_) => return false,
            None if n > 0 => 0,
            None => return false,
        };
        self.goto_hunk(next);
        true
    }

    pub fn prev_hunk(&mut self) -> bool {
        match self.current_hunk {
            Some(c) if c > 0 => {
                self.goto_hunk(c - 1);
                true
            }
            None if !self.diff.hunks.is_empty() => {
                self.goto_hunk(self.diff.hunks.len() - 1);
                true
            }
            _ => false,
        }
    }

    // ---------------------------------------------------------------------
    // UI

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut ViewSettings, folder_nav: bool) -> FileAction {
        let mut action = FileAction::None;
        if settings.diff != self.opts {
            self.opts = settings.diff;
            self.recompute();
        }
        let st = CodeStyle::new(ui.ctx(), settings.font_size);
        ui.spacing_mut().item_spacing.y = 0.0;

        if keys_free(ui.ctx()) {
            action = self.handle_keys(ui, settings, folder_nav, st.row_h);
        }

        self.toolbar(ui, settings, &st, folder_nav, &mut action);
        self.rebuild_items(settings);

        if let Some(err) = &self.error {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("⚠ Could not read file").size(18.0).color(st.pal.del_fg));
                ui.label(RichText::new(err).color(st.pal.text_dim));
            });
            return action;
        }
        if self.is_binary() {
            self.binary_ui(ui, &st);
            return action;
        }
        self.headers(ui, &st, settings.unified);
        self.body(ui, &st, settings.unified, settings.wrap);
        action
    }

    fn handle_keys(&mut self, ui: &mut Ui, s: &mut ViewSettings, folder_nav: bool, row_h: f32) -> FileAction {
        let mut action = FileAction::None;
        let (next, prev, uni, col, ws, wrap) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::N)
                    || i.consume_key(Modifiers::ALT, Key::ArrowDown)
                    || i.consume_key(Modifiers::NONE, Key::J),
                i.consume_key(Modifiers::NONE, Key::P)
                    || i.consume_key(Modifiers::ALT, Key::ArrowUp)
                    || i.consume_key(Modifiers::NONE, Key::K),
                i.consume_key(Modifiers::NONE, Key::U),
                i.consume_key(Modifiers::NONE, Key::C),
                i.consume_key(Modifiers::NONE, Key::W),
                i.consume_key(Modifiers::NONE, Key::T),
            )
        });
        if next && !self.next_hunk() && folder_nav {
            action = FileAction::NextFile;
        }
        if prev && !self.prev_hunk() && folder_nav {
            action = FileAction::PrevFile;
        }
        if uni {
            s.unified = !s.unified;
        }
        if col {
            s.collapse = !s.collapse;
        }
        if wrap {
            s.wrap = !s.wrap;
        }
        if ws {
            s.diff.ignore_whitespace = !s.diff.ignore_whitespace;
        }

        let page = (self.last_view_h - 3.0 * row_h).max(row_h);
        let scroll = ui.input_mut(|i| {
            let mut d = 0.0;
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                d += 3.0 * row_h;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                d -= 3.0 * row_h;
            }
            if i.consume_key(Modifiers::NONE, Key::PageDown) || i.consume_key(Modifiers::NONE, Key::Space) {
                d += page;
            }
            if i.consume_key(Modifiers::NONE, Key::PageUp) || i.consume_key(Modifiers::SHIFT, Key::Space) {
                d -= page;
            }
            if i.consume_key(Modifiers::NONE, Key::Home) {
                d = -f32::INFINITY;
            }
            if i.consume_key(Modifiers::NONE, Key::End) {
                d = f32::INFINITY;
            }
            let mut h = 0.0;
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                h += 40.0;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                h -= 40.0;
            }
            (d, h)
        });
        if scroll.0 != 0.0 {
            let target = if scroll.0.is_infinite() {
                if scroll.0 > 0.0 { 1e9 } else { 0.0 }
            } else {
                self.last_offset + scroll.0
            };
            self.pending_offset = Some(target);
        }
        if scroll.1 != 0.0 {
            self.h_off = (self.h_off + scroll.1).clamp(0.0, self.h_max);
        }
        action
    }

    fn toolbar(
        &mut self,
        ui: &mut Ui,
        s: &mut ViewSettings,
        st: &CodeStyle,
        folder_nav: bool,
        action: &mut FileAction,
    ) {
        let pal = st.pal;
        toolbar_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let n = self.diff.hunks.len();
                if icon_button(ui, "⏶", &format!("Previous change  (p · {}↑)", super::alt()), n > 0 || folder_nav).clicked()
                    && !self.prev_hunk()
                    && folder_nav
                {
                    *action = FileAction::PrevFile;
                }
                if icon_button(ui, "⏷", &format!("Next change  (n · {}↓)", super::alt()), n > 0 || folder_nav).clicked()
                    && !self.next_hunk()
                    && folder_nav
                {
                    *action = FileAction::NextFile;
                }
                let status = if self.is_binary() {
                    if self.bytes_equal { "Binary · identical" } else { "Binary · different" }.to_owned()
                } else if n == 0 {
                    "No differences".to_owned()
                } else {
                    match self.current_hunk {
                        Some(c) => format!("Change {} of {n}", c + 1),
                        None => format!("{n} change{}", if n == 1 { "" } else { "s" }),
                    }
                };
                ui.label(RichText::new(status).strong());
                if !self.is_binary() && n > 0 {
                    ui.separator();
                    ui.label(RichText::new(format!("+{}", self.diff.added)).color(pal.add_fg).monospace());
                    ui.label(RichText::new(format!("−{}", self.diff.removed)).color(pal.del_fg).monospace());
                }
                if let Some(lang) = self.lang {
                    ui.separator();
                    ui.label(RichText::new(lang).color(pal.text_dim));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle(ui, &mut s.diff.ignore_whitespace, "Whitespace", "Ignore whitespace changes  (w)");
                    toggle(ui, &mut s.wrap, "Wrap", "Wrap long lines  (t)");
                    toggle(ui, &mut s.collapse, "↕ Collapse", "Collapse unchanged lines  (c)");
                    segmented(
                        ui,
                        &mut s.unified,
                        &[
                            (false, "◫ Side by side", "Two columns  (u)"),
                            (true, "☰ Unified", "One column  (u)"),
                        ],
                    );
                });
            });
        });
    }

    fn headers(&self, ui: &mut Ui, st: &CodeStyle, unified: bool) {
        let pal = st.pal;
        let h = 26.0;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, pal.panel);
        p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, pal.border));
        let font = egui::FontId::proportional(12.5);
        let label = |x: f32, w: f32, tag: &str, color: Color32, text: &str, missing: bool| {
            let clip = p.with_clip_rect(Rect::from_min_size(pos2(x, rect.top()), vec2(w - 8.0, h)));
            let badge = Rect::from_center_size(pos2(x + 18.0, rect.center().y), vec2(18.0, 16.0));
            clip.rect_filled(badge, 4.0, color);
            clip.text(badge.center(), Align2::CENTER_CENTER, tag, egui::FontId::proportional(11.0), Color32::WHITE);
            let (t, c) = if missing {
                ("does not exist", pal.text_dim)
            } else {
                (text, pal.text)
            };
            clip.text(pos2(x + 34.0, rect.center().y), Align2::LEFT_CENTER, t, font.clone(), c);
        };
        let missing_l = self.left_entry.is_none();
        let missing_r = self.right_entry.is_none();
        if unified {
            let w = rect.width() / 2.0;
            label(rect.left(), w, "A", pal.del_fg, &self.left_label, missing_l);
            label(rect.left() + w, w, "B", pal.add_fg, &self.right_label, missing_r);
        } else {
            let w = (rect.width() - 12.0) / 2.0;
            label(rect.left(), w, "A", pal.del_fg, &self.left_label, missing_l);
            label(rect.left() + w, w, "B", pal.add_fg, &self.right_label, missing_r);
            p.vline(rect.left() + w, rect.y_range(), Stroke::new(1.0, pal.border));
        }
    }

    fn binary_ui(&self, ui: &mut Ui, st: &CodeStyle) {
        let pal = st.pal;
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            let (icon, msg, color) = if self.bytes_equal {
                ("✔", "Binary files are identical", pal.text_dim)
            } else {
                ("≠", "Binary files differ", pal.mod_fg)
            };
            ui.label(RichText::new(icon).size(40.0).color(color));
            ui.label(RichText::new(msg).size(18.0).strong());
            ui.add_space(8.0);
            let size = |d: &TextDoc, e: &Option<Entry>| {
                if e.is_some() { human_size(d.size as u64) } else { "—".into() }
            };
            ui.label(
                RichText::new(format!(
                    "A: {}    B: {}",
                    size(&self.left, &self.left_entry),
                    size(&self.right, &self.right_entry)
                ))
                .color(pal.text_dim)
                .monospace(),
            );
        });
    }

    // Cache visual row counts; scrolling still paints only the visible logical lines.
    fn layout_rows(&mut self, ui: &Ui, st: &CodeStyle, width: f32, unified: bool, wrap: bool) {
        let key = (width, st.font.size, wrap);
        if self.layout_key == Some(key) {
            return;
        }
        self.layout_key = Some(key);
        let gw = st.gutter_width(self.left.len().max(self.right.len()));
        let text_w = if unified {
            width - 2.0 * gw - st.char_w - 18.0
        } else {
            width / 2.0 - gw - 8.0
        }.max(st.char_w);
        self.wrap_width = wrap.then_some(text_w);
        self.visual_offsets.clear();
        self.visual_offsets.push(0);
        for &item in &self.items {
            let mut count = 1;
            if wrap && !matches!(item, Item::Fold { .. }) {
                let row = &self.diff.rows[item.row()];
                for (line, doc, hl, emph, included) in [
                    (row.left, &self.left, &self.left_hl, &row.left_emph, !unified || matches!(item, Item::Left(_))),
                    (row.right, &self.right, &self.right_hl, &row.right_emph, !matches!(item, Item::Left(_))),
                ] {
                    if included && let Some(line) = line {
                        let job = self.wrap_job(line_job(doc.line(line), hl.line(line), emph, st.pal.del_emph, st), st);
                        count = count.max(ui.painter().layout_job(job).rows.len());
                    }
                }
            }
            self.visual_offsets.push(self.visual_offsets.last().unwrap() + count);
        }
        if wrap {
            self.h_off = 0.0;
            self.h_max = 0.0;
        }
    }

    fn wrap_job(&self, mut job: egui::text::LayoutJob, st: &CodeStyle) -> egui::text::LayoutJob {
        if let Some(width) = self.wrap_width {
            job.wrap.max_width = width;
            for section in &mut job.sections {
                section.format.line_height = Some(st.row_h);
            }
        }
        job
    }

    fn paint_text(&self, p: &egui::Painter, rect: Rect, job: egui::text::LayoutJob, st: &CodeStyle) {
        if self.wrap_width.is_some() {
            let galley = p.layout_job(self.wrap_job(job, st));
            p.with_clip_rect(rect.intersect(p.clip_rect())).galley(rect.min, galley, st.pal.text);
        } else {
            paint_job(p, rect, job, self.h_off, st);
        }
    }

    fn body(&mut self, ui: &mut Ui, st: &CodeStyle, unified: bool, wrap: bool) {
        let pal = st.pal;
        let full = ui.available_rect_before_wrap();
        let strip_w = 12.0;
        let body_rect = full.with_max_x(full.max.x - strip_w);
        let strip_rect = full.with_min_x(full.max.x - strip_w);
        ui.painter().rect_filled(body_rect, 0.0, pal.bg);

        self.layout_rows(ui, st, body_rect.width(), unified, wrap);

        if let Some(h) = self.scroll_to_hunk.take()
            && let Some(idx) = self.item_of_hunk(h)
        {
            self.pending_offset = Some(self.visual_offsets[idx] as f32 * st.row_h - body_rect.height() / 3.0);
        }

        let mut sa = ScrollArea::vertical()
            .id_salt(("file-body", self.id))
            .auto_shrink(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(off) = self.pending_offset.take() {
            let max = (*self.visual_offsets.last().unwrap() as f32 * st.row_h - body_rect.height()).max(0.0);
            sa = sa.vertical_scroll_offset(off.clamp(0.0, max));
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(body_rect));
        child.spacing_mut().item_spacing = Vec2::ZERO;
        let total = *self.visual_offsets.last().unwrap();
        let out = sa.show_rows(&mut child, st.row_h, total, |ui, range| {
            self.paint_rows(ui, range, st, unified);
        });
        ui.advance_cursor_after_rect(full);

        self.last_offset = out.state.offset.y;
        self.last_view_h = out.inner_rect.height();

        if self.diff.is_identical() && total > 0 {
            let banner = Rect::from_center_size(
                pos2(body_rect.center().x, body_rect.top() + 26.0),
                vec2(220.0, 30.0),
            );
            let p = ui.painter();
            p.rect_filled(banner, 15.0, pal.panel);
            p.rect_stroke(banner, 15.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            p.text(
                banner.center(),
                Align2::CENTER_CENTER,
                "✔  Files are identical",
                egui::FontId::proportional(13.0),
                pal.text_dim,
            );
        }

        self.overview(ui, strip_rect, out.content_size.y.max(1.0), st);
    }

    fn paint_rows(&mut self, ui: &mut Ui, range: Range<usize>, st: &CodeStyle, unified: bool) {
        let pal = st.pal;
        let max = ui.max_rect();
        let top = max.top();
        let width = max.width();
        let area = Rect::from_min_size(max.min, vec2(width, range.len() as f32 * st.row_h));
        let resp = ui.allocate_rect(area, Sense::click());
        let p = ui.painter().clone();

        let hovered = resp
            .hover_pos()
            .map(|pos| range.start + ((pos.y - top) / st.row_h).max(0.0) as usize)
            .filter(|&i| i < range.end)
            .map(|visual| self.visual_offsets.partition_point(|&offset| offset <= visual) - 1);

        let max_line = self.left.len().max(self.right.len());
        let gw = st.gutter_width(max_line);
        let current = self.current_hunk.and_then(|h| self.diff.hunks.get(h)).copied();
        let text_w = if unified {
            width - 2.0 * gw - st.char_w - 16.0
        } else {
            width / 2.0 - gw - 8.0
        };
        self.h_max = if self.wrap_width.is_some() {
            0.0
        } else {
            (self.max_chars as f32 * st.char_w + 40.0 - text_w).max(0.0)
        };
        self.h_off = self.h_off.min(self.h_max);

        let first = self.visual_offsets.partition_point(|&offset| offset <= range.start).saturating_sub(1);
        let end = self.visual_offsets.partition_point(|&offset| offset < range.end).min(self.items.len());
        for idx in first..end {
            let y = top + (self.visual_offsets[idx] as f32 - range.start as f32) * st.row_h;
            let height = (self.visual_offsets[idx + 1] - self.visual_offsets[idx]) as f32 * st.row_h;
            let r = Rect::from_min_size(pos2(max.left(), y), vec2(width, height));
            let item = self.items[idx];
            if let Item::Fold { start, end } = item {
                p.rect_filled(r, 0.0, pal.fold_bg);
                let text = if hovered == Some(idx) {
                    format!("⋯  {} unchanged lines — click to expand", end - start)
                } else {
                    format!("⋯  {} unchanged lines", end - start)
                };
                p.text(
                    pos2(r.left() + gw + 8.0, r.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    st.small.clone(),
                    if hovered == Some(idx) { pal.accent } else { pal.text_dim },
                );
                continue;
            }
            let row_idx = item.row();
            let in_current = current.is_some_and(|h| h.start <= row_idx && row_idx < h.end);
            if unified {
                self.paint_unified(&p, r, item, gw, st);
            } else {
                let half = width / 2.0;
                let lr = Rect::from_min_size(r.min, vec2(half, r.height()));
                let rr = Rect::from_min_size(pos2(r.left() + half, r.top()), vec2(width - half, r.height()));
                self.paint_side(&p, lr, row_idx, Side::Left, gw, st);
                self.paint_side(&p, rr, row_idx, Side::Right, gw, st);
                p.vline(r.left() + half, r.y_range(), Stroke::new(1.0, pal.border));
            }
            if in_current {
                p.rect_filled(
                    Rect::from_min_size(r.min, vec2(3.0, r.height())),
                    0.0,
                    pal.accent,
                );
            }
            if hovered == Some(idx) {
                p.rect_filled(r, 0.0, pal.hover);
            }
        }

        if resp.clicked()
            && let Some(h) = hovered
        {
            match self.items[h] {
                Item::Fold { start, .. } => {
                    self.expanded.insert(start);
                    self.built_for = None;
                }
                it => {
                    if let Some(hk) = self.diff.hunk_of_row(it.row()) {
                        self.current_hunk = Some(hk);
                    }
                }
            }
        }
        if hovered.is_some_and(|h| matches!(self.items[h], Item::Fold { .. })) {
            resp.clone().on_hover_cursor(CursorIcon::PointingHand);
        }
        if resp.secondary_clicked() {
            self.menu_item = hovered;
        }
        resp.context_menu(|ui| self.context_menu(ui));

        if resp.contains_pointer() {
            let dx = ui.input(|i| i.smooth_scroll_delta.x);
            if dx != 0.0 {
                self.h_off = (self.h_off - dx).clamp(0.0, self.h_max);
            }
        }
    }

    fn paint_side(&self, p: &egui::Painter, rect: Rect, row_idx: usize, side: Side, gw: f32, st: &CodeStyle) {
        let pal = st.pal;
        let row = &self.diff.rows[row_idx];
        let (line, emph, doc, hl) = match side {
            Side::Left => (row.left, &row.left_emph, &self.left, &self.left_hl),
            Side::Right => (row.right, &row.right_emph, &self.right, &self.right_hl),
        };
        let Some(line) = line else {
            paint_filler(p, rect, st);
            return;
        };
        let (bg, gutter_bg, emph_bg, num_color) = match (row.kind, side) {
            (RowKind::Equal, _) => (None, pal.gutter_bg, Color32::TRANSPARENT, pal.gutter_fg),
            (_, Side::Left) => (Some(pal.del_bg), pal.del_gutter, pal.del_emph, pal.del_fg),
            (_, Side::Right) => (Some(pal.add_bg), pal.add_gutter, pal.add_emph, pal.add_fg),
        };
        if let Some(bg) = bg {
            p.rect_filled(rect, 0.0, bg);
        }
        let g = Rect::from_min_size(rect.min, vec2(gw, rect.height()));
        p.rect_filled(g, 0.0, gutter_bg);
        paint_line_number(p, g.with_max_y(g.top() + st.row_h), Some(line + 1), num_color, st);
        let text_rect = Rect::from_min_max(pos2(g.right() + 8.0, rect.top()), rect.max);
        let job = line_job(doc.line(line), hl.line(line), emph, emph_bg, st);
        self.paint_text(p, text_rect, job, st);
    }

    fn paint_unified(&self, p: &egui::Painter, r: Rect, item: Item, gw: f32, st: &CodeStyle) {
        let pal = st.pal;
        let row = &self.diff.rows[item.row()];
        let g = Rect::from_min_size(r.min, vec2(2.0 * gw + st.char_w + 10.0, r.height()));
        let (ln, rn, sign, bg, gutter_bg, emph_bg, text) = match item {
            Item::Left(_) => (
                row.left,
                None,
                "−",
                Some(pal.del_bg),
                pal.del_gutter,
                pal.del_emph,
                Side::Left,
            ),
            Item::Right(_) => (
                None,
                row.right,
                "+",
                Some(pal.add_bg),
                pal.add_gutter,
                pal.add_emph,
                Side::Right,
            ),
            _ => (row.left, row.right, " ", None, pal.gutter_bg, Color32::TRANSPARENT, Side::Right),
        };
        if let Some(bg) = bg {
            p.rect_filled(r, 0.0, bg);
        }
        p.rect_filled(g, 0.0, gutter_bg);
        let num_color = match item {
            Item::Left(_) => pal.del_fg,
            Item::Right(_) => pal.add_fg,
            _ => pal.gutter_fg,
        };
        paint_line_number(p, Rect::from_min_size(g.min, vec2(gw, st.row_h)), ln.map(|l| l + 1), num_color, st);
        paint_line_number(
            p,
            Rect::from_min_size(pos2(g.left() + gw, g.top()), vec2(gw, st.row_h)),
            rn.map(|l| l + 1),
            num_color,
            st,
        );
        p.text(
            pos2(g.right() - st.char_w - 4.0, r.top() + st.row_h / 2.0),
            Align2::LEFT_CENTER,
            sign,
            st.font.clone(),
            num_color,
        );
        let (line, emph, doc, hl) = match text {
            Side::Left => (row.left, &row.left_emph, &self.left, &self.left_hl),
            Side::Right => (row.right, &row.right_emph, &self.right, &self.right_hl),
        };
        if let Some(line) = line {
            let text_rect = Rect::from_min_max(pos2(g.right() + 8.0, r.top()), r.max);
            let job = line_job(doc.line(line), hl.line(line), emph, emph_bg, st);
            self.paint_text(p, text_rect, job, st);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui) {
        let Some(item) = self.menu_item.and_then(|i| self.items.get(i)).copied() else {
            ui.close();
            return;
        };
        if matches!(item, Item::Fold { .. }) {
            ui.close();
            return;
        }
        let row = self.diff.rows[item.row()].clone();
        let mut copy = None;
        if let Some(l) = row.left
            && !matches!(item, Item::Right(_))
            && ui.button("Copy line from A").clicked()
        {
            copy = Some(self.left.line(l).to_owned());
        }
        if let Some(r) = row.right
            && !matches!(item, Item::Left(_))
            && ui.button("Copy line from B").clicked()
        {
            copy = Some(self.right.line(r).to_owned());
        }
        if let Some(h) = self.diff.hunk_of_row(item.row()) {
            let hunk = self.diff.hunks[h];
            ui.separator();
            let rows = &self.diff.rows[hunk.start..hunk.end];
            if ui.button("Copy change from A").clicked() {
                let lines: Vec<&str> = rows.iter().filter_map(|r| r.left).map(|l| self.left.line(l)).collect();
                copy = Some(lines.join("\n"));
            }
            if ui.button("Copy change from B").clicked() {
                let lines: Vec<&str> = rows.iter().filter_map(|r| r.right).map(|l| self.right.line(l)).collect();
                copy = Some(lines.join("\n"));
            }
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
            ui.close();
        }
    }

    /// Change overview on the right edge; doubles as a scrollbar.
    fn overview(&mut self, ui: &mut Ui, rect: Rect, content_h: f32, st: &CodeStyle) {
        let pal = st.pal;
        let row_h = st.row_h;
        let p = ui.painter();
        p.rect_filled(rect, 0.0, pal.gutter_bg);
        p.vline(rect.left(), rect.y_range(), Stroke::new(1.0, pal.border));
        // Map items onto the strip; short files only use the top part of it.
        let total = (*self.visual_offsets.last().unwrap() as f32).max(self.last_view_h / row_h).max(1.0);
        let h = rect.height();
        for &(s, e, m) in &self.markers {
            let y0 = rect.top() + self.visual_offsets[s] as f32 / total * h;
            let y1 = (rect.top() + self.visual_offsets[e] as f32 / total * h).max(y0 + 2.0);
            let sides = match m {
                Mark::Add => ChangeSides::B,
                Mark::Del => ChangeSides::A,
                Mark::Mod => ChangeSides::Both,
            };
            paint_change_marker(p, Rect::from_x_y_ranges(rect.left() + 3.0..=rect.right() - 2.0, y0..=y1), sides, pal);
        }
        if content_h > self.last_view_h {
            let t0 = rect.top() + self.last_offset / content_h * h;
            let t1 = rect.top() + (self.last_offset + self.last_view_h) / content_h * h;
            let thumb = Rect::from_x_y_ranges(rect.left() + 1.0..=rect.right(), t0..=t1.max(t0 + 8.0));
            p.rect_filled(thumb, 2.0, pal.hover);
            p.rect_stroke(thumb, 2.0, Stroke::new(1.0, pal.text_dim.gamma_multiply(0.5)), egui::StrokeKind::Inside);
        }
        let resp = ui
            .interact(rect, ui.id().with(("overview", self.id)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand);
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let frac = ((pos.y - rect.top()) / h).clamp(0.0, 1.0);
            self.pending_offset = Some(frac * content_h - self.last_view_h / 2.0);
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_lines() -> FileView {
        let mut view = FileView::new(None, None, None, DiffOptions::default());
        let left = format!("{}\nshared\nold\n", "\tlet café = 123; ".repeat(30));
        let right = format!("{}\nshared\nnew\n", "let café = 456; ".repeat(12));
        view.left = TextDoc::from_bytes(left.as_bytes());
        view.right = TextDoc::from_bytes(right.as_bytes());
        view.left_hl = highlight(&view.left, "test.rs");
        view.right_hl = highlight(&view.right, "test.rs");
        view.max_chars = view.left.max_line_chars().max(view.right.max_line_chars());
        view.recompute();
        view
    }

    #[test]
    fn wrapping_aligns_pairs_and_reflows_with_layout_width_and_font() {
        let ctx = egui::Context::default();
        let mut view = long_lines();
        let mut settings = ViewSettings::default();
        assert!(!settings.wrap);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let st = CodeStyle::new(ui.ctx(), settings.font_size);
            view.rebuild_items(&settings);
            view.layout_rows(ui, &st, 600.0, false, false);
            assert_eq!(view.visual_offsets, (0..=view.items.len()).collect::<Vec<_>>());
            view.layout_rows(ui, &st, 600.0, false, true);
            let narrow = view.visual_offsets[1];
            assert!(narrow > 1);
            let row = &view.diff.rows[0];
            let job = view.wrap_job(line_job(view.left.line(row.left.unwrap()), view.left_hl.line(0),
                &row.left_emph, st.pal.del_emph, &st), &st);
            let galley = ui.painter().layout_job(job);
            assert_eq!(narrow, galley.rows.len());
            assert!(galley.size().y <= narrow as f32 * st.row_h + 0.1);
            assert_eq!(view.visual_offsets[2] - view.visual_offsets[1], 1);
            view.h_off = 100.0;
            view.layout_rows(ui, &st, 1000.0, false, true);
            assert!(view.visual_offsets[1] < narrow);
            assert_eq!(view.h_off, 0.0);
            let wider = view.visual_offsets[1];
            let large = CodeStyle::new(ui.ctx(), 20.0);
            view.layout_rows(ui, &large, 1000.0, false, true);
            assert!(view.visual_offsets[1] > wider);
            settings.unified = true;
            view.rebuild_items(&settings);
            view.layout_rows(ui, &st, 600.0, true, true);
            assert!(matches!(view.items[0], Item::Left(_)));
            assert!(matches!(view.items[1], Item::Right(_)));
            assert!(view.visual_offsets[1] > 1);
            assert!(view.visual_offsets[2] - view.visual_offsets[1] > 1);
            view.layout_rows(ui, &st, 600.0, true, false);
            assert_eq!(view.visual_offsets, (0..=view.items.len()).collect::<Vec<_>>());
        });
        output.textures_delta.clear();
    }

    #[test]
    fn wrapped_navigation_and_scrolling_use_visual_rows() {
        let ctx = egui::Context::default();
        let mut view = long_lines();
        let mut settings = ViewSettings { wrap: true, ..Default::default() };
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 180.0))),
            ..Default::default()
        }, |ui| {
            view.ui(ui, &mut settings, false);
            assert!(view.h_max == 0.0);
            assert!(view.last_view_h > 0.0);
        });
        output.textures_delta.clear();
        let last = view.diff.hunks.len() - 1;
        view.goto_hunk(last);
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 180.0))),
            ..Default::default()
        }, |ui| { view.ui(ui, &mut settings, false); });
        output.textures_delta.clear();
        assert!(view.last_offset > 0.0);
        assert_eq!(view.current_hunk, Some(last));
    }
}
