//! macOS: receive files dropped on the Dock / Finder icon.
//!
//! Finder delivers those as an "open documents" Apple Event (`aevt/odoc`),
//! which winit does not expose. We register our own handler right when the
//! application is about to finish launching (the point Apple recommends for
//! overriding the default handlers), so it works both for launching the app
//! by dropping files and for dropping onto an already running app.

use std::ffi::{CStr, c_char};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{AnyThread, ClassType, class, define_class, msg_send, sel};

use super::{Incoming, push};
use crate::source::Entry;

const fn four_cc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

const K_CORE_EVENT_CLASS: u32 = four_cc(b"aevt");
const K_AE_OPEN_DOCUMENTS: u32 = four_cc(b"odoc");
const KEY_DIRECT_OBJECT: u32 = four_cc(b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MiniDiffOpenHandler"]
    struct OpenHandler;

    impl OpenHandler {
        #[unsafe(method(appWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: *mut AnyObject) {
            unsafe {
                let manager: *mut AnyObject = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
                let _: () = msg_send![
                    manager,
                    setEventHandler: self,
                    andSelector: sel!(handleOpenDocuments:withReplyEvent:),
                    forEventClass: K_CORE_EVENT_CLASS,
                    andEventID: K_AE_OPEN_DOCUMENTS
                ];
            }
        }

        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(&self, event: *mut AnyObject, _reply: *mut AnyObject) {
            let paths = unsafe { paths_from_event(event) };
            if !paths.is_empty() {
                push(Incoming {
                    entries: paths.into_iter().map(Entry::Fs).collect(),
                    slot: None,
                });
            }
        }
    }
);

unsafe fn paths_from_event(event: *mut AnyObject) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if event.is_null() {
        return out;
    }
    unsafe {
        let list: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
        if list.is_null() {
            return out;
        }
        let count: isize = msg_send![list, numberOfItems];
        // A single file may arrive as a plain descriptor rather than a list.
        let items: Vec<*mut AnyObject> = if count == 0 {
            vec![list]
        } else {
            (1..=count).map(|i| msg_send![list, descriptorAtIndex: i]).collect()
        };
        for item in items {
            if item.is_null() {
                continue;
            }
            let url: *mut AnyObject = msg_send![item, fileURLValue];
            if url.is_null() {
                continue;
            }
            let path: *mut AnyObject = msg_send![url, path];
            if path.is_null() {
                continue;
            }
            let utf8: *const c_char = msg_send![path, UTF8String];
            if !utf8.is_null() {
                out.push(PathBuf::from(CStr::from_ptr(utf8).to_string_lossy().into_owned()));
            }
        }
    }
    out
}

/// Must be called before the event loop starts (i.e. before `eframe::run_native`).
pub fn install_open_handler() {
    let _ = OpenHandler::class();
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::alloc(), init] };
    unsafe {
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let name: *mut AnyObject = msg_send![
            class!(NSString),
            stringWithUTF8String: c"NSApplicationWillFinishLaunchingNotification".as_ptr()
        ];
        let selector: Sel = sel!(appWillFinishLaunching:);
        let _: () = msg_send![
            center,
            addObserver: &*handler,
            selector: selector,
            name: name,
            object: std::ptr::null_mut::<AnyObject>()
        ];
    }
    // The notification center and the Apple Event manager do not retain the handler.
    std::mem::forget(handler);
}
