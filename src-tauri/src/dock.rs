//! "New Window" in the Dock icon's right-click menu. Neither Tauri nor muda
//! exposes `applicationDockMenu:`, so this teaches the existing app delegate
//! that method at runtime and serves a static NSMenu from it.

use crate::state::AppState;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::ffi::CString;
use std::sync::OnceLock;
use tauri::Manager;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
/// Raw +1 NSMenu pointer; the menu (and its target) live for the whole app.
static MENU: OnceLock<usize> = OnceLock::new();

fn ns_string(s: &str) -> *mut AnyObject {
    let c = CString::new(s).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

extern "C-unwind" fn dock_menu(
    _this: &AnyObject,
    _sel: Sel,
    _sender: *mut AnyObject,
) -> *mut AnyObject {
    MENU.get()
        .map(|p| *p as *mut AnyObject)
        .unwrap_or(std::ptr::null_mut())
}

extern "C-unwind" fn do_new_window(_this: &AnyObject, _sel: Sel, _sender: *mut AnyObject) {
    // Menu actions arrive on the main thread, where windows may be built.
    if let Some(app) = APP.get() {
        let _ = crate::commands::system::open_new_window(app);
    }
}

/// Install the Dock menu. Call once from setup, on the main thread.
pub fn install(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
    let lang = app
        .state::<AppState>()
        .config
        .lock()
        .unwrap()
        .language
        .clone();
    let title = if lang.as_deref().is_some_and(|l| l.starts_with("zh")) {
        "開新視窗"
    } else {
        "New Window"
    };

    unsafe {
        // Action target: a minimal NSObject subclass answering newWindow:.
        let mut builder = ClassBuilder::new(c"YCDockMenuTarget", class!(NSObject))
            .expect("YCDockMenuTarget registered twice");
        builder.add_method(
            sel!(newWindow:),
            do_new_window as extern "C-unwind" fn(_, _, _),
        );
        let target_cls = builder.register();
        let target: *mut AnyObject = msg_send![target_cls, new];

        let menu: *mut AnyObject = msg_send![class!(NSMenu), new];
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: ns_string(title),
            action: sel!(newWindow:),
            keyEquivalent: ns_string(""),
        ];
        // NSMenuItem.target is weak; `target` is +1 and never released.
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![menu, addItem: item];
        let _ = MENU.set(menu as usize);

        // Add applicationDockMenu: to whatever delegate tao installed.
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![ns_app, delegate];
        if delegate.is_null() {
            return;
        }
        let cls: *const AnyClass = msg_send![delegate, class];
        objc2::ffi::class_addMethod(
            cls.cast_mut(),
            sel!(applicationDockMenu:),
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> *mut AnyObject,
                objc2::runtime::Imp,
            >(dock_menu),
            c"@@:@".as_ptr(),
        );
    }
}
