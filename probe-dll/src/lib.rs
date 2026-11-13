//! ui-hot-reload research probe (Armored Core VI 1.09.1, stock exe, me3 0.13.0): the first version of the tool, kept
//! for research. It reloads edited menu movies and logs enough to see what the game does with them. Findings:
//! ..\..\docs\research-notes.md.
//!
//! How a reload works: the disk fallback patch (6 NOPs over the `jnz` at 0x1415f427d) makes the game's Scaleform
//! OpenFile read a file from disk when it isn't in the in-memory file repository (after boot, none are). Releasing
//! a movie's cache entry (CSScaleformSystem+0x9b8 map) makes the next lookup load it again, from disk.
//!
//! Keys (polled every 50 ms; never F6, it doesn't reach the game on lugia19's machine):
//! - F7: compare the package's menu files with the last snapshot; changed files' movies get released at the next
//!   lookup (game thread). Turns the disk fallback on if it isn't.
//! - F8: release every cached menu movie at the next lookup.
//! - F9: dump the movie cache, the player map (+0x9d8) and the active player list (+0x9e8) at the next lookup.
//! - F11: toggle the disk fallback patch.
//!
//! Config `ui_hot_reload.ini`, looked up next to the DLL and in its parent folders:
//!   package = <me3 package folder whose menu\ is watched>
//!   disk_fallback = boot | on_reload     (boot: patch at load, before the game's boot requests)
//!
//! Log: Game\debug-logs\ui_hot_reload_probe.log. Lookups, menu opens (player reused or new), loads, CreateMovie results
//! (flagging an unchanged MovieDef, i.e. Scaleform's library cache), OpenFile with where the file came from
//! (repository / disk / failed), file requests, and Scaleform's own log messages.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use retour::RawDetour;

mod watch;

/// CSMenuMan singleton pointer (ER's CSMenuManImp). +0x10 / +0x748: the bytes the main-menu close check reads.
const CS_MENU_MAN: usize = 0x144e15530;
/// vtable of the std::function implementation that wraps a plain function pointer (no captured state).
const FN_POINTER_FUNCTION_VTABLE: usize = 0x14390e4f0;

// ---- game addresses (1.09.1) --------------------------------------------------------------------------------------

/// Menu movie lookup (CSScaleformSystem*, out wrapper, table entry): cache hit, or load (and cache when resident).
const LOOKUP_ADDR: usize = 0x141609040;
const LOOKUP_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55];
/// Opens a menu (CSScaleformSystem*, table entry, flag) -> player: lookup, then the player from the player map
/// (reused if it exists, else a new 0xe8 one), started once (player+0x78) and added to the active list.
const OPEN_MENU_ADDR: usize = 0x1416069a0;
const OPEN_MENU_PROLOGUE: [u8; 8] = [0x48, 0x8b, 0xc4, 0x55, 0x56, 0x57, 0x48, 0x83];
/// Closes a menu (CSScaleformSystem*, player): the player keeps its table entry copy at +0x20 (name +0x28).
/// Reached through the facade 0x1415f3150 (tail jump), called by the menu windows' close methods.
const CLOSE_MENU_ADDR: usize = 0x141606b10;
const CLOSE_MENU_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55];
const PLAYER_NAME: usize = 0x28;
/// Menu window methods (one class family; vtables 0x1444f2c18 / 0x1444f32a0...). Window: table entry copy at +0x48
/// (name +0x50). Open A stores its player at +0x58 (movie instance +0x8, C++ binding +0xa0 made by the factory at
/// +0x98); open B and close B use +0x60.
const WINDOW_OPEN_A_ADDR: usize = 0x140c8aa60;
const WINDOW_OPEN_A_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x48, 0x83, 0xec, 0x40];
const WINDOW_OPEN_B_ADDR: usize = 0x140c8acf0;
const WINDOW_OPEN_B_PROLOGUE: [u8; 8] = [0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xc0];
const WINDOW_CLOSE_B_ADDR: usize = 0x140c8aff0;
const WINDOW_CLOSE_B_PROLOGUE: [u8; 8] = [0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b];
/// Main-menu window (vtable 0x14390e498; 01_GarageTopMenu, 02_010_EquipTop...): table entry copy +0x58 (name
/// pointer +0x60), player +0x68, movie instance +0x10, controller +0x130, open factory +0xa8, command object +0x128.
/// Slot +0x10 = per-frame step: with no controller and a factory it opens (player, instance, controller); when the
/// controller says it's done it closes. Close 0x14641c16c(window): disposes the controller (+0x130 = 0), closes the
/// player (+0x68 = 0), destroys +0x128; the factory stays, so the next step reopens the window.
const WINDOW_STEP_ADDR: usize = 0x140d61af0;
const WINDOW_STEP_PROLOGUE: [u8; 8] = [0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56];
const WINDOW_CLOSE_ADDR: usize = 0x14641c16c;
const WINDOW_CLOSE_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x48, 0x8b, 0xec, 0x48];
const MAIN_WINDOW_NAME: usize = 0x60;
const MAIN_WINDOW_PLAYER: usize = 0x68;
const MAIN_WINDOW_FACTORY: usize = 0xa8;
const MAIN_WINDOW_COMMAND: usize = 0x128;
const MAIN_WINDOW_CONTROLLER: usize = 0x130;
/// Cache-miss loader (loader, out wrapper, wchar path, ?).
const LOADER_ADDR: usize = 0x1415faa20;
const LOADER_PROLOGUE: [u8; 8] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x55];
/// Scaleform CreateMovie (loader, utf-8 path, flags, arena) -> MovieDef.
const CREATE_MOVIE_ADDR: usize = 0x141a4c6f0;
const CREATE_MOVIE_PROLOGUE: [u8; 8] = [0x45, 0x8b, 0xd0, 0x48, 0x8b, 0xc1, 0x48, 0x85];
/// CSScaleformFileOpener::OpenFile (this, utf-8 url, flags, mode) -> Scaleform File.
const OPEN_FILE_ADDR: usize = 0x1415f40b0;
const OPEN_FILE_PROLOGUE: [u8; 8] = [0x40, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57];
/// Menu file request (system, wchar path): puts the file in the in-memory repository (boot requests all 109).
const REQUEST_ADDR: usize = 0x14160b820;
const REQUEST_PROLOGUE: [u8; 8] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83];
/// Gives a repository file back to CSFile after OpenFile opened from it (only called on the repository path).
const RELEASE_OPENED_ADDR: usize = 0x1416040c0;
const RELEASE_OPENED_PROLOGUE: [u8; 8] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83];

/// OpenFile's gate for the disk fallback after a repository miss: `call 0x14120759a` (always 1) / `test al,al` /
/// this `jnz <return null>`. Six NOPs = fall through to the CSFile disk read (only this call site).
const DISK_FALLBACK_JNZ: usize = 0x1415f427d;
const DISK_FALLBACK_ORIGINAL: [u8; 6] = [0x0f, 0x85, 0xfb, 0x01, 0x00, 0x00];

/// 109 entries of 16 bytes: {int layer, byte resident (+4), wchar* name (+8)}.
const MENU_TABLE: usize = 0x144e28cf0;
const MENU_TABLE_COUNT: usize = 0x6d;

// ---- CSScaleformSystem layout ---------------------------------------------------------------------------------------

const SYSTEM_LOADER: usize = 0x968;
const LOADER_LOG: usize = 0x20; // CS::CSScaleformLog object at loader+0x20
const SYSTEM_FILE_COUNT: usize = 0x980; // requested files still held (+0x970/+0x978)
/// MSVC std::map heads (map object 8 bytes before): movie cache (value = wrapper) and players (value = player).
const CACHE_MAP_HEAD: usize = 0x9c0;
const PLAYER_MAP_HEAD: usize = 0x9d8;
/// Active players: fixed vector of {player, int open count} (16 bytes), count at +0xdf0, max 64.
const ACTIVE_LIST: usize = 0x9e8;
const ACTIVE_COUNT: usize = 0xdf0;
/// Map nodes: left +0, parent +8, right +0x10, is-nil byte +0x19, key chars +0x28 / length +0x30, value +0x248.
const NODE_KEY: usize = 0x28;
const NODE_KEY_LEN: usize = 0x30;
const NODE_VALUE: usize = 0x248;
/// Movie wrapper: 0x38 bytes, refcount +8, vtable slot 0 destroys. Player: 0xe8 bytes, started flag +0x78.
const WRAPPER_SIZE: usize = 0x38;
const PLAYER_SIZE: usize = 0xe8;
const PLAYER_STARTED: usize = 0x78;

// ---- keys ------------------------------------------------------------------------------------------------------------

const VK_F5: i32 = 0x74;
const VK_F7: i32 = 0x76;
const VK_F8: i32 = 0x77;
const VK_F9: i32 = 0x78;
const VK_F10: i32 = 0x79;
const VK_F11: i32 = 0x7a;

// ---- state -----------------------------------------------------------------------------------------------------------

type LookupFn = unsafe extern "system" fn(usize, *mut usize, usize) -> usize;
type OpenMenuFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
type CloseMenuFn = unsafe extern "system" fn(usize, usize) -> usize;
type WindowCloseFn = unsafe extern "system" fn(usize, usize, usize, usize);
type WindowFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type LoaderFn = unsafe extern "system" fn(usize, *mut usize, *const u16, usize) -> usize;
type CreateMovieFn = unsafe extern "system" fn(usize, *const u8, u32, usize) -> usize;
type OpenFileFn = unsafe extern "system" fn(usize, *const u8, usize, usize) -> usize;
type RequestFn = unsafe extern "system" fn(usize, *const u16) -> usize;
type ReleaseOpenedFn = unsafe extern "system" fn(usize, usize) -> usize;
type LogMessageFn = unsafe extern "system" fn(usize, u32, *const u8, *mut c_void);
type VsnprintfFn = unsafe extern "C" fn(*mut u8, usize, *const u8, *mut c_void) -> i32;

static ORIG_LOOKUP: AtomicUsize = AtomicUsize::new(0);
static ORIG_OPEN_MENU: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLOSE_MENU: AtomicUsize = AtomicUsize::new(0);
static ORIG_WINDOW_STEP: AtomicUsize = AtomicUsize::new(0);
/// Upper-case-insensitive menu names whose live main-menu window is closed and reopened at its next step.
static PENDING_WINDOW_RELOAD: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Live panel windows (vtable 0x1438e5238), from the open B / close B hooks: (window, menu name).
static PANEL_WINDOWS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
/// Menu names whose live panel windows are closed and reopened at the next main-menu window step (game thread).
static PENDING_PANEL_RELOAD: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Panels whose owners survive mission loads (seen 2026-11-13: closed only on return to the title screen).
const PERSISTENT_PANELS: [&str; 2] = ["01_MenuCommonBG", "02_OnlineArena_CommonBG"];
const PANEL_PLAYER: usize = 0x60;
const PANEL_MOVIE: usize = 0x8;
static ORIG_WINDOW_OPEN_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_WINDOW_OPEN_B: AtomicUsize = AtomicUsize::new(0);
static ORIG_WINDOW_CLOSE_B: AtomicUsize = AtomicUsize::new(0);
static ORIG_LOADER: AtomicUsize = AtomicUsize::new(0);
static ORIG_CREATE_MOVIE: AtomicUsize = AtomicUsize::new(0);
static ORIG_OPEN_FILE: AtomicUsize = AtomicUsize::new(0);
static ORIG_REQUEST: AtomicUsize = AtomicUsize::new(0);
static ORIG_RELEASE_OPENED: AtomicUsize = AtomicUsize::new(0);
static ORIG_LOG_MESSAGE: AtomicUsize = AtomicUsize::new(0);
static VSNPRINTF: AtomicUsize = AtomicUsize::new(0);
static LOG_HOOKED: AtomicBool = AtomicBool::new(false);

struct Config {
    package: Option<PathBuf>,
    fallback_at_boot: bool,
    source: String,
}
static CONFIG: OnceLock<Config> = OnceLock::new();

/// Upper-case menu names whose cached movie is released at the next lookup (game thread).
static PENDING_RELEASE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static DUMP_PENDING: AtomicBool = AtomicBool::new(false);
/// Package menu files at the last snapshot: path -> (modified, size).
static SNAPSHOT: Mutex<Option<HashMap<PathBuf, (SystemTime, u64)>>> = Mutex::new(None);
/// Last MovieDef CreateMovie returned per path (an unchanged one = Scaleform's library handed back the old movie).
static LAST_MOVIE_DEF: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);
static DISK_FALLBACK_ON: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set by release_opened inside an OpenFile call: the file came from the repository.
    static OPENED_FROM_REPOSITORY: Cell<bool> = const { Cell::new(false) };
}

static START: OnceLock<Instant> = OnceLock::new();
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetCurrentThreadId() -> u32;
    fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn LoadLibraryA(name: *const u8) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
    fn GetModuleFileNameW(module: *mut u8, buffer: *mut u16, size: u32) -> u32;
    fn RtlCaptureStackBackTrace(skip: u32, count: u32, frames: *mut usize, hash: *mut u32) -> u16;
}

/// Return addresses of the hook's callers inside the game exe (skips this function and the hook itself).
fn callers() -> String {
    let mut frames = [0usize; 8];
    let n = unsafe { RtlCaptureStackBackTrace(2, frames.len() as u32, frames.as_mut_ptr(), std::ptr::null_mut()) };
    frames[..n as usize]
        .iter()
        .filter(|&&a| (0x140000000..0x147000000).contains(&a))
        .take(5)
        .map(|a| format!("{a:#x}"))
        .collect::<Vec<_>>()
        .join(" <- ")
}

// ---- helpers ---------------------------------------------------------------------------------------------------------

fn log(line: &str) {
    let ms = START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
    let thread = unsafe { GetCurrentThreadId() };
    if let Some(file) = LOG.get() {
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{ms:>8} t{thread:<6} {line}");
            let _ = f.flush();
        }
    }
}

unsafe fn read<T: Copy>(addr: usize) -> T {
    unsafe { std::ptr::read_unaligned(addr as *const T) }
}

unsafe fn wide_str(ptr: *const u16, max: usize) -> String {
    if ptr.is_null() {
        return "<null>".into();
    }
    let mut len = 0;
    while len < max && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

unsafe fn c_str(ptr: *const u8, max: usize) -> String {
    if ptr.is_null() {
        return "<null>".into();
    }
    let mut len = 0;
    while len < max && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }).into_owned()
}

/// Name of a menu table entry (or a copy of one).
unsafe fn entry_name(entry: usize) -> String {
    unsafe { wide_str(read::<usize>(entry + 8) as *const u16, 128) }
}

fn menu_names() -> Vec<String> {
    (0..MENU_TABLE_COUNT).map(|i| unsafe { entry_name(MENU_TABLE + i * 16) }).collect()
}

/// Every (node, key) of one of the system's std::maps.
unsafe fn map_nodes(system: usize, head_offset: usize) -> Vec<(usize, String)> {
    let head: usize = unsafe { read(system + head_offset) };
    let mut out = Vec::new();
    if head == 0 {
        return out;
    }
    let mut stack = vec![unsafe { read::<usize>(head + 8) }];
    while let Some(node) = stack.pop() {
        if node == 0 || node == head || unsafe { read::<u8>(node + 0x19) } != 0 || out.len() > 1000 {
            continue;
        }
        let key_len: usize = unsafe { read(node + NODE_KEY_LEN) };
        let key = unsafe { wide_str(read::<usize>(node + NODE_KEY) as *const u16, key_len.min(256)) };
        out.push((node, key));
        stack.push(unsafe { read(node) });
        stack.push(unsafe { read(node + 0x10) });
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

unsafe fn map_find(system: usize, head_offset: usize, name: &str) -> Option<usize> {
    unsafe { map_nodes(system, head_offset) }.into_iter().find(|(_, key)| key == name).map(|(node, _)| node)
}

/// Value of a map entry: Some(value) (may be 0 when the node was emptied), None when there's no node.
unsafe fn map_value(system: usize, head_offset: usize, name: &str) -> Option<usize> {
    unsafe { map_find(system, head_offset, name) }.map(|node| unsafe { read(node + NODE_VALUE) })
}

fn describe(value: Option<usize>) -> String {
    match value {
        Some(v) => format!("{v:#x}"),
        None => "none".into(),
    }
}

/// (player, open count) of the active list.
unsafe fn active_players(system: usize) -> Vec<(usize, i32)> {
    let count: usize = unsafe { read(system + ACTIVE_COUNT) };
    let base = (system + ACTIVE_LIST + 7) & !7;
    (0..count.min(64)).map(|i| unsafe { (read(base + i * 16), read(base + i * 16 + 8)) }).collect()
}

// ---- disk fallback patch ---------------------------------------------------------------------------------------------

fn set_disk_fallback(on: bool, why: &str) {
    let site = DISK_FALLBACK_JNZ as *mut u8;
    let wanted = if on { [0x90; 6] } else { DISK_FALLBACK_ORIGINAL };
    let mut old = 0u32;
    unsafe {
        let current: [u8; 6] = std::ptr::read(site as *const [u8; 6]);
        if current == wanted {
            DISK_FALLBACK_ON.store(on, Ordering::Relaxed);
            return;
        }
        if current != DISK_FALLBACK_ORIGINAL && current != [0x90; 6] {
            log(&format!("disk fallback: unexpected bytes at {DISK_FALLBACK_JNZ:#x}: {current:02x?}; not patching"));
            return;
        }
        VirtualProtect(site as *mut c_void, 6, 0x40, &mut old);
        std::ptr::write(site as *mut [u8; 6], wanted);
        VirtualProtect(site as *mut c_void, 6, old, &mut old);
    }
    DISK_FALLBACK_ON.store(on, Ordering::Relaxed);
    log(&format!("disk fallback {} ({why})", if on { "ON" } else { "OFF" }));
}

// ---- package file watching -------------------------------------------------------------------------------------------

fn scan_dir(dir: &Path, out: &mut HashMap<PathBuf, (SystemTime, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            scan_dir(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gfx")) {
            out.insert(path, (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()));
        }
    }
}

fn scan_package() -> Option<HashMap<PathBuf, (SystemTime, u64)>> {
    let package = CONFIG.get()?.package.as_ref()?;
    let mut files = HashMap::new();
    scan_dir(&package.join("menu"), &mut files);
    Some(files)
}

/// F7: changed package files -> their menus are released at the next lookup.
fn reload_changed() {
    let Some(now) = scan_package() else {
        log("F7: no package configured (ui_hot_reload.ini: package = ...)");
        return;
    };
    let mut snapshot = SNAPSHOT.lock().unwrap();
    let before = snapshot.take().unwrap_or_default();
    let names = menu_names();
    let mut pending = Vec::new();
    for (path, stamp) in &now {
        let old = before.get(path);
        if old == Some(stamp) {
            continue;
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_uppercase()).unwrap_or_default();
        let note = if old.is_none() { " (new since launch: me3 won't serve it)" } else { "" };
        match names.iter().find(|n| n.to_uppercase() == stem) {
            Some(name) => {
                log(&format!("F7: {} changed{note} -> release {name}", path.display()));
                pending.push(name.clone());
            }
            None => log(&format!("F7: {} changed{note}, but no menu table entry is named {stem}", path.display())),
        }
    }
    *snapshot = Some(now);
    drop(snapshot);
    if pending.is_empty() {
        log("F7: no changed menu files");
        return;
    }
    set_disk_fallback(true, "F7");
    PENDING_WINDOW_RELOAD.lock().unwrap().extend(pending.iter().cloned());
    PENDING_PANEL_RELOAD.lock().unwrap().extend(pending.iter().cloned());
    PENDING_RELEASE.lock().unwrap().extend(pending);
    log("F7: releases happen at the next movie lookup");
}

// ---- game-thread work (runs inside the lookup hook) -----------------------------------------------------------------

/// Drops the cache's reference to a movie the way the game releases these wrappers (atomic decrement of the
/// refcount at +8; at zero, vtable slot 0 destroys it) and empties the entry, so the next lookup loads it again.
/// The old movie must really go: Scaleform's library hands a still-alive MovieDef back for an unchanged path+mtime.
unsafe fn release_cached(system: usize, name: &str) {
    let Some(node) = (unsafe { map_find(system, CACHE_MAP_HEAD, name) }) else {
        log(&format!("release {name}: not in the movie cache (non-resident: every lookup loads it anyway)"));
        return;
    };
    let slot = (node + NODE_VALUE) as *mut usize;
    let wrapper = unsafe { *slot };
    if wrapper == 0 {
        log(&format!("release {name}: cache entry already empty"));
        return;
    }
    unsafe { *slot = 0 };
    let refcount = unsafe { &*((wrapper + 8) as *const AtomicI32) };
    let before = refcount.fetch_sub(1, Ordering::SeqCst);
    if before == 1 {
        let vtable: usize = unsafe { read(wrapper) };
        let destroy: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(read::<usize>(vtable)) };
        unsafe { destroy(wrapper) };
        log(&format!("release {name}: wrapper {wrapper:#x} released and destroyed"));
    } else {
        log(&format!("release {name}: wrapper {wrapper:#x} released, {} other owner(s) keep it alive", before - 1));
    }
}

type MenuPathFn = unsafe extern "system" fn(*mut u8, usize) -> usize;
type RepositoryLookupFn = unsafe extern "system" fn(usize, *const u16) -> usize;
/// The in-memory file repository the game's Scaleform OpenFile reads first; lookup by wchar path -> entry.
const REPOSITORY_PTR: usize = 0x144e245b0;
const REPOSITORY_LOOKUP_ADDR: usize = 0x14153d3d0;
/// (out string object, table entry) -> string object with the menu's wchar path at +8 ("menu:/%s.gfx").
const MENU_PATH_ADDR: usize = 0x14160a5b0;

/// Drops leftover repository entries for a menu's file. Boot requests some files twice (01_900_Black,
/// 01_070_CommandList), opens them once, and the second copy stays in the repository with the already-used file
/// object; a reload would get that instead of a disk miss and the game dies in the load. The game's own release
/// (0x1416040c0, what OpenFile calls after opening from the repository) drops one reference per call.
unsafe fn purge_repository(system: usize, name: &str) {
    let repository: usize = unsafe { read(REPOSITORY_PTR) };
    if repository == 0 {
        return;
    }
    let Some(index) = menu_names().iter().position(|n| n == name) else { return };
    // the game's string object: a generous zeroed buffer, leaked (a few bytes per reload)
    let buffer = Box::leak(vec![0u8; 0x200].into_boxed_slice());
    let make_path: MenuPathFn = unsafe { std::mem::transmute(MENU_PATH_ADDR) };
    unsafe { make_path(buffer.as_mut_ptr(), MENU_TABLE + index * 16) };
    let path = usize::from_le_bytes(buffer[8..16].try_into().unwrap()) as *const u16;
    if path.is_null() {
        return;
    }
    let lookup: RepositoryLookupFn = unsafe { std::mem::transmute(REPOSITORY_LOOKUP_ADDR) };
    let release: ReleaseOpenedFn = unsafe { std::mem::transmute(ORIG_RELEASE_OPENED.load(Ordering::Relaxed)) };
    for _ in 0..8 {
        let entry = unsafe { lookup(repository, path) };
        if entry == 0 {
            return;
        }
        unsafe { release(system, entry) };
        log(&format!("release {name}: dropped a leftover repository entry {entry:#x} for {}", unsafe {
            wide_str(path, 256)
        }));
    }
    log(&format!("release {name}: WARNING: repository entry still there after 8 releases"));
}

unsafe fn dump_state(system: usize) {
    log("==== state dump ====");
    let cache = unsafe { map_nodes(system, CACHE_MAP_HEAD) };
    log(&format!("movie cache ({} entries):", cache.len()));
    let mut wrappers = HashMap::new();
    for (node, name) in &cache {
        let wrapper: usize = unsafe { read(node + NODE_VALUE) };
        let refs = if wrapper != 0 { unsafe { read::<i32>(wrapper + 8) } } else { 0 };
        log(&format!("  {name:<40} wrapper {wrapper:#x} refcount {refs}"));
        wrappers.insert(name.clone(), wrapper);
    }
    let active = unsafe { active_players(system) };
    let players = unsafe { map_nodes(system, PLAYER_MAP_HEAD) };
    log(&format!("players ({} entries):", players.len()));
    for (node, name) in &players {
        let player: usize = unsafe { read(node + NODE_VALUE) };
        if player == 0 {
            log(&format!("  {name:<40} player 0"));
            continue;
        }
        let started: u8 = unsafe { read(player + PLAYER_STARTED) };
        let opens = active.iter().find(|(p, _)| *p == player).map(|(_, c)| *c);
        // Where the player keeps the movie: fields equal to the cached wrapper or to one of its fields.
        let mut refs = Vec::new();
        if let Some(&wrapper) = wrappers.get(name) {
            if wrapper != 0 {
                for off in (0..PLAYER_SIZE).step_by(8) {
                    let v: usize = unsafe { read(player + off) };
                    if v == wrapper {
                        refs.push(format!("+{off:#x}=wrapper"));
                    }
                    for woff in (0x10..WRAPPER_SIZE).step_by(8) {
                        let w: usize = unsafe { read(wrapper + woff) };
                        if v != 0 && v == w {
                            refs.push(format!("+{off:#x}=wrapper+{woff:#x}"));
                        }
                    }
                }
            }
        }
        log(&format!(
            "  {name:<40} player {player:#x} started={started} active opens={} {}",
            opens.map(|c| c.to_string()).unwrap_or("-".into()),
            if refs.is_empty() { "(no field matches the cached wrapper)".into() } else { refs.join(" ") }
        ));
    }
    log(&format!("active list ({} entries):", active.len()));
    for (player, opens) in &active {
        let name = players
            .iter()
            .find(|(node, _)| unsafe { read::<usize>(node + NODE_VALUE) } == *player)
            .map(|(_, n)| n.as_str())
            .unwrap_or("?");
        log(&format!("  {player:#x} opens={opens} ({name})"));
    }
    log("==== end of dump ====");
}

/// F10 (break test): closes every active player with the game's own close (once per open count) and releases every
/// cached movie. The menu windows still hold their player pointers, movie instances and controllers, so expected:
/// the menus vanish, nothing reloads by itself, and a window's later close hits a dangling player.
static NUKE_PENDING: AtomicBool = AtomicBool::new(false);

unsafe fn nuke_players(system: usize) {
    let active = unsafe { active_players(system) };
    log(&format!("F10: closing {} active players", active.len()));
    let close: CloseMenuFn = unsafe { std::mem::transmute(ORIG_CLOSE_MENU.load(Ordering::Relaxed)) };
    for (player, opens) in active {
        let name = unsafe { wide_str(read::<usize>(player + PLAYER_NAME) as *const u16, 128) };
        for _ in 0..opens.max(1) {
            unsafe { close(system, player) };
        }
        log(&format!("F10: closed {name} (player {player:#x}, {opens} open(s))"));
    }
    log(&format!("F10: active list now {} entries", unsafe { active_players(system) }.len()));
    for name in menu_names() {
        if unsafe { map_value(system, CACHE_MAP_HEAD, &name) }.is_some_and(|w| w != 0) {
            unsafe { release_cached(system, &name) };
        }
    }
}

unsafe fn game_thread_work(system: usize) {
    if NUKE_PENDING.swap(false, Ordering::Relaxed) {
        unsafe { nuke_players(system) };
    }
    let pending = std::mem::take(&mut *PENDING_RELEASE.lock().unwrap());
    for name in pending {
        unsafe { release_cached(system, &name) };
        unsafe { purge_repository(system, &name) };
    }
    if DUMP_PENDING.swap(false, Ordering::Relaxed) {
        unsafe { dump_state(system) };
    }
}

// ---- hooks -----------------------------------------------------------------------------------------------------------

unsafe extern "system" fn lookup_hook(system: usize, out: *mut usize, entry: usize) -> usize {
    if !LOG_HOOKED.swap(true, Ordering::Relaxed) {
        unsafe { hook_scaleform_log(system) };
    }
    unsafe { game_thread_work(system) };
    let name = unsafe { entry_name(entry) };
    let resident: u8 = unsafe { read(entry + 4) };
    let cached = unsafe { map_value(system, CACHE_MAP_HEAD, &name) };
    let orig: LookupFn = unsafe { std::mem::transmute(ORIG_LOOKUP.load(Ordering::Relaxed)) };
    let ret = unsafe { orig(system, out, entry) };
    let result = unsafe { *out };
    log(&format!("lookup {name} resident={resident} cached={} -> {result:#x}", describe(cached)));
    ret
}

unsafe extern "system" fn open_menu_hook(system: usize, entry: usize, flag: usize) -> usize {
    let name = unsafe { entry_name(entry) };
    let before = unsafe { map_value(system, PLAYER_MAP_HEAD, &name) };
    let orig: OpenMenuFn = unsafe { std::mem::transmute(ORIG_OPEN_MENU.load(Ordering::Relaxed)) };
    let player = unsafe { orig(system, entry, flag) };
    let kind = match before {
        _ if player == 0 => "no player",
        Some(p) if p == player => "reused",
        _ => "new",
    };
    let started: u8 = if player != 0 { unsafe { read(player + PLAYER_STARTED) } } else { 0 };
    let opens = unsafe { active_players(system) }.into_iter().find(|(p, _)| *p == player).map(|(_, c)| c);
    log(&format!(
        "open {name} (flag {:#x}): player {player:#x} {kind} (map had {}), started={started}, active opens={}; from {}",
        flag & 0xff,
        describe(before),
        opens.map(|c| c.to_string()).unwrap_or("-".into()),
        callers()
    ));
    player
}

/// Main-menu window step. A live window (controller set) with a pending reload is closed with its own close first;
/// the original step then finds no controller and reopens it in the same frame, through a fresh movie lookup (where
/// the pending cache release runs, so the movie is read from disk).
/// Panel windows: the owner reaches the panel's movie through window+8 each time (0x140c8a8c0, which also opens a
/// closed panel lazily), and the game itself swaps a panel's movie by closing it (0x140c8a6b0). So a reload is the
/// panel's own close + open; open rebinds window+8 to the new movie instance. Runs on the game thread.
unsafe fn reload_pending_panels() {
    let pending = std::mem::take(&mut *PENDING_PANEL_RELOAD.lock().unwrap());
    if pending.is_empty() {
        return;
    }
    let windows: Vec<(usize, String)> = PANEL_WINDOWS
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, n)| pending.iter().any(|p| p.eq_ignore_ascii_case(n)))
        .cloned()
        .collect();
    for (window, name) in windows {
        let (player, movie): (usize, usize) = unsafe { (read(window + PANEL_PLAYER), read(window + PANEL_MOVIE)) };
        if player == 0 {
            log(&format!("panel reload {name}: window {window:#x} not open; left alone"));
            continue;
        }
        let close: WindowFn = unsafe { std::mem::transmute(ORIG_WINDOW_CLOSE_B.load(Ordering::Relaxed)) };
        let open: WindowFn = unsafe { std::mem::transmute(ORIG_WINDOW_OPEN_B.load(Ordering::Relaxed)) };
        unsafe { close(window, 0, 0, 0) };
        unsafe { open(window, 0, 0, 0) };
        log(&format!(
            "panel reload {name}: window {window:#x}: player {player:#x} -> {:#x}, movie {movie:#x} -> {:#x}",
            unsafe { read::<usize>(window + PANEL_PLAYER) },
            unsafe { read::<usize>(window + PANEL_MOVIE) }
        ));
        // The panel's owner (a C++ view object) took handles into the old movie when it was built (02_PilotData:
        // "PilotData", "ArenaRankIcon"...), so it keeps writing there: the reloaded panel is a preview of the art.
        let rebuild = if PERSISTENT_PANELS.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
            "only on return to the title screen (its owner survives mission loads)"
        } else {
            "on the next scene change (AC TEST, sortie, back to the garage)"
        };
        log(&format!(
            "panel reload {name}: PREVIEW ONLY: shapes and colors are the new file's, but its text/data and input stay \
             with the old movie until its owner is rebuilt, {rebuild}"
        ));
    }
}

unsafe extern "system" fn window_step_hook(window: usize, out: usize, p3: usize, p4: usize) -> usize {
    unsafe { reload_pending_panels() };
    let name_ptr: usize = unsafe { read(window + MAIN_WINDOW_NAME) };
    let mut pending = PENDING_WINDOW_RELOAD.lock().unwrap();
    if name_ptr != 0 && !pending.is_empty() {
        let name = unsafe { wide_str(name_ptr as *const u16, 128) };
        if let Some(i) = pending.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
            pending.remove(i);
            drop(pending);
            let controller: usize = unsafe { read(window + MAIN_WINDOW_CONTROLLER) };
            let factory: usize = unsafe { read(window + MAIN_WINDOW_FACTORY) };
            if controller != 0 && factory != 0 {
                let player: usize = unsafe { read(window + MAIN_WINDOW_PLAYER) };
                let command: usize = unsafe { read(window + MAIN_WINDOW_COMMAND) };
                log(&format!(
                    "window reload {name}: window {window:#x}, closing (player {player:#x}, controller {controller:#x}, command {command:#x})"
                ));
                // +0xf0..+0x130 is an MSVC std::function (inline storage +0xf0, impl pointer +0x128): the job's
                // "should this menu close" check (0x140d62120 for the garage top menu, asks CSMenuMan). The close
                // destroys it and the reopen never sets it again, so keep it out of the close and put it back.
                let callable: usize = unsafe { read(window + 0xf8) };
                // Only the simple kind is known safe to keep: stored inline (impl = window+0xf0) and wrapping a plain
                // function pointer. Anything else may capture state that the close frees.
                if command != 0 {
                    let function_vtable: usize = unsafe { read(command) };
                    if command != window + 0xf0 || function_vtable != FN_POINTER_FUNCTION_VTABLE {
                        log(&format!(
                            "window reload {name}: WARNING: close check is not an inline function pointer (impl {command:#x}, \
                             vtable {function_vtable:#x}); keeping it anyway, it may refer to freed state"
                        ));
                    }
                }
                unsafe { std::ptr::write_unaligned((window + MAIN_WINDOW_COMMAND) as *mut usize, 0) };
                let close: WindowCloseFn = unsafe { std::mem::transmute(WINDOW_CLOSE_ADDR) };
                unsafe { close(window, 0, 0, 0) };
                unsafe { std::ptr::write_unaligned((window + MAIN_WINDOW_COMMAND) as *mut usize, command) };
                log(&format!("window reload {name}: close-check std::function kept (impl {command:#x}, callable {callable:#x})"));
                log(&format!(
                    "window reload {name}: closed (player {:#x}, controller {:#x}, command {:#x}); stepping to reopen",
                    unsafe { read::<usize>(window + MAIN_WINDOW_PLAYER) },
                    unsafe { read::<usize>(window + MAIN_WINDOW_CONTROLLER) },
                    unsafe { read::<usize>(window + MAIN_WINDOW_COMMAND) }
                ));
                let orig: WindowFn = unsafe { std::mem::transmute(ORIG_WINDOW_STEP.load(Ordering::Relaxed)) };
                let ret = unsafe { orig(window, out, p3, p4) };
                log(&format!(
                    "window reload {name}: after step: player {:#x}, controller {:#x}, command {:#x}",
                    unsafe { read::<usize>(window + MAIN_WINDOW_PLAYER) },
                    unsafe { read::<usize>(window + MAIN_WINDOW_CONTROLLER) },
                    unsafe { read::<usize>(window + MAIN_WINDOW_COMMAND) }
                ));
                return ret;
            }
            log(&format!("window reload {name}: not live (controller {controller:#x}, factory {factory:#x}); left alone"));
        }
    } else {
        drop(pending);
    }
    let orig: WindowFn = unsafe { std::mem::transmute(ORIG_WINDOW_STEP.load(Ordering::Relaxed)) };
    unsafe { orig(window, out, p3, p4) }
}

unsafe extern "system" fn close_menu_hook(system: usize, player: usize) -> usize {
    let name = if player != 0 { unsafe { wide_str(read::<usize>(player + PLAYER_NAME) as *const u16, 128) } } else { "<null>".into() };
    log(&format!("close {name}: player {player:#x}; from {}", callers()));
    let orig: CloseMenuFn = unsafe { std::mem::transmute(ORIG_CLOSE_MENU.load(Ordering::Relaxed)) };
    unsafe { orig(system, player) }
}

/// Name of a menu window's table entry copy (+0x48, name pointer +0x50).
unsafe fn window_name(window: usize) -> String {
    let ptr: usize = unsafe { read(window + 0x50) };
    if ptr == 0 { "<none>".into() } else { unsafe { wide_str(ptr as *const u16, 128) } }
}

unsafe fn window_call(which: &str, orig: &AtomicUsize, w: usize, a: usize, b: usize, c: usize) -> usize {
    let name = unsafe { window_name(w) };
    let vtable: usize = unsafe { read(w) };
    let (p58, p60): (usize, usize) = unsafe { (read(w + 0x58), read(w + 0x60)) };
    let f: WindowFn = unsafe { std::mem::transmute(orig.load(Ordering::Relaxed)) };
    let ret = unsafe { f(w, a, b, c) };
    let (q58, q60): (usize, usize) = unsafe { (read(w + 0x58), read(w + 0x60)) };
    {
        let mut panels = PANEL_WINDOWS.lock().unwrap();
        panels.retain(|(window, _)| *window != w);
        if which == "open B" && q60 != 0 {
            panels.push((w, name.clone()));
        }
    }
    log(&format!(
        "window {which} {name}: window {w:#x} vtable {vtable:#x}, +0x58 {p58:#x}->{q58:#x}, +0x60 {p60:#x}->{q60:#x}, \
         movie +0x8 {:#x}, binding +0xa0 {:#x}; from {}",
        unsafe { read::<usize>(w + 8) },
        unsafe { read::<usize>(w + 0xa0) },
        callers()
    ));
    ret
}

unsafe extern "system" fn window_open_a_hook(w: usize, a: usize, b: usize, c: usize) -> usize {
    unsafe { window_call("open A", &ORIG_WINDOW_OPEN_A, w, a, b, c) }
}
unsafe extern "system" fn window_open_b_hook(w: usize, a: usize, b: usize, c: usize) -> usize {
    unsafe { window_call("open B", &ORIG_WINDOW_OPEN_B, w, a, b, c) }
}
unsafe extern "system" fn window_close_b_hook(w: usize, a: usize, b: usize, c: usize) -> usize {
    unsafe { window_call("close B", &ORIG_WINDOW_CLOSE_B, w, a, b, c) }
}

unsafe extern "system" fn loader_hook(loader: usize, out: *mut usize, path: *const u16, p4: usize) -> usize {
    let path_text = unsafe { wide_str(path, 512) };
    log(&format!("  load {path_text}"));
    let orig: LoaderFn = unsafe { std::mem::transmute(ORIG_LOADER.load(Ordering::Relaxed)) };
    let ret = unsafe { orig(loader, out, path, p4) };
    log(&format!("  load {path_text} -> wrapper {:#x}", unsafe { *out }));
    ret
}

unsafe extern "system" fn create_movie_hook(loader: usize, path: *const u8, flags: u32, arena: usize) -> usize {
    let orig: CreateMovieFn = unsafe { std::mem::transmute(ORIG_CREATE_MOVIE.load(Ordering::Relaxed)) };
    let def = unsafe { orig(loader, path, flags, arena) };
    let path_text = unsafe { c_str(path, 512) };
    let previous = LAST_MOVIE_DEF.lock().unwrap().get_or_insert_default().insert(path_text.clone(), def);
    let note = match previous {
        Some(p) if p == def && def != 0 => " (same MovieDef as last time: Scaleform's library cache, file not read)",
        _ => "",
    };
    log(&format!("    CreateMovie {path_text} -> {def:#x}{note}"));
    def
}

unsafe extern "system" fn open_file_hook(this: usize, url: *const u8, flags: usize, mode: usize) -> usize {
    let previous = OPENED_FROM_REPOSITORY.replace(false);
    let orig: OpenFileFn = unsafe { std::mem::transmute(ORIG_OPEN_FILE.load(Ordering::Relaxed)) };
    let file = unsafe { orig(this, url, flags, mode) };
    let source = match file {
        0 => "FAILED",
        _ if OPENED_FROM_REPOSITORY.get() => "repository",
        _ => "disk",
    };
    OPENED_FROM_REPOSITORY.set(previous);
    let fallback = if DISK_FALLBACK_ON.load(Ordering::Relaxed) { "on" } else { "off" };
    log(&format!("      OpenFile {} -> {file:#x} ({source}; disk fallback {fallback})", unsafe { c_str(url, 512) }));
    file
}

unsafe extern "system" fn request_hook(system: usize, path: *const u16) -> usize {
    let orig: RequestFn = unsafe { std::mem::transmute(ORIG_REQUEST.load(Ordering::Relaxed)) };
    let ret = unsafe { orig(system, path) };
    let count: usize = unsafe { read(system + SYSTEM_FILE_COUNT) };
    log(&format!("request {} -> held files {count}", unsafe { wide_str(path, 512) }));
    ret
}

unsafe extern "system" fn release_opened_hook(system: usize, file_entry: usize) -> usize {
    OPENED_FROM_REPOSITORY.set(true);
    let orig: ReleaseOpenedFn = unsafe { std::mem::transmute(ORIG_RELEASE_OPENED.load(Ordering::Relaxed)) };
    let ret = unsafe { orig(system, file_entry) };
    let count: usize = unsafe { read(system + SYSTEM_FILE_COUNT) };
    log(&format!("        repository file {file_entry:#x} released after open, held files {count}"));
    ret
}

unsafe extern "system" fn log_message_hook(this: usize, id: u32, fmt: *const u8, args: *mut c_void) {
    let vsnprintf = VSNPRINTF.load(Ordering::Relaxed);
    let text = if vsnprintf != 0 && !fmt.is_null() {
        let f: VsnprintfFn = unsafe { std::mem::transmute(vsnprintf) };
        let mut buf = [0u8; 2048];
        let n = unsafe { f(buf.as_mut_ptr(), buf.len() - 1, fmt, args) };
        let n = if n < 0 { buf.len() - 1 } else { (n as usize).min(buf.len() - 1) };
        String::from_utf8_lossy(&buf[..n]).trim_end().to_string()
    } else {
        unsafe { c_str(fmt, 1024) }
    };
    log(&format!("    scaleform log [{id:#x}] {text}"));
    let orig: LogMessageFn = unsafe { std::mem::transmute(ORIG_LOG_MESSAGE.load(Ordering::Relaxed)) };
    unsafe { orig(this, id, fmt, args) };
}

/// Replaces slot 1 (LogMessageVarg) of the game's Scaleform log object's vtable (the game drops these messages).
unsafe fn hook_scaleform_log(system: usize) {
    let loader: usize = unsafe { read(system + SYSTEM_LOADER) };
    let log_object: usize = if loader != 0 { unsafe { read(loader + LOADER_LOG) } } else { 0 };
    if log_object == 0 {
        log("no Scaleform log object found");
        return;
    }
    let slot = unsafe { read::<usize>(log_object) } + 8;
    ORIG_LOG_MESSAGE.store(unsafe { read(slot) }, Ordering::Relaxed);
    let mut old = 0u32;
    unsafe {
        VirtualProtect(slot as *mut c_void, 8, 0x04, &mut old);
        std::ptr::write_unaligned(slot as *mut usize, log_message_hook as *const () as usize);
        VirtualProtect(slot as *mut c_void, 8, old, &mut old);
    }
    log(&format!("Scaleform log object {log_object:#x}: LogMessageVarg hooked"));
}

unsafe fn hook(addr: usize, prologue: &[u8], detour: *const (), orig: &AtomicUsize) -> Result<(), String> {
    let actual = unsafe { std::slice::from_raw_parts(addr as *const u8, prologue.len()) };
    if actual != prologue {
        return Err(format!("prologue at {addr:#x} doesn't match (not 1.09.1?): {actual:02x?}"));
    }
    let d = unsafe { RawDetour::new(addr as *const (), detour) }.map_err(|e| e.to_string())?;
    orig.store(d.trampoline() as *const () as usize, Ordering::Relaxed);
    unsafe { d.enable() }.map_err(|e| e.to_string())?;
    std::mem::forget(d);
    Ok(())
}

// ---- setup -----------------------------------------------------------------------------------------------------------

fn key_loop() {
    let keys = [VK_F5, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11];
    let mut was_down = [false; 6];
    loop {
        for (i, key) in keys.into_iter().enumerate() {
            let down = unsafe { GetAsyncKeyState(key) } as u16 & 0x8000 != 0;
            if down && !was_down[i] {
                match key {
                    VK_F5 => {
                        let menu_man: usize = unsafe { read(CS_MENU_MAN) };
                        watch::toggle(if menu_man != 0 { menu_man + 0x10 } else { 0 });
                    }
                    VK_F7 => reload_changed(),
                    VK_F8 => {
                        set_disk_fallback(true, "F8");
                        PENDING_RELEASE.lock().unwrap().extend(menu_names());
                        log("F8: every cached movie is released at the next lookup");
                    }
                    VK_F9 => {
                        DUMP_PENDING.store(true, Ordering::Relaxed);
                        log("F9: state dump at the next lookup");
                    }
                    VK_F10 => {
                        set_disk_fallback(true, "F10");
                        NUKE_PENDING.store(true, Ordering::Relaxed);
                        log("F10: every active player closed and every movie released at the next lookup");
                    }
                    _ => set_disk_fallback(!DISK_FALLBACK_ON.load(Ordering::Relaxed), "F11"),
                }
            }
            was_down[i] = down;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `ui_hot_reload.ini` next to the DLL or in one of its parent folders.
fn load_config(dll_path: &Path) -> Config {
    let mut config = Config { package: None, fallback_at_boot: false, source: "no ui_hot_reload.ini found".into() };
    let Some(file) = dll_path.ancestors().skip(1).map(|d| d.join("ui_hot_reload.ini")).find(|p| p.is_file()) else {
        return config;
    };
    config.source = file.display().to_string();
    for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            // a relative path is relative to the ini's folder
            "package" => config.package = Some(file.parent().unwrap_or(Path::new(".")).join(value.trim())),
            "disk_fallback" => config.fallback_at_boot = value.trim().eq_ignore_ascii_case("boot"),
            _ => {}
        }
    }
    config
}

fn init(module: *mut u8) -> Result<(), String> {
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    let dll_path = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
    let config = CONFIG.get_or_init(|| load_config(&dll_path));
    log(&format!(
        "config {}: package {}, disk fallback {}",
        config.source,
        config.package.as_ref().map(|p| p.display().to_string()).unwrap_or("(none)".into()),
        if config.fallback_at_boot { "at boot" } else { "on first reload" }
    ));
    *SNAPSHOT.lock().unwrap() = scan_package();
    log(&format!("{} package menu files in the snapshot", SNAPSHOT.lock().unwrap().as_ref().map_or(0, |s| s.len())));

    let msvcrt = unsafe { LoadLibraryA(c"msvcrt.dll".as_ptr() as *const u8) };
    if msvcrt != 0 {
        VSNPRINTF.store(unsafe { GetProcAddress(msvcrt, c"_vsnprintf".as_ptr() as *const u8) }, Ordering::Relaxed);
    }
    unsafe {
        hook(LOOKUP_ADDR, &LOOKUP_PROLOGUE, lookup_hook as *const (), &ORIG_LOOKUP)?;
        hook(OPEN_MENU_ADDR, &OPEN_MENU_PROLOGUE, open_menu_hook as *const (), &ORIG_OPEN_MENU)?;
        hook(CLOSE_MENU_ADDR, &CLOSE_MENU_PROLOGUE, close_menu_hook as *const (), &ORIG_CLOSE_MENU)?;
        hook(WINDOW_STEP_ADDR, &WINDOW_STEP_PROLOGUE, window_step_hook as *const (), &ORIG_WINDOW_STEP)?;
        let close = std::slice::from_raw_parts(WINDOW_CLOSE_ADDR as *const u8, 8);
        if close != WINDOW_CLOSE_PROLOGUE {
            return Err(format!("window close at {WINDOW_CLOSE_ADDR:#x} doesn't match: {close:02x?}"));
        }
        hook(WINDOW_OPEN_A_ADDR, &WINDOW_OPEN_A_PROLOGUE, window_open_a_hook as *const (), &ORIG_WINDOW_OPEN_A)?;
        hook(WINDOW_OPEN_B_ADDR, &WINDOW_OPEN_B_PROLOGUE, window_open_b_hook as *const (), &ORIG_WINDOW_OPEN_B)?;
        hook(WINDOW_CLOSE_B_ADDR, &WINDOW_CLOSE_B_PROLOGUE, window_close_b_hook as *const (), &ORIG_WINDOW_CLOSE_B)?;
        hook(LOADER_ADDR, &LOADER_PROLOGUE, loader_hook as *const (), &ORIG_LOADER)?;
        hook(CREATE_MOVIE_ADDR, &CREATE_MOVIE_PROLOGUE, create_movie_hook as *const (), &ORIG_CREATE_MOVIE)?;
        hook(OPEN_FILE_ADDR, &OPEN_FILE_PROLOGUE, open_file_hook as *const (), &ORIG_OPEN_FILE)?;
        hook(REQUEST_ADDR, &REQUEST_PROLOGUE, request_hook as *const (), &ORIG_REQUEST)?;
        hook(RELEASE_OPENED_ADDR, &RELEASE_OPENED_PROLOGUE, release_opened_hook as *const (), &ORIG_RELEASE_OPENED)?;
    }
    if config.fallback_at_boot {
        set_disk_fallback(true, "config: at boot");
    }
    std::thread::spawn(key_loop);
    Ok(())
}

fn log_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("debug-logs")))
        .unwrap_or_else(|| "debug-logs".into())
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        let dir = log_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = START.set(Instant::now());
        if let Ok(file) = std::fs::File::create(dir.join("ui_hot_reload_probe.log")) {
            let _ = LOG.set(Mutex::new(file));
        }
        match init(module) {
            Ok(()) => log("hooked"),
            Err(e) => log(&format!("not hooked: {e}")),
        }
    }
    1
}
