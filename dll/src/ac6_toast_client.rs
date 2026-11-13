//! Client for ac6_toast.dll (copy this file into your mod, then `mod ac6_toast_client;`). It finds the DLL's
//! exports when called, so load order doesn't matter, and does nothing if ac6_toast.dll isn't loaded.
//!
//! ```ignore
//! if let Some(toast) = ac6_toast_client::show("Recording FXRs", 0.0) { // 0 = until dismissed
//!     toast.update("Recording FXRs: 12 found");
//!     toast.dismiss();
//! }
//! ac6_toast_client::show("Saved", 3.0);
//! ```

#![allow(dead_code)]

use std::sync::OnceLock;

type ShowFn = unsafe extern "C" fn(*const u8, usize, f32) -> u64;
type UpdateFn = unsafe extern "C" fn(u64, *const u8, usize) -> bool;
type DismissFn = unsafe extern "C" fn(u64) -> bool;
type VersionFn = extern "C" fn() -> u32;

struct Api {
    show: ShowFn,
    update: UpdateFn,
    dismiss: DismissFn,
    version: VersionFn,
}

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
}

/// The API version this client was written for (exports never change signature; newer DLLs only add exports).
const NEEDED_VERSION: u32 = 1;
/// "ac6_toast.dll", UTF-16, null-terminated.
const DLL_NAME: [u16; 14] = {
    let ascii = *b"ac6_toast.dll\0";
    let mut wide = [0u16; 14];
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
    let version: VersionFn = unsafe { std::mem::transmute(find(b"ac6_toast_api_version\0")?) };
    if version() < NEEDED_VERSION {
        return None;
    }
    let api = unsafe {
        Api {
            show: std::mem::transmute::<usize, ShowFn>(find(b"ac6_toast_show\0")?),
            update: std::mem::transmute::<usize, UpdateFn>(find(b"ac6_toast_update\0")?),
            dismiss: std::mem::transmute::<usize, DismissFn>(find(b"ac6_toast_dismiss\0")?),
            version,
        }
    };
    Some(API.get_or_init(|| api))
}

/// A shown or queued toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastId(pub u64);

/// Queues a toast for `seconds` (0 or less: until dismissed). None if ac6_toast.dll isn't loaded or its queue is full.
pub fn show(text: &str, seconds: f32) -> Option<ToastId> {
    let api = api()?;
    let id = unsafe { (api.show)(text.as_ptr(), text.len(), seconds) };
    (id != 0).then_some(ToastId(id))
}

impl ToastId {
    /// Replaces the text (in place if it's on screen). False if the toast is finished.
    pub fn update(self, text: &str) -> bool {
        api().is_some_and(|api| unsafe { (api.update)(self.0, text.as_ptr(), text.len()) })
    }

    /// Ends the toast (it fades out) or drops it from the queue.
    pub fn dismiss(self) -> bool {
        api().is_some_and(|api| unsafe { (api.dismiss)(self.0) })
    }
}

/// The DLL's API version, None if it isn't loaded.
pub fn api_version() -> Option<u32> {
    api().map(|api| (api.version)())
}
