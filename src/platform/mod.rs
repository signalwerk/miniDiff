//! Platform glue: how files reach the app outside of egui's own events.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub mod menu;

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::source::Entry;

/// Items opened by the OS (dock icon drop, `open -a`).
#[derive(Debug)]
pub struct Incoming {
    pub entries: Vec<Entry>,
    /// Put the first entry into this compare slot (0 = A, 1 = B) instead of auto-routing.
    pub slot: Option<usize>,
}

static INBOX: Mutex<Vec<Incoming>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);
static MENU_INBOX: Mutex<Vec<MenuAction>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug)]
pub enum MenuAction {
    About, Preferences, CheckUpdates, NewWindow, CloseWindow, Home, Reload, Swap, Shortcuts, Quit,
}

pub fn push_menu(action: MenuAction) {
    MENU_INBOX.lock().unwrap().push(action);
    if let Some(ctx) = CTX.get() { ctx.request_repaint_of(egui::ViewportId::ROOT); }
}

pub fn take_menu() -> Vec<MenuAction> {
    std::mem::take(&mut *MENU_INBOX.lock().unwrap())
}

pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

pub fn push(incoming: Incoming) {
    INBOX.lock().unwrap().push(incoming);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

pub fn take() -> Vec<Incoming> {
    std::mem::take(&mut *INBOX.lock().unwrap())
}

pub fn set_exit_code(code: i32) {
    EXIT_CODE.store(code, Ordering::Relaxed);
}
pub fn exit_code() -> i32 {
    EXIT_CODE.load(Ordering::Relaxed)
}
