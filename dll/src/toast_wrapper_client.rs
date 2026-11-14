//! Client for toast_wrapper.dll (copy this file into your mod, then `mod toast_wrapper_client;`). It finds the DLL's
//! exports when called, so load order doesn't matter, and does nothing if toast_wrapper.dll isn't loaded.
//!
//! ```ignore
//! toast_wrapper_client::show("Saved");            // the game's own timing
//! toast_wrapper_client::show_for("Saved", 3.0);   // 3 s on screen (needs toast_wrapper 0.4+, API 3)
//! ```

#![allow(dead_code)]

use std::sync::OnceLock;

type ShowFn = unsafe extern "C" fn(*const u8, usize) -> bool;
type ShowForFn = unsafe extern "C" fn(*const u8, usize, f32) -> bool;
type VersionFn = extern "C" fn() -> u32;

struct Api {
    show: ShowFn,
    /// API 3 and later.
    show_for: Option<ShowForFn>,
    version: VersionFn,
}

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
}

/// The oldest API version this client works with (exports never change signature; newer DLLs only add exports).
const NEEDED_VERSION: u32 = 2;
/// "toast_wrapper.dll", UTF-16, null-terminated.
const DLL_NAME: [u16; 18] = {
    let ascii = *b"toast_wrapper.dll\0";
    let mut wide = [0u16; 18];
    let mut i = 0;
    while i < ascii.len() {
        wide[i] = ascii[i] as u16;
        i += 1;
    }
    wide
};

static API: OnceLock<Api> = OnceLock::new();

/// The DLL's exports, looked up until found once (natives are never unloaded, so the cache stays valid).
fn api() -> Option<&'static Api> {
    if let Some(api) = API.get() {
        return Some(api);
    }
    let module = unsafe { GetModuleHandleW(DLL_NAME.as_ptr()) };
    if module == 0 {
        return None;
    }
    let find = |name: &[u8]| {
        let addr = unsafe { GetProcAddress(module, name.as_ptr()) };
        (addr != 0).then_some(addr)
    };
    let version: VersionFn = unsafe { std::mem::transmute(find(b"toast_wrapper_api_version\0")?) };
    if version() < NEEDED_VERSION {
        return None;
    }
    let show: ShowFn = unsafe { std::mem::transmute(find(b"toast_wrapper_show\0")?) };
    let show_for = find(b"toast_wrapper_show_for\0").map(|addr| unsafe { std::mem::transmute::<usize, ShowForFn>(addr) });
    Some(API.get_or_init(|| Api { show, show_for, version }))
}

/// Queues a text for the game's announce banner, with the game's own timing. False if toast_wrapper.dll isn't
/// loaded or refused it.
pub fn show(text: &str) -> bool {
    api().is_some_and(|api| unsafe { (api.show)(text.as_ptr(), text.len()) })
}

/// Queues a text that stays on screen for `seconds` (longer or shorter than the game's timing). False if
/// toast_wrapper.dll isn't loaded, is older than API 3, or refused it.
pub fn show_for(text: &str, seconds: f32) -> bool {
    api().and_then(|api| api.show_for).is_some_and(|show_for| unsafe { show_for(text.as_ptr(), text.len(), seconds) })
}

/// The DLL's API version, None if it isn't loaded (or older than this client).
pub fn api_version() -> Option<u32> {
    api().map(|api| (api.version)())
}
