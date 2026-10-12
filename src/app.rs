//! Top-level application state and routing between screens.

use std::path::PathBuf;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use egui::{
    Align, Align2, FontId, Id, Key, LayerId, Layout, Margin, Modifiers, Order, RichText, Stroke, Ui,
    ViewportCommand, vec2,
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
use crate::ui::{ViewSettings, cmd, keys_free, dialog_footer};

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

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    theme: ThemeChoice,
    view: ViewSettings,
    recent: Vec<(String, String)>,
    /// Check for a newer release on startup and every 24 hours.
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

/// Nonblocking picker owned by the window that requested it.
struct PendingPicker {
    slot: usize,
    result: Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>>,
}

struct PickerWake(egui::Context);

impl Wake for PickerWake {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// State that belongs to a single native window.
struct DiffWindow {
    id: egui::ViewportId,
    screen: Screen,
    slots: [Option<Entry>; 2],
    picker: Option<PendingPicker>,
    toast: Option<(String, bool, f64)>,
    close_dialog: bool,
    about_open: bool,
    close_after_merge: bool,
    close_cancelled: bool,
    closed: bool,
    title: String,
}

impl DiffWindow {
    fn new(id: egui::ViewportId) -> Self {
        Self {
            id,
            screen: Screen::Welcome,
            slots: [None, None],
            picker: None,
            toast: None,
            close_dialog: false,
            about_open: false,
            close_after_merge: false,
            close_cancelled: false,
            closed: false,
            title: String::new(),
        }
    }

    fn accepts_incoming(&self) -> bool {
        !self.closed && matches!(self.screen, Screen::Welcome)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobalKind { Preferences, Shortcuts }

impl GlobalKind {
    fn id(self) -> egui::ViewportId { egui::ViewportId::from_hash_of(self.title()) }
    fn title(self) -> &'static str {
        match self { Self::Preferences => "Preferences — MiniDiff", Self::Shortcuts => "Keyboard Shortcuts — MiniDiff" }
    }
    fn size(self) -> egui::Vec2 {
        match self { Self::Preferences => vec2(408.0, 400.0), Self::Shortcuts => vec2(488.0, 900.0) }
    }
}

struct GlobalWindow { kind: GlobalKind, open: bool, center: bool, focus: bool }

fn global_windows() -> [GlobalWindow; 2] {
    [GlobalKind::Preferences, GlobalKind::Shortcuts].map(|kind| GlobalWindow { kind, open: false, center: false, focus: false })
}

/// All windows share preferences, recent comparisons, and one updater.
struct AppState {
    settings: Settings,
    root: Option<DiffWindow>,
    windows: Vec<DiffWindow>,
    updater: crate::update::Updater,
    manual_check: bool,
    focused: egui::ViewportId,
    requests: Vec<Incoming>,
    restart: Option<PathBuf>,
    quit_requested: bool,
    /// CLI merge tools must return to their caller after the last window closes.
    exit_when_windowless: bool,
    global_windows: [GlobalWindow; 2],
}

/// Borrow the shared services while rendering one window.
struct WindowUi<'a> {
    window: &'a mut DiffWindow,
    settings: &'a mut Settings,
    updater: &'a mut crate::update::Updater,
    manual_check: &'a mut bool,
    requests: &'a mut Vec<Incoming>,
    restart: &'a mut Option<PathBuf>,
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        platform::set_context(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        platform::menu::install();
        install_fonts(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.theme.apply(&cc.egui_ctx);
        let mut app = Self {
            settings,
            root: Some(DiffWindow::new(egui::ViewportId::ROOT)),
            windows: Vec::new(),
            updater: crate::update::Updater::new(),
            manual_check: false,
            focused: egui::ViewportId::ROOT,
            requests: Vec::new(),
            restart: None,
            quit_requested: false,
            exit_when_windowless: !cfg!(target_os = "macos") || matches!(launch, Launch::Merge { .. }),
            global_windows: global_windows(),
        };
        if app.settings.auto_update && !matches!(launch, Launch::Merge { .. }) {
            app.updater.check(&cc.egui_ctx);
        }
        WindowUi {
            window: app.root.as_mut().unwrap(),
            settings: &mut app.settings,
            updater: &mut app.updater,
            manual_check: &mut app.manual_check,
            requests: &mut app.requests,
            restart: &mut app.restart,
        }
        .launch(launch);
        app
    }

    fn route_incoming(&mut self, incoming: Incoming, ctx: &egui::Context) {
        let windowless = self.root.is_none() && self.windows.is_empty();
        // Empty requests are explicit New Window commands. OS opens reuse a
        // start screen, otherwise they create a new comparison window.
        let window = if !incoming.entries.is_empty() {
            self.root
                .iter_mut()
                .chain(self.windows.iter_mut())
                .find(|w| w.accepts_incoming())
        } else {
            None
        };
        let window = match window {
            Some(window) => window,
            None if windowless => {
                self.root = Some(DiffWindow::new(egui::ViewportId::ROOT));
                self.root.as_mut().unwrap()
            }
            None => {
                let id = egui::ViewportId::from_hash_of(("diff-window", crate::ui::next_id()));
                self.windows.push(DiffWindow::new(id));
                self.windows.last_mut().unwrap()
            }
        };
        let id = window.id;
        WindowUi {
            window,
            settings: &mut self.settings,
            updater: &mut self.updater,
            manual_check: &mut self.manual_check,
            requests: &mut self.requests,
            restart: &mut self.restart,
        }
        .open_incoming(incoming);
        // A new deferred viewport needs a UI pass to create its native
        // window. Wake the controller if every existing window is occluded
        // or minimized; it hides again once the new viewport is rendered.
        if ctx.input(|i| i.raw.viewports.values().all(|v| v.visible() == Some(false))) {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn remove_closed_windows(&mut self, ctx: &egui::Context) {
        if self.root.as_ref().is_some_and(|w| w.closed) {
            log::debug!(
                "Closing original comparison; {} other windows remain",
                self.windows.len()
            );
            self.root = None;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        for window in self.windows.iter().filter(|w| w.closed) {
            ctx.send_viewport_cmd_to(window.id, ViewportCommand::Visible(false));
        }
        self.windows.retain(|w| !w.closed);
        // Cancelling any unsaved-merge close cancels the pending Quit too.
        for window in self.root.iter_mut().chain(self.windows.iter_mut()) {
            if std::mem::take(&mut window.close_cancelled) {
                self.quit_requested = false;
            }
        }
        if self.root.is_none() && self.windows.is_empty()
            && (self.quit_requested || self.exit_when_windowless)
        {
            log::debug!("Quitting MiniDiff");
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn handle_root_events(&mut self, ctx: &egui::Context) {
        if let Some(id) = ctx.input(|i| i.raw.viewports.iter()
            .find(|(_, info)| info.focused == Some(true)).map(|(id, _)| *id)) {
            self.focused = id;
        }
        for action in platform::take_menu() {
            self.menu_action(action, ctx);
        }
        for window in &mut self.global_windows {
            if window.open && ctx.input(|i| i.raw.viewports.get(&window.kind.id()).is_some_and(|v| v.close_requested())) {
                window.open = false;
                ctx.send_viewport_cmd_to(window.kind.id(), ViewportCommand::Visible(false));
                ctx.request_repaint();
            }
        }
        if self.settings.auto_update && self.root.iter().chain(self.windows.iter())
            .any(|w| !w.closed && !matches!(w.screen, Screen::Merge(_))) {
            self.updater.check_automatically(ctx);
        }
        for incoming in std::mem::take(&mut self.requests) {
            self.route_incoming(incoming, ctx);
        }
        for incoming in platform::take() {
            self.route_incoming(incoming, ctx);
        }
        // Sheets complete even while their owning viewport is backgrounded.
        for window in self.root.iter_mut().chain(self.windows.iter_mut()).filter(|w| !w.closed) {
            WindowUi {
                window,
                settings: &mut self.settings,
                updater: &mut self.updater,
                manual_check: &mut self.manual_check,
                requests: &mut self.requests,
                restart: &mut self.restart,
            }.poll_picker(ctx);
        }
        // Deferred windows can skip their UI callback while occluded. Their
        // close events still arrive in the root's viewport inventory.
        let closing: Vec<_> = ctx.input(|i| {
            i.raw
                .viewports
                .iter()
                .filter(|(id, info)| **id != egui::ViewportId::ROOT && info.close_requested())
                .map(|(id, _)| *id)
                .collect()
        });
        for id in closing {
            if let Some(window) = self.windows.iter_mut().find(|w| w.id == id) {
                WindowUi {
                    window,
                    settings: &mut self.settings,
                    updater: &mut self.updater,
                    manual_check: &mut self.manual_check,
                    requests: &mut self.requests,
                    restart: &mut self.restart,
                }
                .request_close();
                if !window.closed {
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                    ctx.request_repaint_of(id);
                }
            }
        }
        // Close events may also arrive during logic-only frames, or again
        // after the original window has been hidden. Never let eframe end
        // the event loop on a normal macOS window close, even with no windows left.
        if ctx.input(|i| i.viewport().close_requested())
            && (self.root.is_some() || !self.windows.is_empty()
                || (!self.quit_requested && !self.exit_when_windowless)) {
            log::debug!(
                "Cancelling root close event; {} comparison windows remain",
                self.windows.len()
            );
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if let Some(window) = &mut self.root {
                WindowUi {
                    window,
                    settings: &mut self.settings,
                    updater: &mut self.updater,
                    manual_check: &mut self.manual_check,
                    requests: &mut self.requests,
                    restart: &mut self.restart,
                }
                .request_close();
            }
        }
        self.remove_closed_windows(ctx);
    }

    fn menu_action(&mut self, action: platform::MenuAction, ctx: &egui::Context) {
        use platform::MenuAction;
        let global = match action {
            MenuAction::Preferences => Some(GlobalKind::Preferences),
            MenuAction::Shortcuts => Some(GlobalKind::Shortcuts),
            _ => None,
        };
        if let Some(kind) = global {
            self.focused = kind.id();
            let window = self.global_windows.iter_mut().find(|w| w.kind == kind).unwrap();
            window.open = true;
            window.center = true;
            window.focus = true;
            if self.root.is_none() || ctx.input(|i| i.raw.viewports.values().all(|v| v.visible() == Some(false))) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            ctx.send_viewport_cmd_to(kind.id(), ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(kind.id(), ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(kind.id(), ViewportCommand::Focus);
            ctx.request_repaint();
            return;
        }
        if matches!(action, MenuAction::CloseWindow)
            && let Some(window) = self.global_windows.iter_mut().find(|w| w.kind.id() == self.focused) {
                window.open = false;
                ctx.send_viewport_cmd_to(window.kind.id(), ViewportCommand::Visible(false));
                ctx.request_repaint();
                return;
        }
        if matches!(action, MenuAction::Reopen) {
            let global = self.global_windows.iter().find(|w| w.open &&
                (w.kind.id() == self.focused || (self.root.is_none() && self.windows.is_empty()))).map(|w| w.kind);
            if let Some(kind) = global {
                self.menu_action(match kind {
                    GlobalKind::Preferences => MenuAction::Preferences,
                    GlobalKind::Shortcuts => MenuAction::Shortcuts,
                }, ctx);
                return;
            }
            if self.root.is_none() && self.windows.is_empty() {
                self.route_incoming(Incoming { entries: Vec::new(), slot: None }, ctx);
            } else if let Some(window) = self.root.iter().chain(self.windows.iter())
                .find(|w| w.id == self.focused && !w.closed)
                .or_else(|| self.root.iter().chain(self.windows.iter()).find(|w| !w.closed))
            {
                ctx.send_viewport_cmd_to(window.id, ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(window.id, ViewportCommand::Visible(true));
                ctx.send_viewport_cmd_to(window.id, ViewportCommand::Focus);
            }
            return;
        }
        if matches!(action, MenuAction::NewWindow) {
            self.route_incoming(Incoming { entries: Vec::new(), slot: None }, ctx);
            return;
        }
        if matches!(action, MenuAction::Quit) {
            self.quit_requested = true;
            for window in &mut self.global_windows { window.open = false; }
            for window in self.root.iter_mut().chain(self.windows.iter_mut()) {
                window.close_cancelled = false;
            }
        } else if self.root.is_none() && self.windows.is_empty()
            && matches!(action, MenuAction::About | MenuAction::CheckUpdates | MenuAction::Home)
        {
            self.route_incoming(Incoming { entries: Vec::new(), slot: None }, ctx);
        }
        if matches!(action, MenuAction::CheckUpdates) {
            self.manual_check = true;
            self.updater.check(ctx);
        }
        let target = self.root.iter().chain(self.windows.iter())
            .find(|w| w.id == self.focused && !w.closed)
            .or_else(|| self.root.iter().chain(self.windows.iter()).find(|w| !w.closed))
            .map(|w| w.id);
        for window in self.root.iter_mut().chain(self.windows.iter_mut()) {
            if Some(window.id) != target && !matches!(action, MenuAction::Quit) { continue; }
            let id = window.id;
            let mut view = WindowUi {
                window, settings: &mut self.settings, updater: &mut self.updater,
                manual_check: &mut self.manual_check, requests: &mut self.requests,
                restart: &mut self.restart,
            };
            if matches!(action, MenuAction::About | MenuAction::Preferences | MenuAction::Shortcuts | MenuAction::CloseWindow | MenuAction::Quit) {
                view.window.about_open = false;
            }
            match action {
                MenuAction::About => view.window.about_open = true,
                MenuAction::CloseWindow | MenuAction::Quit => view.request_close(),
                MenuAction::Home => view.go_home(),
                MenuAction::Reload => view.reload(),
                MenuAction::Swap => view.swap(),
                MenuAction::CheckUpdates | MenuAction::NewWindow | MenuAction::Reopen | MenuAction::Preferences | MenuAction::Shortcuts => {}
            }
            if view.window.close_dialog {
                ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
            }
            ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
            ctx.request_repaint_of(id);
        }
    }
}

impl WindowUi<'_> {
    fn launch(&mut self, launch: Launch) {
        match launch {
            Launch::Welcome => {}
            Launch::Open(entries) => self.open_incoming(Incoming { entries, slot: None }),
            Launch::Compare { left, right, labels } => self.compare(left, right, labels),
            Launch::Merge {
                base,
                local,
                remote,
                output,
                labels,
            } => self.open_merge(base, local, remote, output, labels, true),
        }
    }

    fn notify(&mut self, msg: impl Into<String>, error: bool) {
        self.window.toast = Some((msg.into(), error, f64::NAN));
    }

    // ------------------------------------------------------------------
    // Routing

    fn open_incoming(&mut self, inc: Incoming) {
        if !self.window.accepts_incoming() && inc.slot.is_none() {
            self.requests.push(inc);
            return;
        }
        let Incoming { mut entries, slot } = inc;
        if let Some(s) = slot {
            if let Some(e) = entries.drain(..).next() {
                self.window.slots[s.min(1)] = Some(e);
                self.window.screen = Screen::Welcome;
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
                if self.window.slots[0].is_some() && self.window.slots[1].is_some() {
                    self.window.slots = [None, None];
                }
                let i = if self.window.slots[0].is_none() { 0 } else { 1 };
                self.window.slots[i] = Some(e);
                self.window.screen = Screen::Welcome;
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
        if let [Some(a), Some(b)] = &self.window.slots {
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
        self.window.screen = Screen::Merge(Box::new(view));
        true
    }

    fn compare(&mut self, a: Entry, b: Entry, labels: Option<(String, String)>) {
        if a.is_dir() != b.is_dir() {
            self.notify("Can't compare a file with a folder.", true);
            self.window.slots = [Some(a), None];
            self.window.screen = Screen::Welcome;
            return;
        }
        // Recent stores reopenable filesystem comparisons; embedded demos stay out.
        if let (Some(pa), Some(pb)) = (a.fs_path(), b.fs_path()) {
            let pair = (pa.display().to_string(), pb.display().to_string());
            self.settings.recent.retain(|r| *r != pair);
            self.settings.recent.insert(0, pair);
            self.settings.recent.truncate(12);
        }
        self.window.slots = [Some(a.clone()), Some(b.clone())];
        self.window.screen = if a.is_dir() {
            Screen::Folder(Box::new(FolderView::new(a, b)))
        } else {
            Screen::File(Box::new(FileView::new(
                Some(a),
                Some(b),
                labels,
                self.settings.view.diff,
            )))
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
        self.window.screen = Screen::Merge(Box::new(MergeView::new(doc, file_name, output, labels, tool_mode)));
    }

    fn open_demo(&mut self, merge: bool) {
        if merge {
            let (base, local, remote) = crate::demo::merge();
            let labels = [
                "Local · local.rs".into(),
                "Base · base.rs".into(),
                "Remote · remote.rs".into(),
            ];
            self.open_merge(Some(base), local, remote, None, Some(labels), false);
            if let Screen::Merge(m) = &mut self.window.screen {
                m.file_name = "merged.rs".into();
                m.demo = true;
            }
        } else {
            let (a, b) = crate::demo::folders();
            self.compare(a, b, None);
        }
    }

    fn go_home(&mut self) {
        if let Screen::Merge(m) = &self.window.screen
            && m.needs_save()
        {
            self.window.close_after_merge = false;
            self.window.close_dialog = true;
            return;
        }
        self.window.screen = Screen::Welcome;
    }

    fn swap(&mut self) {
        match &mut self.window.screen {
            Screen::File(fv) => {
                let (a, b) = fv.entries();
                let labels = Some((fv.right_label.clone(), fv.left_label.clone()));
                self.window.slots = [b.clone(), a.clone()];
                self.window.screen = Screen::File(Box::new(FileView::new(b, a, labels, self.settings.view.diff)));
            }
            Screen::Folder(f) => {
                let (a, b) = (f.left.clone(), f.right.clone());
                self.window.slots = [Some(b.clone()), Some(a.clone())];
                self.window.screen = Screen::Folder(Box::new(FolderView::new(b, a)));
            }
            Screen::Merge(m) => m.swap(),
            _ => {}
        }
    }

    fn reload(&mut self) {
        match &mut self.window.screen {
            Screen::File(fv) => {
                let (a, b) = fv.entries();
                let labels = Some((fv.left_label.clone(), fv.right_label.clone()));
                **fv = FileView::new(a, b, labels, self.settings.view.diff);
            }
            Screen::Folder(f) => f.reload(),
            _ => {}
        }
    }
    fn pick(&mut self, slot: usize, ctx: &egui::Context) {
        if self.window.picker.is_some() {
            return;
        }
        // Create the native sheet on the main thread; never run AppKit's
        // blocking modal loop from inside an egui/winit event callback.
        let dialog = rfd::AsyncFileDialog::new().set_title(if slot == 0 { "Choose A" } else { "Choose B" });
        #[cfg(target_os = "macos")]
        let result = dialog.pick_file_or_folder();
        // rfd exposes its combined picker only on macOS. Keep source ports
        // buildable until their native backends support the same selection.
        #[cfg(not(target_os = "macos"))]
        let result = dialog.pick_file();
        self.window.picker = Some(PendingPicker {
            slot,
            result: Box::pin(async move { result.await.map(|file| file.path().to_owned()) }),
        });
        self.poll_picker(ctx);
    }

    fn poll_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = &mut self.window.picker else { return };
        let waker = Waker::from(Arc::new(PickerWake(ctx.clone())));
        let Poll::Ready(path) = picker.result.as_mut().poll(&mut Context::from_waker(&waker)) else { return };
        let slot = picker.slot;
        self.window.picker = None;
        if let Some(path) = path {
            // A drop or menu action may already have opened a comparison.
            // Preserve it and route the selection to another welcome window.
            let incoming = Incoming {
                entries: vec![Entry::Fs(path)],
                slot: self.window.accepts_incoming().then_some(slot),
            };
            self.open_incoming(incoming);
            ctx.request_repaint_of(self.window.id);
        }
    }

    // ------------------------------------------------------------------
    // UI

    fn global_keys(&mut self, ctx: &egui::Context) {
        let (new_window, close, home, reload, swap, preferences_key) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::Comma),
            )
        });
        if new_window {
            self.requests.push(Incoming {
                entries: Vec::new(),
                slot: None,
            });
        }
        if close {
            self.request_close();
        }
        if home {
            self.go_home();
        }
        if reload {
            self.reload();
        }
        if swap {
            self.swap();
        }
        if preferences_key {
            platform::push_menu(platform::MenuAction::Preferences);
        }
        if keys_free(ctx) && matches!(self.window.screen, Screen::File(_) | Screen::Folder(_)) {
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
        if matches!(self.window.screen, Screen::Welcome)
            && !*self.manual_check
            && !matches!(
                self.updater.state(),
                crate::update::State::Available(_)
                    | crate::update::State::Installing(_)
                    | crate::update::State::Installed(_)
            )
        {
            return;
        }
        let pal = theme::palette(ui.ctx());
        let frame = egui::Frame::new()
            .fill(pal.panel)
            .inner_margin(Margin::symmetric(10, 6))
            .stroke(Stroke::NONE);
        egui::Panel::top("app-bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                if matches!(self.window.screen, Screen::File(_) | Screen::Folder(_) | Screen::Merge(_))
                    && ui.button("⇄ Swap")
                        .on_hover_text(format!("Swap A and B  ({}⇧S)", cmd()))
                        .clicked()
                {
                    self.swap();
                }
                let crumb = match &self.window.screen {
                    Screen::Welcome | Screen::File(_) | Screen::Folder(_) => None,
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
                    self.update_ui(ui);
                    if matches!(self.window.screen, Screen::File(_) | Screen::Folder(_))
                        && ui.button("⟳").on_hover_text(format!("Reload  ({}R)", cmd())).clicked()
                    {
                        self.reload();
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
        let msg = match (&self.window.slots, &self.window.screen) {
            ([Some(_), None], Screen::Welcome) => "Drop to set B",
            _ => "Drop two files or folders to compare",
        };
        p.text(
            inner.center(),
            Align2::CENTER_CENTER,
            "⇣",
            FontId::proportional(56.0),
            pal.accent,
        );
        p.text(
            inner.center() + vec2(0.0, 50.0),
            Align2::CENTER_CENTER,
            msg,
            FontId::proportional(20.0),
            pal.text,
        );
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some((msg, err, since)) = &mut self.window.toast else {
            return;
        };
        if since.is_nan() {
            *since = now;
        }
        if now - *since > 6.0 {
            self.window.toast = None;
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
        if !self.window.close_dialog {
            return;
        }
        let (file_name, error) = match &self.window.screen {
            Screen::Merge(m) => (m.file_name.as_str(), m.message.as_ref().filter(|(_, error)| *error).map(|(text, _)| text.as_str())),
            _ => ("Merge result", None),
        };
        let modal = information_modal(ctx, "unsaved-merge").show(ctx, |ui| {
            unsaved_merge_contents(ui, file_name, error)
        });
        let mut choice = modal.inner;
        if modal.should_close() {
            choice = Some(MergeCloseChoice::Cancel);
        }
        match choice {
            Some(MergeCloseChoice::Save) => {
                if let Screen::Merge(m) = &mut self.window.screen
                    && m.save(true)
                {
                    self.window.close_dialog = false;
                    self.finish_merge(ctx);
                }
            }
            Some(MergeCloseChoice::Discard) => {
                self.window.close_dialog = false;
                if let Screen::Merge(m) = &mut self.window.screen {
                    m.unsaved = false;
                }
                self.finish_merge(ctx);
            }
            Some(_) => {
                self.window.close_dialog = false;
                self.window.close_cancelled = self.window.close_after_merge;
                self.window.close_after_merge = false;
            }
            None => {}
        }
    }

    fn request_close(&mut self) {
        if matches!(&self.window.screen, Screen::Merge(m) if m.needs_save()) {
            self.window.close_after_merge = true;
            self.window.close_dialog = true;
        } else {
            self.update_exit_code();
            self.window.closed = true;
        }
    }

    fn finish_merge(&mut self, _ctx: &egui::Context) {
        let tool = matches!(&self.window.screen, Screen::Merge(m) if m.tool_mode);
        if tool || self.window.close_after_merge {
            self.update_exit_code();
            self.window.closed = true;
        } else {
            self.window.screen = Screen::Welcome;
        }
    }

    fn update_exit_code(&self) {
        if let Screen::Merge(m) = &self.window.screen
            && m.tool_mode
        {
            platform::set_exit_code(if m.saved && !m.unsaved { 0 } else { 1 });
        }
    }

    fn window_title(&self) -> String {
        match &self.window.screen {
            Screen::Welcome => "MiniDiff".into(),
            Screen::File(fv) => format!("{} ⇄ {} — MiniDiff", short(&fv.left_label), short(&fv.right_label)),
            Screen::Folder(f) => format!("{} — MiniDiff", f.title()),
            Screen::Merge(m) => format!("{}Merge {} — MiniDiff", if m.needs_save() { "● " } else { "" }, m.file_name),
        }
    }
}

fn short(label: &str) -> String {
    label.rsplit(['/', '\\']).next().unwrap_or(label).to_owned()
}

impl WindowUi<'_> {
    /// Update pill in the app bar (native only).
    fn update_ui(&mut self, ui: &mut Ui) {
        use crate::update::{State, Updater};
        let pal = theme::palette(ui.ctx());
        match self.updater.state() {
            State::Available(m) => {
                let b =
                    egui::Button::new(RichText::new(format!("⬆ Update to {}", m.version)).color(egui::Color32::WHITE))
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
                    *self.restart = Some(bundle);
                }
            }
            State::UpToDate if *self.manual_check => {
                *self.manual_check = false;
                self.notify(format!("MiniDiff {} is up to date.", env!("CARGO_PKG_VERSION")), false);
            }
            State::Failed(e) if *self.manual_check => {
                *self.manual_check = false;
                self.notify(e, true);
            }
            State::Checking if *self.manual_check => {
                ui.spinner();
            }
            _ => {}
        }
    }
}

fn information_modal(ctx: &egui::Context, id: &str) -> egui::Modal {
    crate::ui::dialog_modal(ctx, Id::new(id))
}

fn information_footer(ui: &mut Ui) -> bool {
    dialog_footer(ui, |ui| {
        ui.add_sized([76.0, 28.0], egui::Button::new("Close")).clicked()
    })
}

#[derive(Clone, Copy)]
enum MergeCloseChoice { Save, Discard, Cancel }

fn unsaved_merge_contents(ui: &mut Ui, file_name: &str, error: Option<&str>) -> Option<MergeCloseChoice> {
    let pal = theme::palette(ui.ctx());
    ui.set_width(380.0_f32.min(ui.ctx().content_rect().width() - 80.0));
    ui.label(RichText::new("Unsaved merge").size(22.0).strong());
    ui.add_space(12.0);
    ui.add(egui::Label::new(RichText::new(file_name).size(14.0).strong()).truncate()).on_hover_text(file_name);
    ui.add_space(4.0);
    ui.label(RichText::new("Save your changes before continuing. Discarding loses your unsaved merge changes.").size(13.0).color(pal.text_dim));
    if let Some(error) = error {
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(error).size(13.0).color(pal.del_fg)).truncate()).on_hover_text(error);
    }
    dialog_footer(ui, |ui| {
        let mut choice = None;
        let save = egui::Button::new(RichText::new("Save").strong().color(egui::Color32::WHITE)).fill(pal.accent);
        if ui.add_sized([76.0, 28.0], save).clicked() { choice = Some(MergeCloseChoice::Save); }
        if ui.add_sized([76.0, 28.0], egui::Button::new("Cancel")).clicked() { choice = Some(MergeCloseChoice::Cancel); }
        if ui.add_sized([76.0, 28.0], egui::Button::new(RichText::new("Discard").color(pal.del_fg))).clicked() { choice = Some(MergeCloseChoice::Discard); }
        choice
    })
}

fn preferences_contents(ui: &mut Ui, settings: &mut Settings, update: &crate::update::State) -> bool {
    use crate::update::State;
    let pal = theme::palette(ui.ctx());
    ui.set_width(360.0_f32.min(ui.ctx().content_rect().width() - 48.0));
    ui.label(RichText::new("Preferences").size(22.0).strong());
    ui.add_space(18.0);
    ui.label(RichText::new("Appearance").size(14.0).strong().color(pal.text));
    ui.add_space(6.0);
    if crate::ui::segmented(ui, &mut settings.theme, &[
        (ThemeChoice::System, "System", "Follow the system appearance"),
        (ThemeChoice::Light, "Light", "Use light appearance"),
        (ThemeChoice::Dark, "Dark", "Use dark appearance"),
    ]) {
        settings.theme.apply(ui.ctx());
    }
    ui.add_space(18.0);
    ui.label(RichText::new("Updates").size(14.0).strong().color(pal.text));
    ui.add_space(6.0);
    ui.checkbox(&mut settings.auto_update, "Check for updates automatically");
    ui.add_space(4.0);
    ui.label(RichText::new("On startup and every 24 hours while MiniDiff is open.").size(13.0).color(pal.text_dim));
    ui.add_space(12.0);
    let busy = matches!(update, State::Checking | State::Installing(_) | State::Installed(_));
    let clicked = ui.add_enabled(!busy, egui::Button::new("Check for Updates").min_size(vec2(0.0, 28.0))).clicked();
    match update {
        State::Checking => { ui.horizontal(|ui| { ui.spinner(); ui.label("Checking for updates…"); }); }
        State::UpToDate => { ui.label(RichText::new("MiniDiff is up to date.").size(13.0).color(pal.add_fg)); }
        State::Available(m) => { ui.label(RichText::new(format!("MiniDiff {} is available.", m.version)).size(13.0).color(pal.add_fg)); }
        State::Failed(error) => { ui.label(RichText::new("Could not check for updates.").size(13.0).color(pal.del_fg)).on_hover_text(error); }
        State::Installing(_) => { ui.label("Installing update…"); }
        State::Installed(_) => { ui.label("Restart to finish the update."); }
        State::Idle => {}
    }
    clicked
}

fn shortcut_section(ui: &mut Ui, title: &str, rows: &[(&str, String)]) {
    let pal = theme::palette(ui.ctx());
    ui.label(RichText::new(title).size(14.0).strong().color(pal.text));
    ui.add_space(6.0);
    egui::Grid::new(title)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (action, keys) in rows {
                ui.allocate_ui_with_layout(vec2(240.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(240.0);
                    ui.label(RichText::new(*action).size(13.0).color(pal.text));
                });
                egui::Frame::new()
                    .fill(pal.toolbar)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(4)
                    .inner_margin(Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(RichText::new(keys).monospace().size(12.0).color(pal.text));
                    });
                ui.end_row();
            }
        });
}

fn help_shortcuts(ui: &mut Ui) {
    let c = cmd();
    let a = crate::ui::alt();
    let pal = theme::palette(ui.ctx());
    ui.set_width(440.0_f32.min(ui.ctx().content_rect().width() - 48.0));
    ui.label(RichText::new("Keyboard shortcuts").size(22.0).strong());
    ui.add_space(4.0);
    ui.label(RichText::new("Single-key shortcuts apply when you aren’t editing text.").size(13.0).color(pal.text_dim));
    ui.add_space(18.0);
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    let max_height = (ui.ctx().content_rect().height() - 240.0).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("shortcut-list")
        .max_height(max_height)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            shortcut_section(ui, "Navigation", &[
                ("Next / previous change", "n / p · j / k".into()),
                ("Alternate navigation", format!("{a}↓ / {a}↑")),
                ("Next / previous file", format!("{c}↓ / {c}↑")),
            ]);
            ui.add_space(18.0);
            shortcut_section(ui, "Comparison", &[
                ("Side by side / unified", "u".into()),
                ("Collapse unchanged lines", "c".into()),
                ("Ignore whitespace", "w".into()),
                ("Increase / decrease font size", "+ / −".into()),
                ("Toggle file list", "b".into()),
                ("Swap sides", format!("{c}⇧S")),
                ("Reload files", format!("{c}R")),
            ]);
            ui.add_space(18.0);
            shortcut_section(ui, "Merge", &[
                ("Take A / B", "a / b · 1 / 2".into()),
                ("Combine A+B / B+A", "3 / 4".into()),
                ("Take base", "0".into()),
                ("Edit / reset chunk", "e / r".into()),
                ("Save result", format!("{c}S")),
            ]);
            ui.add_space(18.0);
            shortcut_section(ui, "Windows & settings", &[
                ("New / close window", format!("{c}N / {c}W")),
                ("Start screen", format!("{c}O")),
                ("Preferences", format!("{c},")),
            ]);
        });
}

fn about_contents(ui: &mut Ui) {
    let pal = theme::palette(ui.ctx());
    ui.set_width(320.0);
    ui.vertical_centered(|ui| {
        let (icon, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
        welcome::paint_logo(ui.painter(), icon.center(), 64.0);
        ui.add_space(12.0);
        ui.label(RichText::new("About MiniDiff").size(22.0).strong());
        ui.add_space(4.0);
        ui.label(RichText::new(format!("Version {}", crate::update::CURRENT)).size(13.0).color(pal.text_dim));
        ui.add_space(18.0);
        ui.label(RichText::new("Compare text files and folders.").size(14.0));
        ui.label(RichText::new("Resolve merge conflicts.").size(14.0));
        ui.add_space(16.0);
        ui.hyperlink_to("minidiff.signalwerk.ch", "https://minidiff.signalwerk.ch/");
        ui.add_space(6.0);
        ui.label(RichText::new("MIT licensed").size(12.0).color(pal.text_dim));
    });
}

impl WindowUi<'_> {
    fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();

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
        central.show(ui, |ui| match &mut self.window.screen {
            Screen::Welcome => {
                welcome_action = welcome::ui(ui, &self.window.slots, &self.settings.recent, hovering);
            }
            Screen::File(fv) => {
                fv.ui(ui, &mut self.settings.view, false);
            }
            Screen::Folder(f) => f.ui(ui, &mut self.settings.view),
            Screen::Merge(m) => merge_action = m.ui(ui, &mut self.settings.view),
        });

        match welcome_action {
            WelcomeAction::None => {}
            WelcomeAction::Pick(i) => self.pick(i, &ctx),
            WelcomeAction::Clear(i) => self.window.slots[i] = None,
            WelcomeAction::Compare => self.compare_slots_if_ready(),
            WelcomeAction::DemoFiles => {
                let (a, b) = crate::demo::files();
                self.compare(a, b, None);
            }
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
                if let Screen::Merge(m) = &mut self.window.screen {
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
        if self.window.about_open {
            let response = information_modal(&ctx, "about-minidiff").show(&ctx, |ui| {
                about_contents(ui);
                information_footer(ui)
            });
            if response.inner || response.should_close() {
                self.window.about_open = false;
            }
        }

        // The controller removes only this viewport after the merge guard.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.request_close();
        }

        let title = self.window_title();
        if title != self.window.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window.title = title;
        }
    }
}

/// Deferred viewport callbacks borrow the same application state as the root.
pub struct MiniDiffApp {
    state: Arc<Mutex<AppState>>,
}

impl MiniDiffApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        Self {
            state: Arc::new(Mutex::new(AppState::new(cc, launch))),
        }
    }
}

impl AppState {
    fn global_ui(&mut self, kind: GlobalKind, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let window = self.global_windows.iter_mut().find(|w| w.kind == kind).unwrap();
        if !window.open { return; }
        if window.center {
            #[cfg(target_os = "macos")]
            { window.center = !platform::macos::center_window(kind.title()); }
            #[cfg(not(target_os = "macos"))]
            if let Some(command) = ViewportCommand::center_on_screen(&ctx) {
                ctx.send_viewport_cmd(command);
                window.center = false;
            }
            if window.center { ctx.request_repaint(); }
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        } else if window.focus {
            // Apply focus after the first frame, once the controller's wake-up is over.
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            window.focus = false;
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        let previous_settings = self.settings.clone();
        let pal = theme::palette(&ctx);
        let mut close = ctx.input(|i| i.viewport().close_requested());
        close |= ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)
            || i.consume_key(Modifiers::COMMAND, Key::W));
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(pal.panel).inner_margin(Margin::same(24)))
            .show(ui, |ui| {
                match kind {
                    GlobalKind::Preferences => {
                        if preferences_contents(ui, &mut self.settings, &self.updater.state()) {
                            self.manual_check = true;
                            self.updater.check(&ctx);
                        }
                    }
                    GlobalKind::Shortcuts => help_shortcuts(ui),
                }
                ui.add_space((ui.available_height() - 70.0).max(0.0));
                close |= information_footer(ui);
            });
        if close {
            self.global_windows.iter_mut().find(|w| w.kind == kind).unwrap().open = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        if self.settings != previous_settings {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            for window in &self.windows { ctx.request_repaint_of(window.id); }
            for window in &self.global_windows {
                if window.open { ctx.request_repaint_of(window.kind.id()); }
            }
        }
    }

    fn root_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let previous_settings = self.settings.clone();
        if let Some(window) = &mut self.root {
            WindowUi {
                window,
                settings: &mut self.settings,
                updater: &mut self.updater,
                manual_check: &mut self.manual_check,
                requests: &mut self.requests,
                restart: &mut self.restart,
            }
            .ui(ui);
        } else {
            // eframe's root owns the event loop. Keep it hidden after its
            // comparison closes, so the other windows can live independently.
            if !self.global_windows.iter().any(|w| w.open && (w.center || w.focus)) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
        }
        for incoming in std::mem::take(&mut self.requests) {
            self.route_incoming(incoming, &ctx);
        }
        if let Some(bundle) = self.restart.take() {
            let unsaved = self
                .root
                .iter()
                .chain(self.windows.iter())
                .any(|w| matches!(&w.screen, Screen::Merge(m) if m.needs_save()));
            if unsaved {
                if let Some(window) = self.root.iter_mut().chain(self.windows.iter_mut()).next() {
                    window.toast = Some((
                        "Save merge changes in all windows before restarting.".into(),
                        true,
                        f64::NAN,
                    ));
                    ctx.send_viewport_cmd_to(window.id, ViewportCommand::Focus);
                }
            } else {
                crate::update::relaunch(&bundle);
                self.root = None;
                self.windows.clear();
                self.quit_requested = true;
            }
        }
        self.remove_closed_windows(&ctx);
        if self.settings != previous_settings {
            for window in &self.windows {
                ctx.request_repaint_of(window.id);
            }
        }
    }
}

impl eframe::App for MiniDiffApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.lock().unwrap().handle_root_events(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let viewports: Vec<_> = {
            let mut app = self.state.lock().unwrap();
            app.root_ui(ui);
            app.windows
                .iter()
                .map(|w| {
                    (
                        w.id,
                        egui::ViewportBuilder::default()
                            .with_title(if w.title.is_empty() { "MiniDiff" } else { &w.title })
                            .with_inner_size([1360.0, 860.0])
                            .with_min_inner_size([640.0, 400.0])
                            .with_drag_and_drop(true),
                    )
                })
                .collect()
        };
        // Release the lock before registering callbacks: an embedded backend
        // can call them synchronously, while native windows run independently.
        for (id, builder) in viewports {
            let state = Arc::clone(&self.state);
            ctx.show_viewport_deferred(id, builder, move |ui, _class| {
                let mut app = state.lock().unwrap();
                let previous_settings = app.settings.clone();
                let AppState {
                    settings,
                    windows,
                    updater,
                    manual_check,
                    requests,
                    restart,
                    ..
                } = &mut *app;
                if let Some(window) = windows.iter_mut().find(|w| w.id == id) {
                    WindowUi {
                        window,
                        settings,
                        updater,
                        manual_check,
                        requests,
                        restart,
                    }
                    .ui(ui);
                    if window.closed || window.close_cancelled || !requests.is_empty() || restart.is_some() {
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                }
                if *settings != previous_settings {
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    for window in windows {
                        if window.id != id {
                            ui.ctx().request_repaint_of(window.id);
                        }
                    }
                }
            });
        }
        let globals: Vec<_> = self.state.lock().unwrap().global_windows.iter()
            .filter(|w| w.open).map(|w| w.kind).collect();
        for kind in globals {
            let state = Arc::clone(&self.state);
            let builder = egui::ViewportBuilder::default()
                .with_title(kind.title())
                .with_active(true)
                .with_visible(true)
                .with_inner_size(kind.size())
                .with_min_inner_size([kind.size().x, 400.0])
                .with_clamp_size_to_monitor_size(true)
                .with_resizable(kind == GlobalKind::Shortcuts);
            ctx.show_viewport_deferred(kind.id(), builder, move |ui, _class| {
                state.lock().unwrap().global_ui(kind, ui);
            });
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let app = self.state.lock().unwrap();
        eframe::set_value(storage, eframe::APP_KEY, &app.settings);
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
        add(
            "sf-pro",
            "/System/Library/Fonts/SFNS.ttf",
            egui::FontFamily::Proportional,
        );
        add(
            "sf-mono",
            "/System/Library/Fonts/SFNSMono.ttf",
            egui::FontFamily::Monospace,
        );
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppState {
        AppState {
            settings: Settings::default(),
            root: Some(DiffWindow::new(egui::ViewportId::ROOT)),
            windows: Vec::new(),
            updater: crate::update::Updater::new(),
            manual_check: false,
            focused: egui::ViewportId::ROOT,
            requests: Vec::new(),
            restart: None,
            quit_requested: false,
            exit_when_windowless: !cfg!(target_os = "macos"),
            global_windows: global_windows(),
        }
    }

    fn root_ui(app: &mut AppState) -> WindowUi<'_> {
        WindowUi {
            window: app.root.as_mut().unwrap(),
            settings: &mut app.settings,
            updater: &mut app.updater,
            manual_check: &mut app.manual_check,
            requests: &mut app.requests,
            restart: &mut app.restart,
        }
    }

    fn folders() -> Incoming {
        let (a, b) = crate::demo::folders();
        Incoming {
            entries: vec![a, b],
            slot: None,
        }
    }

    #[test]
    fn home_screen_demos_preserve_recent_comparisons() {
        for with_history in [false, true] {
            let mut app = app();
            if with_history {
                root_ui(&mut app).compare(
                    Entry::Fs(PathBuf::from("samples/left/src/util/format.ts")),
                    Entry::Fs(PathBuf::from("samples/right/src/util/format.ts")),
                    None,
                );
                assert_eq!(app.settings.recent.len(), 1);
                root_ui(&mut app).go_home();
            }
            let recent = app.settings.recent.clone();

            let (a, b) = crate::demo::files();
            root_ui(&mut app).compare(a, b, None);
            assert!(matches!(app.root.as_ref().unwrap().screen, Screen::File(_)));
            assert_eq!(app.settings.recent, recent);
            root_ui(&mut app).swap();
            root_ui(&mut app).reload();
            root_ui(&mut app).go_home();
            root_ui(&mut app).compare_slots_if_ready();
            assert_eq!(app.settings.recent, recent);

            root_ui(&mut app).go_home();
            root_ui(&mut app).open_demo(false);
            assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Folder(_)));
            assert_eq!(app.settings.recent, recent);
            root_ui(&mut app).swap();
            root_ui(&mut app).reload();
            root_ui(&mut app).go_home();
            root_ui(&mut app).compare_slots_if_ready();
            assert_eq!(app.settings.recent, recent);

            root_ui(&mut app).go_home();
            root_ui(&mut app).open_demo(true);
            assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Merge(_)));
            root_ui(&mut app).go_home();
            assert_eq!(app.settings.recent, recent);
        }
    }

    #[test]
    fn unified_picker_automatically_compares_files_or_folders() {
        let ctx = egui::Context::default();
        for (a, b, folders) in [
            ("samples/left/src/util/format.ts", "samples/right/src/util/format.ts", false),
            ("samples/left", "samples/right", true),
        ] {
            let mut app = app();
            for (slot, path) in [a, b].into_iter().enumerate() {
                app.root.as_mut().unwrap().picker = Some(PendingPicker {
                    slot,
                    result: Box::pin(std::future::ready(Some(PathBuf::from(path)))),
                });
                app.handle_root_events(&ctx);
            }
            let screen = &app.root.as_ref().unwrap().screen;
            assert!(if folders { matches!(screen, Screen::Folder(_)) } else { matches!(screen, Screen::File(_)) });
        }
    }

    #[test]
    fn picker_results_are_applied_to_the_background_owner_and_cancel_is_harmless() {
        let ctx = egui::Context::default();
        let mut app = app();
        let id = egui::ViewportId::from_hash_of("picker-child");
        let mut child = DiffWindow::new(id);
        child.picker = Some(PendingPicker {
            slot: 1,
            result: Box::pin(std::future::ready(Some(PathBuf::from("samples/right/src/util/format.ts")))),
        });
        app.windows.push(child);
        app.handle_root_events(&ctx);
        assert!(app.root.as_ref().unwrap().slots.iter().all(Option::is_none));
        let child = &mut app.windows[0];
        assert!(child.slots[0].is_none());
        assert_eq!(child.slots[1].as_ref().unwrap().fs_path(), Some(std::path::Path::new("samples/right/src/util/format.ts")));
        assert!(child.picker.is_none());
        child.picker = Some(PendingPicker { slot: 0, result: Box::pin(std::future::ready(None)) });
        app.handle_root_events(&ctx);
        assert!(app.windows[0].slots[0].is_none());
        assert!(app.windows[0].slots[1].is_some());
        assert!(app.windows[0].picker.is_none());
    }

    #[test]
    fn picker_completion_preserves_a_comparison_opened_while_the_sheet_was_pending() {
        let ctx = egui::Context::default();
        let mut app = app();
        root_ui(&mut app).open_demo(false);
        app.root.as_mut().unwrap().picker = Some(PendingPicker {
            slot: 0,
            result: Box::pin(std::future::ready(Some(PathBuf::from("samples/left/src/util/format.ts")))),
        });
        app.handle_root_events(&ctx);
        assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Folder(_)));
        assert!(app.root.as_ref().unwrap().picker.is_none());
        assert_eq!(app.requests.len(), 1);
        assert!(app.requests[0].slot.is_none());
        app.handle_root_events(&ctx);
        assert_eq!(app.windows.len(), 1);
        assert!(app.windows[0].slots[0].is_some());
    }

    #[test]
    fn edited_merge_demo_closes_and_goes_home_without_saving() {
        let ctx = egui::Context::default();
        for home in [false, true] {
            let mut app = app();
            root_ui(&mut app).open_demo(true);
            if let Screen::Merge(m) = &mut app.root.as_mut().unwrap().screen {
                assert!(m.demo);
                m.unsaved = true;
                assert!(!m.needs_save());
                assert!(!m.save(false));
                assert!(m.message.is_none());
            }
            let mut view = root_ui(&mut app);
            if home { view.go_home(); } else { view.request_close(); }
            assert!(!view.window.close_dialog);
            assert!(if home { matches!(view.window.screen, Screen::Welcome) } else { view.window.closed });
        }
        let mut app = app();
        root_ui(&mut app).open_demo(true);
        if let Screen::Merge(m) = &mut app.root.as_mut().unwrap().screen { m.unsaved = true; }
        app.menu_action(platform::MenuAction::Quit, &ctx);
        assert!(app.root.as_ref().unwrap().closed);
        assert!(!app.root.as_ref().unwrap().close_dialog);
    }

    #[test]
    fn native_quit_protects_unsaved_merge_and_replaces_preferences_with_confirmation() {
        let ctx = egui::Context::default();
        let mut app = app();
        root_ui(&mut app).open_demo(true);
        if let Screen::Merge(merge) = &mut app.root.as_mut().unwrap().screen {
            merge.demo = false; merge.unsaved = true;
        }
        app.menu_action(platform::MenuAction::Preferences, &ctx);
        app.route_incoming(Incoming { entries: Vec::new(), slot: None }, &ctx);
        app.menu_action(platform::MenuAction::Quit, &ctx);
        app.remove_closed_windows(&ctx);
        let root = app.root.as_ref().unwrap();
        assert!(!root.closed && root.close_dialog);
        assert!(app.global_windows.iter().all(|w| !w.open));
        assert!(matches!(&root.screen, Screen::Merge(merge) if merge.unsaved));
        assert!(app.windows.is_empty());
    }

    #[test]
    fn native_menu_targets_focused_child_and_works_after_root_closes() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(Incoming { entries: Vec::new(), slot: None }, &ctx);
        let child = app.windows[0].id;
        app.focused = child;
        app.menu_action(platform::MenuAction::About, &ctx);
        app.menu_action(platform::MenuAction::Preferences, &ctx);
        app.menu_action(platform::MenuAction::Shortcuts, &ctx);
        assert!(!app.root.as_ref().unwrap().about_open);
        assert!(app.global_windows.iter().all(|w| w.open));
        assert!(app.windows[0].about_open);
        root_ui(&mut app).request_close();
        app.remove_closed_windows(&ctx);
        assert!(app.root.is_none());
        app.menu_action(platform::MenuAction::NewWindow, &ctx);
        assert_eq!(app.windows.len(), 2);
        app.focused = child;
        app.menu_action(platform::MenuAction::CloseWindow, &ctx);
        app.remove_closed_windows(&ctx);
        assert_eq!(app.windows.len(), 1);
        assert_ne!(app.windows[0].id, child);
    }

    #[test]
    fn global_windows_open_without_comparisons_and_close_independently() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.root = None;
        for action in [platform::MenuAction::Preferences, platform::MenuAction::Shortcuts] {
            app.menu_action(action, &ctx);
            app.menu_action(action, &ctx);
        }
        assert!(app.root.is_none() && app.windows.is_empty());
        assert!(app.global_windows.iter().all(|w| w.open && w.center));
        app.focused = GlobalKind::Preferences.id();
        app.menu_action(platform::MenuAction::CloseWindow, &ctx);
        assert!(!app.global_windows[0].open && app.global_windows[1].open);
        app.route_incoming(folders(), &ctx);
        root_ui(&mut app).request_close();
        app.remove_closed_windows(&ctx);
        assert!(app.global_windows[1].open);
        app.menu_action(platform::MenuAction::Preferences, &ctx);
        assert!(app.global_windows[0].open && app.global_windows[0].center);
    }

    #[test]
    fn global_window_contents_fit_and_escape_closes_only_that_window() {
        fn text_bounds(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_bounds(s, label)),
                _ => None,
            }
        }
        for kind in [GlobalKind::Preferences, GlobalKind::Shortcuts] {
            for height in [400.0, kind.size().y] {
                let ctx = egui::Context::default();
                let mut app = app();
                let index = if kind == GlobalKind::Preferences { 0 } else { 1 };
                app.global_windows[index].open = true;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(kind.size().x, height));
                let mut input = egui::RawInput { viewport_id: kind.id(), screen_rect: Some(screen), ..Default::default() };
                input.viewports.insert(kind.id(), egui::ViewportInfo::default());
                for _ in 0..4 {
                    let mut output = ctx.run_ui(input.clone(), |ui| app.global_ui(kind, ui));
                    let close = output.shapes.iter().find_map(|s| text_bounds(&s.shape, "Close")).unwrap();
                    assert!(screen.shrink(12.0).contains_rect(close), "footer outside global window: {close:?}");
                    output.textures_delta.clear();
                }
                input.events.push(egui::Event::Key { key: Key::Escape, physical_key: None,
                    pressed: true, repeat: false, modifiers: Modifiers::NONE });
                let mut output = ctx.run_ui(input, |ui| app.global_ui(kind, ui));
                output.textures_delta.clear();
                assert!(!app.global_windows[index].open);
                assert!(!app.root.as_ref().unwrap().closed);
            }
        }
    }

    #[test]
    fn information_overlays_fit_small_windows_in_both_themes() {
        for dark in [false, true] {
            for size in [vec2(640.0, 400.0), vec2(880.0, 620.0)] {
                for overlay in ["about", "shortcuts", "preferences", "preferences-checking", "unsaved"] {
                    let ctx = egui::Context::default();
                    install_fonts(&ctx);
                    theme::install(&ctx);
                    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let mut bounds = egui::Rect::NOTHING;
                    for _ in 0..6 {
                        let mut output = ctx.run_ui(egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        }, |ui| {
                            bounds = information_modal(ui.ctx(), "fit-test").show(ui.ctx(), |ui| {
                                match overlay {
                                    "shortcuts" => help_shortcuts(ui),
                                    "preferences" => { preferences_contents(ui, &mut Settings::default(), &crate::update::State::Idle); }
                                    "preferences-checking" => { preferences_contents(ui, &mut Settings::default(), &crate::update::State::Checking); }
                                    "unsaved" => { unsaved_merge_contents(ui, "merged.rs", Some("No output file to save to")); }
                                    _ => about_contents(ui),
                                }
                                if overlay != "unsaved" { information_footer(ui); }
                            }).response.rect;
                        });
                        output.textures_delta.clear();
                    }
                    assert!(screen.shrink(12.0).contains_rect(bounds),
                        "overlay does not fit: dark={dark}, overlay={overlay}, size={size:?}, bounds={bounds:?}");
                }
            }
        }
    }

    #[test]
    fn unsaved_warning_buttons_preserve_their_close_guards() {
        fn button_center(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| button_center(s, label)),
                _ => None,
            }
        }
        for label in ["Save", "Cancel", "Discard"] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut app = app();
            root_ui(&mut app).open_demo(true);
            if let Screen::Merge(m) = &mut app.root.as_mut().unwrap().screen { m.demo = false; m.unsaved = true; }
            root_ui(&mut app).request_close();
            let mut center = None;
            for _ in 0..3 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| root_ui(&mut app).close_dialog_ui(ui.ctx()));
                center = output.shapes.iter().find_map(|s| button_center(&s.shape, label));
                output.textures_delta.clear();
            }
            let center = center.expect("warning action button");
            for pressed in [true, false] {
                let mut output = ctx.run_ui(egui::RawInput {
                    events: vec![egui::Event::PointerMoved(center), egui::Event::PointerButton {
                        pos: center, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                    }], ..Default::default()
                }, |ui| root_ui(&mut app).close_dialog_ui(ui.ctx()));
                output.textures_delta.clear();
            }
            let window = app.root.as_ref().unwrap();
            let Screen::Merge(m) = &window.screen else { panic!("merge was replaced") };
            match label {
                "Save" => {
                    // The demo has no output path: failed saves must keep the warning open.
                    assert!(window.close_dialog && m.unsaved && !window.closed);
                    assert!(m.message.as_ref().is_some_and(|(_, error)| *error));
                }
                "Cancel" => assert!(!window.close_dialog && window.close_cancelled && m.unsaved && !window.closed),
                _ => assert!(!window.close_dialog && !m.unsaved && window.closed),
            }
        }
    }

    #[test]
    fn about_overlay_shows_running_version_and_escape_closes_it() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.root.as_mut().unwrap().about_open = true;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| root_ui(&mut app).ui(ui));
            output.textures_delta.clear();
        }
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            root_ui(&mut app).ui(ui);
        });
        output.textures_delta.clear();
        fn texts(shape: &egui::epaint::Shape, labels: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => labels.push(text.galley.text().to_owned()),
                egui::epaint::Shape::Vec(shapes) => for shape in shapes { texts(shape, labels); },
                _ => {}
            }
        }
        let mut labels = Vec::new();
        for shape in output.shapes { texts(&shape.shape, &mut labels); }
        assert!(labels.iter().any(|text| text == "About MiniDiff"));
        assert!(labels.iter().any(|text| text == &format!("Version {}", crate::update::CURRENT)));
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::Escape, physical_key: None, pressed: true, repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            root_ui(&mut app).ui(ui);
        });
        output.textures_delta.clear();
        assert!(!app.root.as_ref().unwrap().about_open);
    }

    #[test]
    fn drops_preserve_existing_comparisons() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(folders(), &ctx);
        root_ui(&mut app).open_incoming(folders());
        assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Folder(_)));
        assert_eq!(app.requests.len(), 1);
        for incoming in std::mem::take(&mut app.requests) {
            app.route_incoming(incoming, &ctx);
        }
        assert_eq!(app.windows.len(), 1);
        assert!(matches!(app.windows[0].screen, Screen::Folder(_)));
        // Closing the original comparison leaves the second native window alive.
        root_ui(&mut app).request_close();
        app.remove_closed_windows(&ctx);
        assert!(app.root.is_none());
        assert_eq!(app.windows.len(), 1);
        app.route_incoming(folders(), &ctx);
        assert_eq!(app.windows.len(), 2);
    }

    #[test]
    fn new_window_and_separate_single_drops_use_start_screens() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(
            Incoming {
                entries: Vec::new(),
                slot: None,
            },
            &ctx,
        );
        assert_eq!(app.windows.len(), 1);
        assert!(app.root.as_ref().unwrap().accepts_incoming());
        assert!(app.windows[0].accepts_incoming());
        let (a, b) = crate::demo::folders();
        app.route_incoming(
            Incoming {
                entries: vec![a],
                slot: None,
            },
            &ctx,
        );
        assert!(app.root.as_ref().unwrap().slots[0].is_some());
        app.route_incoming(
            Incoming {
                entries: vec![b],
                slot: None,
            },
            &ctx,
        );
        assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Folder(_)));
        app.route_incoming(folders(), &ctx);
        assert_eq!(app.windows.len(), 1);
        assert!(matches!(app.windows[0].screen, Screen::Folder(_)));
    }

    #[test]
    fn windowless_app_reopens_and_only_explicit_quit_exits() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.exit_when_windowless = false;
        let mut input = egui::RawInput::default();
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        for _ in 0..2 {
            let output = ctx.run_logic(&input, |ctx| app.handle_root_events(ctx));
            let commands = &output.viewport_commands[&egui::ViewportId::ROOT];
            assert!(commands.contains(&ViewportCommand::CancelClose));
            assert!(!commands.contains(&ViewportCommand::Close));
            assert!(app.root.is_none() && app.windows.is_empty());
        }
        for action in [platform::MenuAction::Reopen, platform::MenuAction::NewWindow, platform::MenuAction::About] {
            app.menu_action(action, &ctx);
            assert!(app.root.is_some() && app.windows.is_empty());
            root_ui(&mut app).request_close();
            app.remove_closed_windows(&ctx);
        }
        app.route_incoming(folders(), &ctx);
        assert!(matches!(app.root.as_ref().unwrap().screen, Screen::Folder(_)));
        app.route_incoming(folders(), &ctx);
        root_ui(&mut app).request_close();
        app.remove_closed_windows(&ctx);
        app.windows[0].closed = true;
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| app.remove_closed_windows(ctx));
        assert!(!output.viewport_commands.get(&egui::ViewportId::ROOT).is_some_and(|c| c.contains(&ViewportCommand::Close)));
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.menu_action(platform::MenuAction::Quit, ctx);
            app.remove_closed_windows(ctx);
        });
        assert!(output.viewport_commands[&egui::ViewportId::ROOT].contains(&ViewportCommand::Close));
    }

    #[test]
    fn cancelling_unsaved_quit_preserves_app_but_tool_sessions_exit() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.exit_when_windowless = false;
        root_ui(&mut app).open_demo(true);
        if let Screen::Merge(m) = &mut app.root.as_mut().unwrap().screen { m.demo = false; m.unsaved = true; }
        app.menu_action(platform::MenuAction::Quit, &ctx);
        assert!(app.quit_requested && app.root.as_ref().unwrap().close_dialog);
        app.root.as_mut().unwrap().close_cancelled = true;
        app.root.as_mut().unwrap().close_dialog = false;
        app.remove_closed_windows(&ctx);
        assert!(!app.quit_requested);
        if let Screen::Merge(m) = &mut app.root.as_mut().unwrap().screen { m.unsaved = false; }
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            root_ui(&mut app).request_close();
            app.remove_closed_windows(ctx);
        });
        assert!(!output.viewport_commands[&egui::ViewportId::ROOT].contains(&ViewportCommand::Close));
        app.exit_when_windowless = true;
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| app.remove_closed_windows(ctx));
        assert!(output.viewport_commands[&egui::ViewportId::ROOT].contains(&ViewportCommand::Close));
    }

    #[test]
    fn root_close_events_are_cancelled_while_other_windows_remain() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(folders(), &ctx);
        app.route_incoming(folders(), &ctx);
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        for _ in 0..2 {
            let output = ctx.run_logic(&input, |ctx| app.handle_root_events(ctx));
            assert!(output.viewport_commands[&egui::ViewportId::ROOT].contains(&ViewportCommand::CancelClose));
            app.remove_closed_windows(&ctx);
            assert!(app.root.is_none());
            assert_eq!(app.windows.len(), 1);
        }
    }

    #[test]
    fn dropping_and_closing_preserve_unsaved_merge() {
        let ctx = egui::Context::default();
        let mut app = app();
        let mut view = root_ui(&mut app);
        view.open_demo(true);
        if let Screen::Merge(m) = &mut view.window.screen {
            m.demo = false; m.unsaved = true;
        }
        view.open_incoming(folders());
        view.request_close();
        assert!(matches!(&view.window.screen, Screen::Merge(m) if m.unsaved));
        assert!(view.window.close_dialog);
        assert!(!view.window.closed);
        assert_eq!(view.requests.len(), 1);
        // A confirmed discard closes only this window rather than going home.
        if let Screen::Merge(m) = &mut view.window.screen {
            m.unsaved = false;
        }
        view.finish_merge(&ctx);
        assert!(view.window.closed);
    }

    #[test]
    fn background_child_close_events_are_handled_without_painting() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(folders(), &ctx);
        app.route_incoming(folders(), &ctx);
        let id = app.windows[0].id;
        let mut input = egui::RawInput::default();
        let info = input.viewports.entry(id).or_default();
        info.occluded = Some(true);
        info.events.push(egui::ViewportEvent::Close);
        let output = ctx.run_logic(&input, |ctx| app.handle_root_events(ctx));
        assert!(app.windows.is_empty());
        assert!(app.root.is_some());
        assert!(output.viewport_commands[&id].contains(&ViewportCommand::Visible(false)));
    }

    #[test]
    fn background_child_merge_close_focuses_unsaved_confirmation() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.route_incoming(folders(), &ctx);
        app.route_incoming(folders(), &ctx);
        root_ui(&mut app).open_demo(true);
        std::mem::swap(&mut app.root.as_mut().unwrap().screen, &mut app.windows[0].screen);
        if let Screen::Merge(m) = &mut app.windows[0].screen {
            m.demo = false; m.unsaved = true;
        }
        let id = app.windows[0].id;
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(id)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let output = ctx.run_logic(&input, |ctx| app.handle_root_events(ctx));
        assert!(matches!(&app.windows[0].screen, Screen::Merge(m) if m.unsaved));
        assert!(app.windows[0].close_dialog);
        assert!(output.viewport_commands[&id].contains(&ViewportCommand::Focus));
    }
}
