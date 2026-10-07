//! Folder comparison: a tree of differences on the left, the file diff on the right.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Key, Margin, Modifiers, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, pos2, vec2,
};

use super::file_view::{FileAction, FileView};
use super::{ViewSettings, keys_free};
use crate::folder::{Counts, Node, Progress, Status, compare_roots};
use crate::source::Entry;
use crate::theme::Palette;

enum Load {
    Loading {
        rx: std::sync::mpsc::Receiver<Node>,
        progress: Arc<Progress>,
    },
    Ready {
        root: Node,
        counts: HashMap<String, Counts>,
    },
}

#[derive(Clone, Copy)]
struct Filter {
    changed: bool,
    left_only: bool,
    right_only: bool,
    same: bool,
}

impl Filter {
    fn allows(&self, s: Status) -> bool {
        match s {
            Status::Same => self.same,
            Status::Changed | Status::Error => self.changed,
            Status::LeftOnly => self.left_only,
            Status::RightOnly => self.right_only,
        }
    }
}

pub struct FolderView {
    pub left: Entry,
    pub right: Entry,
    load: Load,
    expanded: HashSet<String>,
    selected: Option<String>,
    file_view: Option<FileView>,
    filter: Filter,
    search: String,
    sidebar_open: bool,
    scroll_to_selected: bool,
}

struct FlatRow<'a> {
    node: &'a Node,
    depth: usize,
}

fn status_style(s: Status, pal: &Palette) -> (&'static str, Color32) {
    match s {
        Status::Same => ("", pal.text_dim),
        Status::Changed => ("M", pal.mod_fg),
        Status::LeftOnly => ("A", pal.del_fg),
        Status::RightOnly => ("B", pal.add_fg),
        Status::Error => ("!", pal.del_fg),
    }
}

fn collect_counts(node: &Node, out: &mut HashMap<String, Counts>) -> Counts {
    if !node.is_dir {
        let mut c = Counts::default();
        match node.status {
            Status::Same => c.same = 1,
            Status::Changed | Status::Error => c.changed = 1,
            Status::LeftOnly => c.left_only = 1,
            Status::RightOnly => c.right_only = 1,
        }
        return c;
    }
    let mut c = Counts::default();
    for child in &node.children {
        let cc = collect_counts(child, out);
        c.same += cc.same;
        c.changed += cc.changed;
        c.left_only += cc.left_only;
        c.right_only += cc.right_only;
    }
    out.insert(node.rel.clone(), c);
    c
}

impl FolderView {
    pub fn new(left: Entry, right: Entry) -> Self {
        let mut view = Self {
            load: Self::start(&left, &right),
            left,
            right,
            expanded: HashSet::new(),
            selected: None,
            file_view: None,
            filter: Filter {
                changed: true,
                left_only: true,
                right_only: true,
                same: true,
            },
            search: String::new(),
            sidebar_open: true,
            scroll_to_selected: false,
        };
        if matches!(view.load, Load::Ready { .. }) {
            view.on_ready();
        }
        view
    }
    fn start(left: &Entry, right: &Entry) -> Load {
        let (tx, rx) = std::sync::mpsc::channel();
        let progress = Arc::new(Progress::default());
        let (l, r, p) = (left.clone(), right.clone(), progress.clone());
        std::thread::spawn(move || {
            let _ = tx.send(compare_roots(&l, &r, &p));
        });
        Load::Loading { rx, progress }
    }

    fn ready(root: Node) -> Load {
        let mut counts = HashMap::new();
        collect_counts(&root, &mut counts);
        Load::Ready { root, counts }
    }

    pub fn reload(&mut self) {
        self.load = Self::start(&self.left, &self.right);
        self.file_view = None;
        if matches!(self.load, Load::Ready { .. }) {
            self.on_ready();
        }
    }

    fn poll(&mut self) {
        if let Load::Loading { rx, .. } = &self.load
            && let Ok(root) = rx.try_recv()
        {
            self.load = Self::ready(root);
            self.on_ready();
        }
    }

    /// Expand changed folders and select a file once the comparison is done.
    fn on_ready(&mut self) {
        {
            if let Load::Ready { root, .. } = &self.load {
                // Expand folders that contain differences.
                let mut exp = HashSet::new();
                fn walk(n: &Node, exp: &mut HashSet<String>) {
                    for c in &n.children {
                        if c.is_dir && c.status != Status::Same {
                            exp.insert(c.rel.clone());
                            walk(c, exp);
                        }
                    }
                }
                walk(root, &mut exp);
                self.expanded.extend(exp);
            }
            match self.selected.clone() {
                Some(sel) => self.select(&sel),
                None => {
                    // Prefer the first modified file over added / removed ones.
                    let first = self.root().and_then(|root| {
                        let mut found = None;
                        root.walk_files(&mut |n| {
                            if found.is_none() && n.status == Status::Changed {
                                found = Some(n.rel.clone());
                            }
                        });
                        found
                    });
                    if let Some(first) = first.or_else(|| self.ordered_files().first().cloned()) {
                        self.select(&first);
                    }
                }
            }
        }
    }

    pub fn title(&self) -> String {
        format!("{} ↔ {}", self.left.name(), self.right.name())
    }

    fn root(&self) -> Option<&Node> {
        match &self.load {
            Load::Ready { root, .. } => Some(root),
            Load::Loading { .. } => None,
        }
    }

    fn matches_search(&self, n: &Node) -> bool {
        self.search.is_empty() || n.rel.to_lowercase().contains(&self.search.to_lowercase())
    }

    fn visible(&self, n: &Node) -> bool {
        if n.is_dir {
            let own = matches!(n.status, Status::LeftOnly | Status::RightOnly)
                && self.filter.allows(n.status)
                && self.matches_search(n);
            own || n.children.iter().any(|c| self.visible(c))
        } else {
            self.filter.allows(n.status) && self.matches_search(n)
        }
    }

    fn flatten<'a>(&self, node: &'a Node, depth: usize, out: &mut Vec<FlatRow<'a>>) {
        for c in &node.children {
            if !self.visible(c) {
                continue;
            }
            out.push(FlatRow { node: c, depth });
            if c.is_dir && (self.expanded.contains(&c.rel) || !self.search.is_empty()) {
                self.flatten(c, depth + 1, out);
            }
        }
    }

    /// Changed files in tree order (for next/previous file navigation).
    fn ordered_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(root) = self.root() {
            root.walk_files(&mut |n| {
                if n.status != Status::Same && self.filter.allows(n.status) && self.matches_search(n) {
                    out.push(n.rel.clone());
                }
            });
        }
        out
    }

    fn select(&mut self, rel: &str) {
        let Some(node) = self.root().and_then(|r| r.find(rel)) else {
            return;
        };
        if node.is_dir {
            return;
        }
        let labels = (
            format!("{}/{}", self.left.name(), node.rel),
            format!("{}/{}", self.right.name(), node.rel),
        );
        let fv = FileView::new(
            node.left.clone(),
            node.right.clone(),
            Some(labels),
            crate::diff::DiffOptions::default(),
        );
        // Expand all ancestors so the selection is visible.
        let mut path = String::new();
        for part in rel.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            self.expanded.insert(path.clone());
        }
        self.selected = Some(rel.to_owned());
        self.file_view = Some(fv);
        self.scroll_to_selected = true;
    }

    fn step_file(&mut self, delta: isize) {
        let files = self.ordered_files();
        if files.is_empty() {
            return;
        }
        let pos = self
            .selected
            .as_ref()
            .and_then(|s| files.iter().position(|f| f == s));
        let next = match pos {
            Some(p) => (p as isize + delta).clamp(0, files.len() as isize - 1) as usize,
            None => 0,
        };
        if pos != Some(next) {
            let f = files[next].clone();
            self.select(&f);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut ViewSettings) {
        self.poll();
        let pal = crate::theme::palette(ui.ctx());

        if keys_free(ui.ctx()) {
            let (next, prev, sidebar) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::ArrowDown),
                    i.consume_key(Modifiers::COMMAND, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::B),
                )
            });
            if next {
                self.step_file(1);
            }
            if prev {
                self.step_file(-1);
            }
            if sidebar {
                self.sidebar_open = !self.sidebar_open;
            }
        }

        let panel = egui::Panel::left("folder-sidebar")
            .resizable(true)
            .default_size(300.0)
            .size_range(200.0..=700.0)
            .frame(egui::Frame::new().fill(pal.panel).inner_margin(Margin::ZERO));
        if self.sidebar_open {
            panel.show(ui, |ui| self.sidebar(ui, pal));
        }

        egui::CentralPanel::no_frame().show(ui, |ui| {
            if let Load::Loading { progress, .. } = &self.load {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                ui.add_space(ui.available_height() / 3.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!(
                            "Comparing folders… {} files",
                            progress.files.load(Ordering::Relaxed)
                        ))
                        .color(pal.text_dim),
                    );
                });
                return;
            }
            let mut action = FileAction::None;
            if let Some(fv) = &mut self.file_view {
                action = fv.ui(ui, settings, true);
            } else {
                self.summary(ui, pal);
            }
            match action {
                FileAction::NextFile => self.step_file(1),
                FileAction::PrevFile => self.step_file(-1),
                FileAction::None => {}
            }
        });
    }

    fn summary(&self, ui: &mut Ui, pal: &Palette) {
        let Load::Ready { root, counts } = &self.load else { return };
        let c = counts.get(&root.rel).copied().unwrap_or_default();
        ui.add_space(ui.available_height() / 4.0);
        ui.vertical_centered(|ui| {
            if c.changed + c.left_only + c.right_only == 0 {
                ui.label(RichText::new("✔").size(44.0).color(pal.add_fg));
                ui.label(RichText::new("Folders are identical").size(20.0).strong());
                ui.label(RichText::new(format!("{} files compared", c.same)).color(pal.text_dim));
            } else {
                ui.label(RichText::new("Select a file to see its changes").size(18.0).strong());
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{} changed · {} only in A · {} only in B · {} identical",
                        c.changed, c.left_only, c.right_only, c.same
                    ))
                    .color(pal.text_dim),
                );
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{}↓ / {}↑ steps through changed files", super::cmd(), super::cmd())).color(pal.text_dim).small());
            }
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, pal: &'static Palette) {
        let header = |ui: &mut Ui, tag: &str, color: Color32, e: &Entry| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tag).color(Color32::WHITE).background_color(color).monospace().size(11.0));
                ui.add(egui::Label::new(RichText::new(e.display_path()).size(12.0)).truncate())
                    .on_hover_text(e.display_path());
            });
        };
        egui::Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            header(ui, " A ", pal.del_fg, &self.left);
            header(ui, " B ", pal.add_fg, &self.right);
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("🔍 Filter files…")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(2.0);
            let c = match &self.load {
                Load::Ready { root, counts } => counts.get(&root.rel).copied().unwrap_or_default(),
                Load::Loading { .. } => Counts::default(),
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                chip(ui, &mut self.filter.changed, &format!("M {}", c.changed), pal.mod_fg, "Changed files");
                chip(ui, &mut self.filter.left_only, &format!("A {}", c.left_only), pal.del_fg, "Only in A");
                chip(ui, &mut self.filter.right_only, &format!("B {}", c.right_only), pal.add_fg, "Only in B");
                chip(ui, &mut self.filter.same, &format!("= {}", c.same), pal.text_dim, "Identical files");
            });
        });
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            Stroke::new(1.0, pal.border),
        );

        let Some(root) = self.root() else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        let mut rows = Vec::new();
        self.flatten(root, 0, &mut rows);
        let row_h = 24.0;
        let mut clicked: Option<(String, bool)> = None;
        let selected = self.selected.clone();
        let scroll_to = if self.scroll_to_selected {
            selected.as_ref().and_then(|s| rows.iter().position(|r| &r.node.rel == s))
        } else {
            None
        };
        let counts = match &self.load {
            Load::Ready { counts, .. } => counts,
            Load::Loading { .. } => unreachable!(),
        };

        let mut sa = ScrollArea::vertical().id_salt("folder-tree").auto_shrink(false);
        if let Some(i) = scroll_to {
            let view_h = ui.available_height();
            sa = sa.vertical_scroll_offset((i as f32 * row_h - view_h / 3.0).max(0.0));
        }
        if rows.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("No matching files").color(pal.text_dim)));
        }
        sa.show_rows(ui, row_h, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows[range] {
                let n = row.node;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let p = ui.painter();
                let is_sel = selected.as_deref() == Some(n.rel.as_str());
                if is_sel {
                    p.rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(5), pal.accent_bg);
                } else if resp.hovered() {
                    p.rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(5), pal.hover);
                }
                let mut x = rect.left() + 12.0 + row.depth as f32 * 14.0;
                let cy = rect.center().y;
                if n.is_dir {
                    let open = self.expanded.contains(&n.rel) || !self.search.is_empty();
                    p.text(
                        pos2(x, cy),
                        Align2::LEFT_CENTER,
                        if open { "⏷" } else { "⏵" },
                        FontId::proportional(10.0),
                        pal.text_dim,
                    );
                }
                x += 14.0;
                let (badge, color) = status_style(n.status, pal);
                let icon = if n.is_dir { "🗀" } else { "🗋" };
                p.text(pos2(x, cy), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), if n.status == Status::Same { pal.text_dim } else { color });
                x += 20.0;
                let name_color = match n.status {
                    Status::Same => pal.text_dim,
                    Status::Changed => pal.text,
                    _ => color,
                };
                let right_reserved = 44.0;
                let clip = p.with_clip_rect(Rect::from_x_y_ranges(x..=rect.right() - right_reserved, rect.y_range()));
                clip.text(pos2(x, cy), Align2::LEFT_CENTER, &n.name, FontId::proportional(13.0), name_color);
                // Right side: status badge or change count for folders.
                if n.is_dir {
                    if let Some(c) = counts.get(&n.rel) {
                        let k = c.changed + c.left_only + c.right_only;
                        if k > 0 {
                            p.text(
                                pos2(rect.right() - 14.0, cy),
                                Align2::RIGHT_CENTER,
                                k.to_string(),
                                FontId::proportional(11.0),
                                pal.text_dim,
                            );
                        }
                    }
                } else if !badge.is_empty() {
                    let b = Rect::from_center_size(pos2(rect.right() - 22.0, cy), vec2(16.0, 15.0));
                    p.rect_filled(b, 3.0, color.gamma_multiply(0.2));
                    p.text(b.center(), Align2::CENTER_CENTER, badge, FontId::monospace(10.0), color);
                }
                if resp.clicked() {
                    clicked = Some((n.rel.clone(), n.is_dir));
                }
                if let Some(e) = &n.error {
                    resp.on_hover_text(e);
                } else {
                    resp.on_hover_cursor(CursorIcon::PointingHand);
                }
            }
        });
        self.scroll_to_selected = false;

        if let Some((rel, is_dir)) = clicked {
            if is_dir {
                if !self.expanded.remove(&rel) {
                    self.expanded.insert(rel);
                }
            } else {
                self.select(&rel);
                self.scroll_to_selected = false;
            }
        }
    }
}

fn chip(ui: &mut Ui, on: &mut bool, text: &str, color: Color32, tip: &str) {
    let pal = crate::theme::palette(ui.ctx());
    let rt = RichText::new(text).size(11.5).color(if *on { color } else { pal.text_dim });
    let b = egui::Button::new(rt)
        .fill(if *on { color.gamma_multiply(0.15) } else { Color32::TRANSPARENT })
        .stroke(Stroke::new(1.0, if *on { color.gamma_multiply(0.6) } else { pal.border }))
        .corner_radius(CornerRadius::same(10));
    if ui.add(b).on_hover_text(tip).clicked() {
        *on = !*on;
    }
}
