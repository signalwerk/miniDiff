//! Top-level application state and routing between screens.

use std::path::PathBuf;

use egui::{
    Align, Align2, FontId, Id, Key, LayerId, Layout, Margin, Modifiers, Order, RichText, Sense, Stroke,
    Ui, ViewportCommand, vec2,
};

use crate::merge::{MergeDoc, has_conflict_markers};
use crate::platform::{self, Incoming};
use crate::source::Entry;
use crate::text::TextDoc;
use crate::theme::{self, ThemeChoice};
use crate::ui::file_view::FileView;
use crate::ui::folder_view::FolderView;
use crate::ui::merge_view::{MergeAction, MergeView};
use crate::ui::welcome::{self, WelcomeAction};
use crate::ui::{ViewSettings, cmd, keys_free};

/// What to show on startup (from the command line).
pub enum Launch {
    Welcome,
    /// One or two dropped / passed paths, routed like a drop.
    Open(Vec<Entry>),
    Compare {
        left: Entry,
        right: Entry,
        labels: Option<(String, String)>,
    },
    Merge {
        base: Option<Entry>,
        local: Entry,
        remote: Entry,
        output: Option<PathBuf>,
        labels: Option<[String; 3]>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    theme: ThemeChoice,
    view: ViewSettings,
    recent: Vec<(String, String)>,
    /// Check minidiff.signalwerk.ch for a newer release on startup.
    auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            view: ViewSettings::default(),
            recent: Vec::new(),
            auto_update: true,
        }
    }
}

enum Screen {
    Welcome,
    File(Box<FileView>),
    Folder(Box<FolderView>),
    Merge(Box<MergeView>),
}

pub struct MiniDiffApp {
    settings: Settings,
    screen: Screen,
    slots: [Option<Entry>; 2],
    toast: Option<(String, bool, f64)>,
    close_dialog: bool,
    allow_close: bool,
    title: String,
    updater: crate::update::Updater,
    /// The user asked for an update check, so report "up to date" and errors.
    manual_check: bool,
}

impl MiniDiffApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        platform::set_context(&cc.egui_ctx);
        install_fonts(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.theme.apply(&cc.egui_ctx);

        let mut app = Self {
            settings,
            screen: Screen::Welcome,
            slots: [None, None],
            toast: None,
            close_dialog: false,
            allow_close: false,
            title: String::new(),
            updater: crate::update::Updater::new(),
            manual_check: false,
        };
        // Don't distract (or restart) while running as git / Tower merge tool.
        if app.settings.auto_update && !matches!(launch, Launch::Merge { .. }) {
            app.updater.check(&cc.egui_ctx);
        }
        match launch {
            Launch::Welcome => {}
            Launch::Open(entries) => app.open_incoming(Incoming { entries, slot: None }),
            Launch::Compare { left, right, labels } => app.compare(left, right, labels),
            Launch::Merge {
                base,
                local,
                remote,
                output,
                labels,
            } => app.open_merge(base, local, remote, output, labels, true),
        }
        app
    }

    fn notify(&mut self, msg: impl Into<String>, error: bool) {
        self.toast = Some((msg.into(), error, f64::NAN));
    }

    // ------------------------------------------------------------------
    // Routing

    fn open_incoming(&mut self, inc: Incoming) {
        let Incoming { mut entries, slot } = inc;
        if let Some(s) = slot {
            if let Some(e) = entries.drain(..).next() {
                self.slots[s.min(1)] = Some(e);
                self.screen = Screen::Welcome;
                self.compare_slots_if_ready();
            }
            return;
        }
        match entries.len() {
            0 => {}
            1 => {
                let e = entries.remove(0);
                if !e.is_dir() && self.try_open_conflicted(&e) {
                    return;
                }
                if self.slots[0].is_some() && self.slots[1].is_some() {
                    self.slots = [None, None];
                }
                let i = if self.slots[0].is_none() { 0 } else { 1 };
                self.slots[i] = Some(e);
                self.screen = Screen::Welcome;
                self.compare_slots_if_ready();
            }
            2 => {
                let b = entries.remove(1);
                let a = entries.remove(0);
                self.compare(a, b, None);
            }
            n => self.notify(format!("Drop one or two items (got {n})."), true),
        }
    }

    fn compare_slots_if_ready(&mut self) {
        if let [Some(a), Some(b)] = &self.slots {
            let (a, b) = (a.clone(), b.clone());
            self.compare(a, b, None);
        }
    }

    fn try_open_conflicted(&mut self, e: &Entry) -> bool {
        let Ok(bytes) = e.read() else { return false };
        let doc = TextDoc::from_bytes(&bytes);
        if doc.binary || !has_conflict_markers(&doc) {
            return false;
        }
        let Some(merge) = MergeDoc::from_conflict_markers(&doc) else {
            return false;
        };
        let view = MergeView::new(
            merge,
            e.name(),
            e.fs_path().map(PathBuf::from),
            ["Local (ours)".into(), "Base".into(), "Remote (theirs)".into()],
            false,
        );
        self.screen = Screen::Merge(Box::new(view));
        true
    }

    fn compare(&mut self, a: Entry, b: Entry, labels: Option<(String, String)>) {
        if a.is_dir() != b.is_dir() {
            self.notify("Can't compare a file with a folder.", true);
            self.slots = [Some(a), None];
            self.screen = Screen::Welcome;
            return;
        }
        if let (Some(pa), Some(pb)) = (a.fs_path(), b.fs_path()) {
            let pair = (pa.display().to_string(), pb.display().to_string());
            self.settings.recent.retain(|r| *r != pair);
            self.settings.recent.insert(0, pair);
            self.settings.recent.truncate(12);
        }
        self.slots = [Some(a.clone()), Some(b.clone())];
        self.screen = if a.is_dir() {
            Screen::Folder(Box::new(FolderView::new(a, b)))
        } else {
            Screen::File(Box::new(FileView::new(Some(a), Some(b), labels, self.settings.view.diff)))
        };
    }

    fn open_merge(
        &mut self,
        base: Option<Entry>,
        local: Entry,
        remote: Entry,
        output: Option<PathBuf>,
        labels: Option<[String; 3]>,
        tool_mode: bool,
    ) {
        let read = |e: &Entry| e.read().map(|b| TextDoc::from_bytes(&b));
        let (l, r) = match (read(&local), read(&remote)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(e), _) | (_, Err(e)) => {
                self.notify(format!("Could not read merge input: {e}"), true);
                return;
            }
        };
        let doc = match base.as_ref().map(read) {
            Some(Ok(b)) => MergeDoc::three_way(&b, &l, &r),
            _ => {
                // No usable base: maybe the output already holds conflict markers.
                let from_output = output
                    .as_ref()
                    .and_then(|p| std::fs::read(p).ok())
                    .map(|b| TextDoc::from_bytes(&b))
                    .and_then(|d| MergeDoc::from_conflict_markers(&d));
                from_output.unwrap_or_else(|| MergeDoc::two_way(&l, &r))
            }
        };
        let file_name = output
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| local.name());
        let labels = labels.unwrap_or_else(|| {
            [
                format!("Local · {}", local.name()),
                base.as_ref().map_or("Base".into(), |b| format!("Base · {}", b.name())),
                format!("Remote · {}", remote.name()),
            ]
        });
        if tool_mode {
            // Until the user saves, report failure so git keeps the conflict.
            platform::set_exit_code(1);
        }
        self.screen = Screen::Merge(Box::new(MergeView::new(doc, file_name, output, labels, tool_mode)));
    }

    fn open_demo(&mut self, merge: bool) {
        if merge {
            let (base, local, remote) = crate::demo::merge();
            let labels = ["Local · local.rs".into(), "Base · base.rs".into(), "Remote · remote.rs".into()];
            self.open_merge(Some(base), local, remote, None, Some(labels), false);
            if let Screen::Merge(m) = &mut self.screen {
                m.file_name = "merged.rs".into();
            }
        } else {
            let (a, b) = crate::demo::folders();
            self.compare(a, b, None);
        }
    }

    fn go_home(&mut self) {
        if let Screen::Merge(m) = &self.screen
            && m.unsaved
        {
            self.close_dialog = true;
            return;
        }
        self.screen = Screen::Welcome;
    }

    fn swap(&mut self) {
        match &self.screen {
            Screen::File(fv) => {
                let (a, b) = fv.entries();
                let labels = Some((fv.right_label.clone(), fv.left_label.clone()));
                self.slots = [b.clone(), a.clone()];
                self.screen = Screen::File(Box::new(FileView::new(b, a, labels, self.settings.view.diff)));
            }
            Screen::Folder(f) => {
                let (a, b) = (f.left.clone(), f.right.clone());
                self.slots = [Some(b.clone()), Some(a.clone())];
                self.screen = Screen::Folder(Box::new(FolderView::new(b, a)));
            }
            _ => {}
        }
    }

    fn reload(&mut self) {
        match &mut self.screen {
            Screen::File(fv) => {
                let (a, b) = fv.entries();
                let labels = Some((fv.left_label.clone(), fv.right_label.clone()));
                **fv = FileView::new(a, b, labels, self.settings.view.diff);
            }
            Screen::Folder(f) => f.reload(),
            _ => {}
        }
    }
    fn pick(&mut self, folder: bool, slot: usize) {
        let dialog = rfd::FileDialog::new().set_title(if slot == 0 { "Choose A" } else { "Choose B" });
        let picked = if folder { dialog.pick_folder() } else { dialog.pick_file() };
        if let Some(p) = picked {
            self.open_incoming(Incoming {
                entries: vec![Entry::Fs(p)],
                slot: Some(slot),
            });
        }
    }

    // ------------------------------------------------------------------
    // UI

    fn global_keys(&mut self, ctx: &egui::Context) {
        let (home, reload, swap, theme_key) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::L),
            )
        });
        if home {
            self.go_home();
        }
        if reload {
            self.reload();
        }
        if swap {
            self.swap();
        }
        if theme_key {
            self.settings.theme = self.settings.theme.next();
            self.settings.theme.apply(ctx);
        }
        if keys_free(ctx) && matches!(self.screen, Screen::File(_) | Screen::Folder(_)) {
            let (bigger, smaller) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals),
                    i.consume_key(Modifiers::NONE, Key::Minus),
                )
            });
            if bigger {
                self.settings.view.font_size = (self.settings.view.font_size + 1.0).min(28.0);
            }
            if smaller {
                self.settings.view.font_size = (self.settings.view.font_size - 1.0).max(8.0);
            }
        }
    }

    fn app_bar(&mut self, ui: &mut Ui) {
        let pal = theme::palette(ui.ctx());
        let frame = egui::Frame::new()
            .fill(pal.panel)
            .inner_margin(Margin::symmetric(10, 6))
            .stroke(Stroke::NONE);
        egui::Panel::top("app-bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (logo, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                welcome::paint_logo(ui.painter(), logo.center(), 22.0, pal);
                let name = ui.add(
                    egui::Label::new(RichText::new("MiniDiff").strong().size(14.0)).sense(Sense::click()),
                );
                if resp.clicked() || name.clicked() {
                    self.go_home();
                }
                resp.on_hover_text(format!("Start screen  ({}O)", cmd()));

                let crumb = match &self.screen {
                    Screen::Welcome => None,
                    Screen::File(fv) => Some(("File", short(&fv.left_label), short(&fv.right_label))),
                    Screen::Folder(f) => Some(("Folder", f.left.name(), f.right.name())),
                    Screen::Merge(m) => Some(("Merge", m.file_name.clone(), String::new())),
                };
                if let Some((kind, a, b)) = crumb {
                    ui.label(RichText::new("›").color(pal.text_dim));
                    ui.label(RichText::new(kind).color(pal.text_dim));
                    ui.label(RichText::new(a).strong());
                    if !b.is_empty() {
                        ui.label(RichText::new("⇄").color(pal.text_dim));
                        ui.label(RichText::new(b).strong());
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button(RichText::new("?").size(14.0), |ui| self.help_menu(ui));
                    self.update_ui(ui);
                    let t = self.settings.theme;
                    if ui
                        .button(t.label())
                        .on_hover_text(format!("Theme: auto / light / dark  ({}⇧L)", cmd()))
                        .clicked()
                    {
                        self.settings.theme = t.next();
                        self.settings.theme.apply(ui.ctx());
                    }
                    if matches!(self.screen, Screen::File(_) | Screen::Folder(_)) {
                        if ui.button("⟳").on_hover_text(format!("Reload  ({}R)", cmd())).clicked() {
                            self.reload();
                        }
                        if ui.button("⇄ Swap").on_hover_text(format!("Swap A and B  ({}⇧S)", cmd())).clicked() {
                            self.swap();
                        }
                    }
                });
            });
        });
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        let pal = theme::palette(ctx);
        let rect = ctx.content_rect();
        let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("drop-overlay")));
        p.rect_filled(rect, 0.0, pal.bg.gamma_multiply(0.85));
        let inner = rect.shrink(18.0);
        p.rect_stroke(inner, 16.0, Stroke::new(2.5, pal.accent), egui::StrokeKind::Inside);
        let msg = match (&self.slots, &self.screen) {
            ([Some(_), None], Screen::Welcome) => "Drop to set B",
            _ => "Drop two files or folders to compare",
        };
        p.text(inner.center(), Align2::CENTER_CENTER, "⇣", FontId::proportional(56.0), pal.accent);
        p.text(inner.center() + vec2(0.0, 50.0), Align2::CENTER_CENTER, msg, FontId::proportional(20.0), pal.text);
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some((msg, err, since)) = &mut self.toast else { return };
        if since.is_nan() {
            *since = now;
        }
        if now - *since > 6.0 {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let pal = theme::palette(ctx);
        let (msg, err) = (msg.clone(), *err);
        egui::Area::new(Id::new("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(if err { pal.del_bg } else { pal.toolbar })
                    .stroke(Stroke::new(1.0, if err { pal.del_fg } else { pal.border }))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(if err { pal.del_fg } else { pal.text }));
                    });
            });
    }

    fn close_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.close_dialog {
            return;
        }
        let mut choice = None;
        let modal = egui::Modal::new(Id::new("unsaved-merge")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Unsaved merge");
            ui.label("The merge result has changes that were not saved.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save").strong()).clicked() {
                    choice = Some(0);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(2);
                }
            });
        });
        if modal.should_close() {
            choice = Some(2);
        }
        match choice {
            Some(0) => {
                if let Screen::Merge(m) = &mut self.screen
                    && m.save(true)
                {
                    self.close_dialog = false;
                    self.finish_merge(ctx);
                }
            }
            Some(1) => {
                self.close_dialog = false;
                if let Screen::Merge(m) = &mut self.screen {
                    m.unsaved = false;
                }
                self.finish_merge(ctx);
            }
            Some(_) => self.close_dialog = false,
            None => {}
        }
    }

    /// Leave the merge screen: quit if we were launched as a merge tool.
    fn finish_merge(&mut self, ctx: &egui::Context) {
        let tool = matches!(&self.screen, Screen::Merge(m) if m.tool_mode);
        if tool {
            self.update_exit_code();
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else {
            self.screen = Screen::Welcome;
        }
    }

    fn update_exit_code(&self) {
        if let Screen::Merge(m) = &self.screen
            && m.tool_mode
        {
            platform::set_exit_code(if m.saved && !m.unsaved { 0 } else { 1 });
        }
    }

    fn window_title(&self) -> String {
        match &self.screen {
            Screen::Welcome => "MiniDiff".into(),
            Screen::File(fv) => format!("{} ⇄ {} — MiniDiff", short(&fv.left_label), short(&fv.right_label)),
            Screen::Folder(f) => format!("{} — MiniDiff", f.title()),
            Screen::Merge(m) => format!("{}Merge {} — MiniDiff", if m.unsaved { "● " } else { "" }, m.file_name),
        }
    }
}

fn short(label: &str) -> String {
    label.rsplit(['/', '\\']).next().unwrap_or(label).to_owned()
}

impl MiniDiffApp {
    fn help_menu(&mut self, ui: &mut Ui) {
        help_shortcuts(ui);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("MiniDiff {}", env!("CARGO_PKG_VERSION"))).weak());
            if ui.button("Check for updates").clicked() {
                self.manual_check = true;
                self.updater.check(ui.ctx());
                ui.close();
            }
        });
        ui.checkbox(&mut self.settings.auto_update, "Check for updates on startup");
    }

    /// Update pill in the app bar (native only).
    fn update_ui(&mut self, ui: &mut Ui) {
        use crate::update::{State, Updater};
        let pal = theme::palette(ui.ctx());
        match self.updater.state() {
            State::Available(m) => {
                let b = egui::Button::new(
                    RichText::new(format!("⬆ Update to {}", m.version)).color(egui::Color32::WHITE),
                )
                .fill(pal.accent);
                let can_install = Updater::can_self_install(&m);
                let tip = if can_install {
                    "Download, install and restart MiniDiff"
                } else {
                    "Open the release page"
                };
                if ui.add(b).on_hover_text(tip).clicked() {
                    if can_install {
                        self.updater.install(ui.ctx(), m);
                    } else if let Some(url) = m.notes_url {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    }
                }
            }
            State::Installing(m) => {
                ui.label(RichText::new(format!("Installing {}…", m.version)).color(pal.text_dim));
                ui.spinner();
            }
            State::Installed(bundle) => {
                let b = egui::Button::new(RichText::new("⟳ Restart to finish update").color(egui::Color32::WHITE))
                    .fill(pal.add_fg);
                if ui.add(b).clicked() {
                    crate::update::relaunch(&bundle);
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            }
            State::UpToDate if self.manual_check => {
                self.manual_check = false;
                self.notify(format!("MiniDiff {} is up to date.", env!("CARGO_PKG_VERSION")), false);
            }
            State::Failed(e) if self.manual_check => {
                self.manual_check = false;
                self.notify(e, true);
            }
            State::Checking if self.manual_check => {
                ui.spinner();
            }
            _ => {}
        }
    }
}

fn help_shortcuts(ui: &mut Ui) {
    let c = cmd();
    let rows: &[(&str, String)] = &[
        ("Next / previous change", format!("n · p   ({a}↓ · {a}↑)", a = crate::ui::alt())),
        ("Next / previous file", format!("{c}↓ · {c}↑")),
        ("Side by side ⇄ unified", "u".into()),
        ("Collapse unchanged", "c".into()),
        ("Ignore whitespace", "w".into()),
        ("Font size", "+ · −".into()),
        ("Toggle file list", "b".into()),
        ("Merge: take A / B", "a · b   (1 · 2)".into()),
        ("Merge: A+B / B+A / base", "3 · 4 · 0".into()),
        ("Merge: edit / reset", "e · r".into()),
        ("Save merge", format!("{c}S")),
        ("Swap sides", format!("{c}⇧S")),
        ("Reload", format!("{c}R")),
        ("Start screen", format!("{c}O")),
    ];
    ui.set_min_width(320.0);
    ui.label(RichText::new("Keyboard shortcuts").strong());
    ui.separator();
    egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
        for (what, keys) in rows {
            ui.label(*what);
            ui.label(RichText::new(keys).monospace().weak());
            ui.end_row();
        }
    });
}

impl eframe::App for MiniDiffApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        for inc in platform::take() {
            self.open_incoming(inc);
        }
        {
            let dropped: Vec<Entry> = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| Entry::Fs(f.path().to_path_buf()))
                    .collect()
            });
            if !dropped.is_empty() {
                self.open_incoming(Incoming {
                    entries: dropped,
                    slot: None,
                });
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.global_keys(&ctx);
        self.app_bar(ui);

        let mut welcome_action = WelcomeAction::None;
        let mut merge_action = MergeAction::None;
        let bg = theme::palette(&ctx).bg;
        let central = egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(bg));
        central.show(ui, |ui| match &mut self.screen {
            Screen::Welcome => {
                welcome_action = welcome::ui(ui, &self.slots, &self.settings.recent, hovering);
            }
            Screen::File(fv) => {
                fv.ui(ui, &mut self.settings.view, false);
            }
            Screen::Folder(f) => f.ui(ui, &mut self.settings.view),
            Screen::Merge(m) => merge_action = m.ui(ui, &mut self.settings.view),
        });

        match welcome_action {
            WelcomeAction::None => {}
            WelcomeAction::PickFile(i) => self.pick(false, i),
            WelcomeAction::PickFolder(i) => self.pick(true, i),
            WelcomeAction::Clear(i) => self.slots[i] = None,
            WelcomeAction::Compare => self.compare_slots_if_ready(),
            WelcomeAction::DemoFolders => self.open_demo(false),
            WelcomeAction::DemoMerge => self.open_demo(true),
            WelcomeAction::OpenRecent(i) => {
                if let Some((a, b)) = self.settings.recent.get(i).cloned() {
                    let (a, b) = (PathBuf::from(a), PathBuf::from(b));
                    if a.exists() && b.exists() {
                        self.compare(Entry::Fs(a), Entry::Fs(b), None);
                    } else {
                        self.settings.recent.remove(i);
                        self.notify("That comparison no longer exists.", true);
                    }
                }
            }
        }
        match merge_action {
            MergeAction::None => {}
            MergeAction::SavedAndClose => self.finish_merge(&ctx),
            MergeAction::Cancel => {
                if let Screen::Merge(m) = &mut self.screen {
                    m.unsaved = false;
                    m.saved = false;
                }
                self.finish_merge(&ctx);
            }
        }

        if hovering {
            self.drop_overlay(&ctx);
        }
        self.toast_ui(&ctx);
        self.close_dialog_ui(&ctx);

        // Intercept closing the window with an unsaved merge.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if matches!(&self.screen, Screen::Merge(m) if m.unsaved) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.close_dialog = true;
            } else {
                self.update_exit_code();
            }
        }

        let title = self.window_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

/// Use the macOS system fonts (SF Pro / SF Mono) when available, and let the
/// UI font fall back to Hack, which carries most of the symbols we use.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("Hack".to_owned());
    #[cfg(target_os = "macos")]
    {
        use std::sync::Arc;
        let mut add = |name: &str, path: &str, family: egui::FontFamily| {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
                fonts.families.entry(family).or_default().insert(0, name.to_owned());
            }
        };
        add("sf-pro", "/System/Library/Fonts/SFNS.ttf", egui::FontFamily::Proportional);
        add("sf-mono", "/System/Library/Fonts/SFNSMono.ttf", egui::FontFamily::Monospace);
    }
    ctx.set_fonts(fonts);
}
