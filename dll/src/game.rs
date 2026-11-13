//! Game addresses, layouts and small memory helpers (Armored Core VI 1.09.1). How they fit together:
//! ../../docs/how-it-works.md.

use std::ffi::c_void;

// ---- menu movies ----------------------------------------------------------------------------------------------------

/// Menu table: 109 entries of 16 bytes {int layer, byte resident (+4), wchar* name (+8)}, filled at startup.
pub const MENU_TABLE: usize = 0x144e28cf0;
pub const MENU_TABLE_COUNT: usize = 0x6d;
/// `[[SCALEFORM_HOLDER] + 8]` = CSScaleformSystem.
const SCALEFORM_HOLDER: usize = 0x144e28818;
/// The system's movie cache (std::map, head node pointer at +0x9c0). Nodes: key chars +0x28 / length +0x30, cached
/// movie wrapper +0x248 (refcount +8, vtable slot 0 destroys it).
const CACHE_MAP_HEAD: usize = 0x9c0;
const NODE_KEY: usize = 0x28;
const NODE_KEY_LEN: usize = 0x30;
pub const NODE_VALUE: usize = 0x248;

/// The in-memory file repository the game's Scaleform file opener reads first: lookup by wchar path -> entry.
pub const REPOSITORY: usize = 0x144e245b0;
pub const REPOSITORY_LOOKUP: usize = 0x14153d3d0;
/// Drops one reference of a repository entry (what the file opener calls after opening from it).
pub const REPOSITORY_RELEASE: usize = 0x1416040c0;
/// (out string object, table entry) -> string object with the menu's wchar path at +8 ("menu:/%s.gfx").
pub const MENU_PATH: usize = 0x14160a5b0;

/// The file opener's `jnz` that skips its disk read when a file isn't in the repository. NOPed = read from disk.
const DISK_FALLBACK_JNZ: usize = 0x1415f427d;
const DISK_FALLBACK_ORIGINAL: [u8; 6] = [0x0f, 0x85, 0xfb, 0x01, 0x00, 0x00];

// ---- self-owned menu windows (vtable 0x14390e498) ---------------------------------------------------------------

/// Per-frame step (vtable slot +0x10): with no controller it opens the window (player, movie, controller).
pub const WINDOW_STEP: usize = 0x140d61af0;
pub const WINDOW_STEP_PROLOGUE: [u8; 8] = [0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56];
/// The window's close: disposes the controller, closes the player, destroys the close check at +0x128.
pub const WINDOW_CLOSE: usize = 0x14641c16c;
pub const WINDOW_CLOSE_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x48, 0x8b, 0xec, 0x48];
pub const WINDOW_NAME: usize = 0x60;
pub const WINDOW_FACTORY: usize = 0xa8;
/// std::function "should this window close" set by the window's creator: inline storage +0xf0, impl pointer +0x128.
pub const WINDOW_CLOSE_CHECK_STORAGE: usize = 0xf0;
pub const WINDOW_CLOSE_CHECK: usize = 0x128;
pub const WINDOW_CONTROLLER: usize = 0x130;
/// vtable of a std::function holding a plain function pointer (no captured state).
pub const FUNCTION_POINTER_FUNCTION: usize = 0x14390e4f0;

// ---- embedded panels (vtable 0x1438e5238) -----------------------------------------------------------------------

pub const PANEL_OPEN: usize = 0x140c8acf0;
pub const PANEL_OPEN_PROLOGUE: [u8; 8] = [0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xc0];
pub const PANEL_CLOSE: usize = 0x140c8aff0;
pub const PANEL_CLOSE_PROLOGUE: [u8; 8] = [0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b];
pub const PANEL_NAME: usize = 0x50;
pub const PANEL_PLAYER: usize = 0x60;
/// Panels whose owners survive mission loads (closed only when returning to the title screen).
pub const PERSISTENT_PANELS: [&str; 2] = ["01_MenuCommonBG", "02_OnlineArena_CommonBG"];

// ---- helpers --------------------------------------------------------------------------------------------------------

unsafe extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
}

pub unsafe fn read<T: Copy>(addr: usize) -> T {
    unsafe { std::ptr::read_unaligned(addr as *const T) }
}

pub unsafe fn write<T: Copy>(addr: usize, value: T) {
    unsafe { std::ptr::write_unaligned(addr as *mut T, value) }
}

pub unsafe fn wide_str(ptr: *const u16, max: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    while len < max && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Name stored behind a wchar pointer at `addr` (window/panel name fields, menu table entries).
pub unsafe fn name_at(addr: usize) -> String {
    unsafe { wide_str(read::<usize>(addr) as *const u16, 128) }
}

pub fn menu_names() -> Vec<String> {
    (0..MENU_TABLE_COUNT).map(|i| unsafe { name_at(MENU_TABLE + i * 16 + 8) }).collect()
}

pub fn scaleform_system() -> usize {
    let holder: usize = unsafe { read(SCALEFORM_HOLDER) };
    if holder == 0 { 0 } else { unsafe { read(holder + 8) } }
}

/// The movie cache node for a menu (MSVC std::map: left +0, parent +8, right +0x10, is-nil byte +0x19).
pub unsafe fn cache_node(system: usize, name: &str) -> Option<usize> {
    let head: usize = unsafe { read(system + CACHE_MAP_HEAD) };
    if head == 0 {
        return None;
    }
    let mut stack = vec![unsafe { read::<usize>(head + 8) }];
    let mut visited = 0;
    while let Some(node) = stack.pop() {
        if node == 0 || node == head || unsafe { read::<u8>(node + 0x19) } != 0 {
            continue;
        }
        visited += 1;
        if visited > 1000 {
            break;
        }
        let key_len: usize = unsafe { read(node + NODE_KEY_LEN) };
        let key = unsafe { wide_str(read::<usize>(node + NODE_KEY) as *const u16, key_len.min(256)) };
        if key == name {
            return Some(node);
        }
        stack.push(unsafe { read(node) });
        stack.push(unsafe { read(node + 0x10) });
    }
    None
}

/// Lets the file opener read menu files from disk when they aren't in the repository (after boot, none are).
pub fn patch_disk_fallback() -> Result<(), String> {
    let site = DISK_FALLBACK_JNZ as *mut u8;
    unsafe {
        let current: [u8; 6] = std::ptr::read(site as *const [u8; 6]);
        if current == [0x90; 6] {
            return Ok(());
        }
        if current != DISK_FALLBACK_ORIGINAL {
            return Err(format!("unexpected bytes at {DISK_FALLBACK_JNZ:#x}: {current:02x?} (not 1.09.1?)"));
        }
        let mut old = 0u32;
        VirtualProtect(site as *mut c_void, 6, 0x40, &mut old);
        std::ptr::write(site as *mut [u8; 6], [0x90; 6]);
        VirtualProtect(site as *mut c_void, 6, old, &mut old);
    }
    Ok(())
}

pub fn check_bytes(addr: usize, expected: &[u8]) -> Result<(), String> {
    let actual = unsafe { std::slice::from_raw_parts(addr as *const u8, expected.len()) };
    if actual == expected {
        Ok(())
    } else {
        Err(format!("code at {addr:#x} doesn't match (not 1.09.1?): {actual:02x?}"))
    }
}
