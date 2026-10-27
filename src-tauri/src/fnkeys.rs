//! "F-keys without fn" — while the app is focused, flip the system's
//! HIDFKeyMode parameter (the "Use F1, F2, etc. keys as standard function
//! keys" setting) and restore it on blur/quit. Same mechanism as Fluor;
//! needs no Accessibility permission. When IOKit says no (odd hardware,
//! future OS lockdown), everything degrades to the ⌘-equivalents.

use crate::state::AppState;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

type IoObject = u32;
type IoConnect = u32;

const KIOHID_PARAM_CONNECT_TYPE: u32 = 1;
const FKEY_MODE_KEY: &str = "HIDFKeyMode";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(master_port: u32, matching: *mut c_void) -> IoObject;
    fn IOServiceOpen(
        service: IoObject,
        owning_task: u32,
        connect_type: u32,
        connect: *mut IoConnect,
    ) -> i32;
    fn IOServiceClose(connect: IoConnect) -> i32;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IOHIDSetCFTypeParameter(connect: IoConnect, key: CFStringRef, value: CFTypeRef) -> i32;
    fn IOHIDCopyCFTypeParameter(
        connect: IoConnect,
        key: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
}

pub struct FnKeys {
    connect: IoConnect,
    /// The user's own fn mode, remembered while we hold ours.
    saved: Mutex<Option<i64>>,
}

// The io_connect_t handle is just a mach port name; calls are thread-safe.
unsafe impl Send for FnKeys {}
unsafe impl Sync for FnKeys {}

impl FnKeys {
    pub fn open() -> Option<Self> {
        unsafe {
            let matching = IOServiceMatching(c"IOHIDSystem".as_ptr());
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return None;
            }
            let mut connect: IoConnect = 0;
            // libc deprecates its mach_task_self in favor of the mach2 crate;
            // one extern avoids pulling a whole crate for one symbol.
            unsafe extern "C" {
                static mach_task_self_: u32;
            }
            let kr = IOServiceOpen(
                service,
                mach_task_self_,
                KIOHID_PARAM_CONNECT_TYPE,
                &mut connect,
            );
            IOObjectRelease(service);
            if kr != 0 || connect == 0 {
                return None;
            }
            Some(Self {
                connect,
                saved: Mutex::new(None),
            })
        }
    }

    fn get(&self) -> Option<i64> {
        let key = CFString::from_static_string(FKEY_MODE_KEY);
        let mut value: CFTypeRef = std::ptr::null();
        let kr = unsafe {
            IOHIDCopyCFTypeParameter(self.connect, key.as_concrete_TypeRef(), &mut value)
        };
        if kr != 0 || value.is_null() {
            return None;
        }
        let number = unsafe { CFNumber::wrap_under_create_rule(value as _) };
        number.to_i64()
    }

    fn set(&self, mode: i64) {
        let key = CFString::from_static_string(FKEY_MODE_KEY);
        let value = CFNumber::from(mode);
        unsafe {
            IOHIDSetCFTypeParameter(
                self.connect,
                key.as_concrete_TypeRef(),
                value.as_CFTypeRef(),
            );
        }
    }

    /// App gained focus: remember the user's mode once, then force F-keys on.
    pub fn acquire(&self) {
        let mut saved = self.saved.lock().unwrap();
        if saved.is_none() {
            *saved = Some(self.get().unwrap_or(0));
        }
        self.set(1);
    }

    /// App lost focus or is quitting: hand the user's mode back.
    pub fn release(&self) {
        if let Some(mode) = self.saved.lock().unwrap().take() {
            self.set(mode);
        }
    }
}

impl Drop for FnKeys {
    fn drop(&mut self) {
        self.release();
        unsafe {
            IOServiceClose(self.connect);
        }
    }
}

/// Managed state: None when the IOHIDSystem connection could not be opened.
pub struct FnKeysState(pub Option<FnKeys>);

/// Toggle the setting from the UI and apply it right away (the window is
/// focused while the user clicks).
#[tauri::command]
pub fn fkeys_set(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    {
        let mut config = state.config.lock().unwrap();
        config.fkeys_without_fn = enabled;
        config.save();
    }
    if let Some(fk) = &app.state::<FnKeysState>().0 {
        if enabled {
            fk.acquire();
        } else {
            fk.release();
        }
    }
}

/// Whether the IOKit switch is available at all (UI grays the toggle out).
#[tauri::command]
pub fn fkeys_available(app: AppHandle) -> bool {
    app.state::<FnKeysState>().0.is_some()
}
