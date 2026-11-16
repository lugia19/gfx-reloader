//! Menu reloads. Requests come from the key/watcher thread; game work runs in the window step hook (game thread).
//! Per menu: release the cached movie, reopen live panels (preview only), close + reopen the live window. If no
//! window picks it up within one round of steps, it's off screen and loads the new file next time.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::game::*;
use crate::{TOAST_LINE, TOAST_LONG_SECONDS, log, toast, toast_for};

/// Window step/close, panel open/close.
type WindowFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type OpenFileFn = unsafe extern "system" fn(usize, *const u8, usize, usize) -> usize;
type StoreLookupFn = unsafe extern "system" fn(usize, *const u16) -> usize;
type StoreReleaseFn = unsafe extern "system" fn(usize, usize) -> usize;
type ModifyTimeFn = unsafe extern "system" fn(usize, *const u8) -> i64;
type PlayerCloseFn = unsafe extern "system" fn(usize, usize) -> usize;

pub static ORIG_WINDOW_STEP: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PANEL_OPEN: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PLAYER_CLOSE: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_OPEN_FILE: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_MODIFY_TIME: AtomicUsize = AtomicUsize::new(0);

/// Added (shifted above real times) to mod files' modification time, so Scaleform never reuses an older movie.
static RELOAD_ROUND: AtomicI64 = AtomicI64::new(0);

/// `data0:/...` is an archive; mod loaders rename overridden files (me3: `\\me3??54`). One letter = a drive.
fn is_archive_path(path: &str) -> bool {
    path.split_once(":/").is_some_and(|(root, _)| root.len() >= 2 && root.chars().all(|c| c.is_ascii_alphanumeric()))
}

static PENDING: AtomicBool = AtomicBool::new(false);
static LIST_REQUESTED: AtomicBool = AtomicBool::new(false);
static REQUESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ROUND: Mutex<Option<Round>> = Mutex::new(None);
/// Every panel close goes through the player close (ER has no panel close to hook).
static PANELS: Mutex<Vec<Panel>> = Mutex::new(Vec::new());

struct Panel {
    panel: usize,
    player: usize,
    name: String,
}

/// In case the round's first window closes and never steps again.
const ROUND_SAFETY_CAP: Duration = Duration::from_secs(2);

struct Waiting {
    name: String,
    previewed: bool,
}

struct Round {
    /// When it steps again, every live window has stepped.
    first_window: usize,
    started: Instant,
    waiting: Vec<Waiting>,
    menus: usize,
    previews: usize,
}

pub fn request(names: Vec<String>) {
    let mut requests = REQUESTS.lock().unwrap();
    requests.extend(names);
    PENDING.store(true, Ordering::Relaxed);
}

pub fn request_list() {
    let _requests = REQUESTS.lock().unwrap(); // the idle check uses this lock
    LIST_REQUESTED.store(true, Ordering::Relaxed);
    PENDING.store(true, Ordering::Relaxed);
}

fn warn(text: &str) {
    log(&format!("warning: {text}"));
    toast_for(text, TOAST_LONG_SECONDS);
}

fn round_summary(round: &Round) -> String {
    let menus = if round.menus == 1 { "1 menu".to_string() } else { format!("{} menus", round.menus) };
    if round.previews == 0 {
        return format!("Reloaded {menus}");
    }
    format!("Reloaded {menus} ({} preview only, broken until the next scene change)", round.previews)
}

/// F9, game thread.
unsafe fn list_on_screen(system: usize) {
    let panels: Vec<String> = PANELS.lock().unwrap().iter().map(|p| p.name.clone()).collect();
    let mut names = unsafe { active_menu_names(system) };
    names.sort();
    names.dedup();
    let entries: Vec<String> = names
        .iter()
        .map(|n| if panels.iter().any(|p| p == n) { format!("{n} (preview only)") } else { n.clone() })
        .collect();
    log(&format!("on screen: {}", entries.join(", ")));
    let room = TOAST_LINE - "UI on screen (10/10): ".len();
    let mut lines: Vec<String> = Vec::new();
    for entry in entries {
        match lines.last_mut() {
            Some(line) if line.len() + 2 + entry.len() <= room => {
                line.push_str(", ");
                line.push_str(&entry);
            }
            _ => lines.push(entry),
        }
    }
    let total = lines.len();
    for (i, line) in lines.iter().enumerate() {
        toast_for(&format!("UI on screen ({}/{total}): {line}", i + 1), TOAST_LONG_SECONDS);
    }
}

fn original<F: Copy>(slot: &AtomicUsize) -> F {
    assert_eq!(size_of::<F>(), size_of::<usize>());
    unsafe { std::mem::transmute_copy(&slot.load(Ordering::Relaxed)) }
}

// ---- hooks ----------------------------------------------------------------------------------------------------------

/// Every live window, every frame.
pub unsafe extern "system" fn window_step_hook(window: usize, out: usize, p3: usize, p4: usize) -> usize {
    if PENDING.load(Ordering::Relaxed) {
        if let Some(name) = unsafe { next_reload(window) } {
            return unsafe { reload_window(window, &name, out, p3, p4) };
        }
    }
    unsafe { original::<WindowFn>(&ORIG_WINDOW_STEP)(window, out, p3, p4) }
}

pub unsafe extern "system" fn panel_open_hook(panel: usize, a: usize, b: usize, c: usize) -> usize {
    let result = unsafe { original::<WindowFn>(&ORIG_PANEL_OPEN)(panel, a, b, c) };
    let player: usize = unsafe { read(panel + PANEL_PLAYER) };
    let mut panels = PANELS.lock().unwrap();
    panels.retain(|p| p.panel != panel);
    if player != 0 {
        panels.push(Panel { panel, player, name: unsafe { name_at(panel + PANEL_NAME) } });
    }
    result
}

pub unsafe extern "system" fn player_close_hook(system: usize, player: usize) -> usize {
    PANELS.lock().unwrap().retain(|p| p.player != player);
    unsafe { original::<PlayerCloseFn>(&ORIG_PLAYER_CLOSE)(system, player) }
}

/// Mod files are read from disk by the default opener; archive paths go to the game's store.
pub unsafe extern "system" fn open_file_hook(this: usize, url: *const u8, flags: usize, mode: usize) -> usize {
    let path = (!url.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(url.cast()) }.to_string_lossy());
    if let Some(path) = path.filter(|p| !is_archive_path(p)) {
        let default_open: OpenFileFn = unsafe { std::mem::transmute(code(&game().file_opener.default_open)) };
        let file = unsafe { default_open(this, url, flags, mode) };
        if file != 0 {
            unsafe { release_store_entry(this, &path) };
            return file;
        }
        log(&format!("warning: {path:?} couldn't be read from disk; using the boot copy"));
    }
    unsafe { original::<OpenFileFn>(&ORIG_OPEN_FILE)(this, url, flags, mode) }
}

/// As the game's opener does after opening from its store (AC6).
unsafe fn release_store_entry(opener: usize, path: &str) {
    let Some(store) = &game().file_opener.store else { return };
    let repository: usize = unsafe { read(at(store.repository)) };
    let system: usize = unsafe { read(opener + OPENER_SYSTEM) };
    if repository == 0 || system == 0 {
        return;
    }
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let lookup: StoreLookupFn = unsafe { std::mem::transmute(code(&store.lookup)) };
    let entry = unsafe { lookup(repository, wide.as_ptr()) };
    if entry != 0 {
        let release: StoreReleaseFn = unsafe { std::mem::transmute(code(&store.release)) };
        unsafe { release(system, entry) };
    }
}

/// A copied file keeps its source's time, so Scaleform would reuse the old movie; mod files get the round added.
pub unsafe extern "system" fn modify_time_hook(this: usize, url: *const u8) -> i64 {
    let time = unsafe { original::<ModifyTimeFn>(&ORIG_MODIFY_TIME)(this, url) };
    if url.is_null() || is_archive_path(&unsafe { std::ffi::CStr::from_ptr(url.cast()) }.to_string_lossy()) {
        return time;
    }
    time.wrapping_add(RELOAD_ROUND.load(Ordering::Relaxed) << 32)
}

// ---- game-thread work -----------------------------------------------------------------------------------------------

/// Game thread. Returns the menu to reload if `window` is the one it waits for.
unsafe fn next_reload(window: usize) -> Option<String> {
    if LIST_REQUESTED.swap(false, Ordering::Relaxed) {
        let system = scaleform_system();
        if system != 0 {
            unsafe { list_on_screen(system) };
        }
    }
    let mut round = ROUND.lock().unwrap();
    if round.as_ref().is_some_and(|r| r.first_window == window || r.started.elapsed() > ROUND_SAFETY_CAP) {
        let finished = round.take().unwrap();
        for menu in &finished.waiting {
            if !menu.previewed {
                log(&format!("{}: not on screen right now; the new file is used the next time it opens", menu.name));
            }
        }
        toast(&round_summary(&finished));
    }
    let new = unsafe { handle_requests() };
    if !new.is_empty() {
        let current = round.get_or_insert_with(|| Round {
            first_window: window,
            started: Instant::now(),
            waiting: Vec::new(),
            menus: 0,
            previews: 0,
        });
        current.menus += new.len();
        current.previews += new.iter().filter(|m| m.previewed).count();
        current.waiting.extend(new);
    }
    let Some(current) = round.as_mut() else {
        // idle, unless a request just arrived
        let requests = REQUESTS.lock().unwrap();
        PENDING.store(!requests.is_empty(), Ordering::Relaxed);
        return None;
    };
    let name = unsafe { name_at(window + WINDOW_NAME) };
    let index = current.waiting.iter().position(|m| m.name.eq_ignore_ascii_case(&name))?;
    let menu = current.waiting.remove(index);
    let live = unsafe { read::<usize>(window + WINDOW_CONTROLLER) } != 0
        && unsafe { read::<usize>(window + WINDOW_FACTORY) } != 0;
    if !live {
        log(&format!("{}: opening right now; it uses the new file", menu.name));
        return None;
    }
    Some(menu.name)
}

unsafe fn handle_requests() -> Vec<Waiting> {
    let requests = std::mem::take(&mut *REQUESTS.lock().unwrap());
    if requests.is_empty() {
        return Vec::new();
    }
    let system = scaleform_system();
    if system == 0 {
        log("reload skipped: the menu system isn't up yet");
        return Vec::new();
    }
    RELOAD_ROUND.fetch_add(1, Ordering::Relaxed);
    requests
        .into_iter()
        .map(|name| {
            unsafe { release_cached(system, &name) };
            let previewed = unsafe { reload_panels(system, &name) };
            Waiting { name, previewed }
        })
        .collect()
}

/// Must really free the movie, or Scaleform hands it back on the next load.
unsafe fn release_cached(system: usize, name: &str) {
    let Some(node) = (unsafe { cache_node(system, name) }) else { return };
    let value = node + game().node_value;
    let wrapper: usize = unsafe { read(value) };
    if wrapper == 0 {
        return;
    }
    unsafe { write::<usize>(value, 0) };
    let refcount = unsafe { &*((wrapper + 8) as *const std::sync::atomic::AtomicI32) };
    if refcount.fetch_sub(1, Ordering::SeqCst) == 1 {
        let destroy: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(read::<usize>(read::<usize>(wrapper))) };
        unsafe { destroy(wrapper) };
    }
}

/// Owners keep handles into the old movie, so text, data and input stay there.
unsafe fn reload_panels(system: usize, name: &str) -> bool {
    let panels: Vec<(usize, usize)> = PANELS
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| (p.panel, p.player))
        .collect();
    if panels.is_empty() {
        return false;
    }
    // through the hooked functions, so PANELS follows
    let open: WindowFn = unsafe { std::mem::transmute(code(&game().panel_open)) };
    for (panel, player) in panels {
        unsafe { close_panel(system, panel, player) };
        unsafe { open(panel, 0, 0, 0) };
    }
    log(&format!(
        "{name}: panel reloaded as a PREVIEW: new shapes and colors, but its text, data and input stay with the old \
         movie until the game rebuilds it (usually the next scene change)"
    ));
    true
}

/// ER closes panels inline, so do it by hand there.
unsafe fn close_panel(system: usize, panel: usize, player: usize) {
    match &game().panel_close {
        Some(close) => {
            let close: WindowFn = unsafe { std::mem::transmute(code(close)) };
            unsafe { close(panel, 0, 0, 0) };
        }
        None => {
            let close: PlayerCloseFn = unsafe { std::mem::transmute(code(&game().player_close)) };
            unsafe { close(system, player) };
            unsafe { write::<usize>(panel + PANEL_PLAYER, 0) };
        }
    }
}

/// Own close, then the original step reopens it (no controller + factory = open).
unsafe fn reload_window(window: usize, name: &str, out: usize, p3: usize, p4: usize) -> usize {
    // Only the creator sets the close check and the close destroys it: keep it out. Known safe: inline function pointer.
    let close_check: usize = unsafe { read(window + WINDOW_CLOSE_CHECK) };
    if close_check != 0
        && (close_check != window + WINDOW_CLOSE_CHECK_STORAGE
            || unsafe { read::<usize>(close_check) } != at(game().function_pointer_function))
    {
        warn(&format!("{name}: unusual close check kept through the reload; report it if it misbehaves"));
    }
    unsafe { write::<usize>(window + WINDOW_CLOSE_CHECK, 0) };
    let close: WindowFn = unsafe { std::mem::transmute(code(&game().window_close)) };
    unsafe { close(window, 0, 0, 0) };
    unsafe { write::<usize>(window + WINDOW_CLOSE_CHECK, close_check) };
    let result = unsafe { original::<WindowFn>(&ORIG_WINDOW_STEP)(window, out, p3, p4) };
    if unsafe { read::<usize>(window + WINDOW_CONTROLLER) } != 0 {
        log(&format!("{name}: reloaded live"));
    } else {
        warn(&format!("{name}: closed but didn't reopen; it should come back next time it opens"));
    }
    result
}
