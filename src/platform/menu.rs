//! One AppKit menu bar for all native windows. Actions enter the app's logic inbox.
use super::{MenuAction, push_menu};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{AnyThread, class, define_class, msg_send, sel};
use std::ffi::CString;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MiniDiffMenuHandler"]
    struct MenuHandler;
    impl MenuHandler {
        #[unsafe(method(menuAction:))]
        fn menu_action(&self, item: *mut AnyObject) {
            let tag: isize = unsafe { msg_send![item, tag] };
            let action = match tag {
                1 => MenuAction::About, 2 => MenuAction::Preferences,
                3 => MenuAction::CheckUpdates, 4 => MenuAction::NewWindow,
                5 => MenuAction::CloseWindow, 6 => MenuAction::Home,
                7 => MenuAction::Reload, 8 => MenuAction::Swap,
                9 => MenuAction::Shortcuts, 10 => MenuAction::Quit,
                _ => return,
            };
            push_menu(action);
        }
    }
);

unsafe fn string(text: &str) -> *mut AnyObject {
    let text = CString::new(text).expect("menu title");
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

unsafe fn menu(title: &str) -> Retained<AnyObject> {
    unsafe {
        let allocated: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let initialized: *mut AnyObject = msg_send![allocated, initWithTitle: string(title)];
        Retained::from_raw(initialized).expect("NSMenu initialization")
    }
}

unsafe fn item(
    menu: &AnyObject,
    title: &str,
    key: &str,
    action: Option<Sel>,
    target: *const AnyObject,
    tag: isize,
    modifiers: usize,
) -> Retained<AnyObject> {
    unsafe {
        let allocated: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let initialized: *mut AnyObject = msg_send![allocated,
            initWithTitle: string(title), action: action, keyEquivalent: string(key)];
        let item = Retained::from_raw(initialized).expect("NSMenuItem initialization");
        let _: () = msg_send![&*item, setTarget: target];
        let _: () = msg_send![&*item, setTag: tag];
        let _: () = msg_send![&*item, setKeyEquivalentModifierMask: modifiers];
        let _: () = msg_send![menu, addItem: &*item];
        item
    }
}

unsafe fn separator(menu: &AnyObject) {
    unsafe {
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: item];
    }
}

/// Call once on the main thread after winit has created its default menu.
pub fn install() {
    unsafe {
        let handler: Retained<MenuHandler> = msg_send![MenuHandler::alloc(), init];
        let target = (&*handler as *const MenuHandler).cast::<AnyObject>();
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let main = menu("MiniDiff");
        let command = 1usize << 20;
        let shift = 1usize << 17;
        let option = 1usize << 19;
        for (title, entries) in [
            (
                "MiniDiff",
                vec![
                    ("About MiniDiff", "", 1),
                    ("", "", 0),
                    ("Preferences…", ",", 2),
                    ("Check for Updates…", "", 3),
                ],
            ),
            (
                "File",
                vec![
                    ("New Window", "n", 4),
                    ("Close Window", "w", 5),
                    ("", "", 0),
                    ("Start Screen", "o", 6),
                    ("Reload", "r", 7),
                    ("Swap Sides", "s", 8),
                ],
            ),
            ("Window", vec![]),
            ("Help", vec![("Keyboard Shortcuts", "", 9)]),
        ] {
            let submenu = menu(title);
            let parent = item(&main, title, "", None, std::ptr::null(), 0, 0);
            let _: () = msg_send![&*parent, setSubmenu: &*submenu];
            for (label, key, tag) in entries {
                if tag == 0 {
                    separator(&submenu);
                    continue;
                }
                item(
                    &submenu,
                    label,
                    key,
                    Some(sel!(menuAction:)),
                    target,
                    tag,
                    if tag == 8 { command | shift } else { command },
                );
            }
            if title == "MiniDiff" {
                separator(&submenu);
                item(
                    &submenu,
                    "Hide MiniDiff",
                    "h",
                    Some(sel!(hide:)),
                    app,
                    0,
                    command,
                );
                item(
                    &submenu,
                    "Hide Others",
                    "h",
                    Some(sel!(hideOtherApplications:)),
                    app,
                    0,
                    command | option,
                );
                item(
                    &submenu,
                    "Show All",
                    "",
                    Some(sel!(unhideAllApplications:)),
                    app,
                    0,
                    0,
                );
                separator(&submenu);
                item(
                    &submenu,
                    "Quit MiniDiff",
                    "q",
                    Some(sel!(menuAction:)),
                    target,
                    10,
                    command,
                );
            }
            if title == "Window" {
                item(
                    &submenu,
                    "Minimize",
                    "m",
                    Some(sel!(performMiniaturize:)),
                    std::ptr::null(),
                    0,
                    command,
                );
                item(
                    &submenu,
                    "Zoom",
                    "",
                    Some(sel!(performZoom:)),
                    std::ptr::null(),
                    0,
                    0,
                );
                let _: () = msg_send![app, setWindowsMenu: &*submenu];
            }
            if title == "Help" {
                let _: () = msg_send![app, setHelpMenu: &*submenu];
            }
        }
        let _: () = msg_send![app, setMainMenu: &*main];
        // NSMenuItem targets are weak; the process owns this one handler.
        std::mem::forget(handler);
    }
}
