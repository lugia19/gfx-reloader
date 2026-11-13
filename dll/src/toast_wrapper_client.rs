//! Client for toast_wrapper.dll (copy this file into your mod, then `mod toast_wrapper_client;`). It finds the DLL's
//! exports when called, so load order doesn't matter, and does nothing if toast_wrapper.dll isn't loaded.
//!
//! ```ignore
//! toast_wrapper_client::show("Saved");
//! ```

#![allow(dead_code)]

use std::sync::OnceLock;

type ShowFn = unsafe extern "C" fn(*const u8, usize) -> bool;
type VersionFn = extern "C" fn() -> u32;

struct Api {
    show: ShowFn,
    version: VersionFn,
}

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
}

/// The API version this client was written for (exports never change signature; newer DLLs only add exports).
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
    Some(API.get_or_init(|| Api { show, version }))
}

/// Queues a text for the game's announce banner. False if toast_wrapper.dll isn't loaded or refused it.
pub fn show(text: &str) -> bool {
    api().is_some_and(|api| unsafe { (api.show)(text.as_ptr(), text.len()) })
}

/// The DLL's API version, None if it isn't loaded (or older than this client).
pub fn api_version() -> Option<u32> {
    api().map(|api| (api.version)())
}
