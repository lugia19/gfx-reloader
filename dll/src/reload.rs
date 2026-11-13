//! Reloading menus. Requests come from the key/watcher thread; everything that touches the game runs on the game
//! thread, inside the self-owned windows' step (called every frame for every live window).
//!
//! Per requested menu:
//! 1. its cached movie is released and any leftover repository entry for its file is dropped, so the next load reads
//!    the file from disk;
//! 2. live embedded panels with that movie are closed and reopened (new art, but their owner keeps talking to the old
//!    movie: preview only);
//! 3. a live self-owned window with that movie is closed with its own close and reopened by its step in the same
//!    frame, keeping its creator's close check;
//! 4. if no window picked it up once every live window has stepped (the first window to step after the request steps
//!    again), the menu isn't on screen: it loads the new file next time.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::game::*;
use crate::log;

type WindowFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type RepositoryLookupFn = unsafe extern "system" fn(usize, *const u16) -> usize;
type RepositoryReleaseFn = unsafe extern "system" fn(usize, usize) -> usize;
type MenuPathFn = unsafe extern "system" fn(*mut u8, usize) -> usize;

pub static ORIG_WINDOW_STEP: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PANEL_OPEN: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PANEL_CLOSE: AtomicUsize = AtomicUsize::new(0);

/// Set while there are requests or menus waiting for their window, so the per-frame hook does nothing otherwise.
static PENDING: AtomicBool = AtomicBool::new(false);
/// Menu names asked for by F7 / the watcher, not yet handled on the game thread.
static REQUESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Menus waiting for their live self-owned window to step, until every live window has had its step.
static ROUND: Mutex<Option<Round>> = Mutex::new(None);
/// Live embedded panels: (panel window, menu name), kept by the panel open/close hooks.
static PANELS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

/// Only in case the round's first window closes and never steps again.
const ROUND_SAFETY_CAP: Duration = Duration::from_secs(2);

struct Waiting {
    name: String,
    /// A panel already showed the new movie (so "not on screen" would be wrong).
    previewed: bool,
}

struct Round {
    /// The window that stepped when the requests were handled: when it steps again, every live window has stepped.
    first_window: usize,
    started: Instant,
    waiting: Vec<Waiting>,
}

pub fn request(names: Vec<String>) {
    let mut requests = REQUESTS.lock().unwrap();
    requests.extend(names);
    PENDING.store(true, Ordering::Relaxed);
}

fn original(slot: &AtomicUsize) -> WindowFn {
    unsafe { std::mem::transmute(slot.load(Ordering::Relaxed)) }
}

// ---- hooks ----------------------------------------------------------------------------------------------------------

/// Self-owned window step (0x140d61af0), every live window, every frame.
pub unsafe extern "system" fn window_step_hook(window: usize, out: usize, p3: usize, p4: usize) -> usize {
    if PENDING.load(Ordering::Relaxed) {
        if let Some(name) = unsafe { next_reload(window) } {
            return unsafe { reload_window(window, &name, out, p3, p4) };
        }
    }
    unsafe { original(&ORIG_WINDOW_STEP)(window, out, p3, p4) }
}

pub unsafe extern "system" fn panel_open_hook(panel: usize, a: usize, b: usize, c: usize) -> usize {
    let result = unsafe { original(&ORIG_PANEL_OPEN)(panel, a, b, c) };
    let mut panels = PANELS.lock().unwrap();
    panels.retain(|(p, _)| *p != panel);
    if unsafe { read::<usize>(panel + PANEL_PLAYER) } != 0 {
        panels.push((panel, unsafe { name_at(panel + PANEL_NAME) }));
    }
    result
}

pub unsafe extern "system" fn panel_close_hook(panel: usize, a: usize, b: usize, c: usize) -> usize {
    PANELS.lock().unwrap().retain(|(p, _)| *p != panel);
    unsafe { original(&ORIG_PANEL_CLOSE)(panel, a, b, c) }
}

// ---- game-thread work -----------------------------------------------------------------------------------------------

/// Game thread, while something is pending: ends the current round if every live window has stepped, handles new
/// requests, and returns the menu to reload if `window` is the live window one of them is waiting for.
unsafe fn next_reload(window: usize) -> Option<String> {
    let mut round = ROUND.lock().unwrap();
    if round.as_ref().is_some_and(|r| r.first_window == window || r.started.elapsed() > ROUND_SAFETY_CAP) {
        for menu in round.take().unwrap().waiting {
            if !menu.previewed {
                log(&format!("{}: not on screen right now; the new file is used the next time it opens", menu.name));
            }
        }
    }
    let new = unsafe { handle_requests() };
    if !new.is_empty() {
        round.get_or_insert_with(|| Round { first_window: window, started: Instant::now(), waiting: Vec::new() })
            .waiting
            .extend(new);
    }
    let Some(current) = round.as_mut() else {
        // nothing waiting: go idle unless a request arrived meanwhile (checked under the lock request() uses)
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

/// Releases each requested movie and reloads its live panels; returns the menus that now wait for their window.
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
    requests
        .into_iter()
        .map(|name| {
            unsafe { release_cached(system, &name) };
            unsafe { drop_repository_leftovers(system, &name) };
            let previewed = unsafe { reload_panels(&name) };
            Waiting { name, previewed }
        })
        .collect()
}

/// Drops the cache's reference to a menu's movie (refcount at +8; at zero, vtable slot 0 destroys it) and empties the
/// entry, so the next lookup loads the file again. The old movie must really go, or Scaleform hands it back.
unsafe fn release_cached(system: usize, name: &str) {
    let Some(node) = (unsafe { cache_node(system, name) }) else { return };
    let wrapper: usize = unsafe { read(node + NODE_VALUE) };
    if wrapper == 0 {
        return;
    }
    unsafe { write::<usize>(node + NODE_VALUE, 0) };
    let refcount = unsafe { &*((wrapper + 8) as *const std::sync::atomic::AtomicI32) };
    if refcount.fetch_sub(1, Ordering::SeqCst) == 1 {
        let destroy: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(read::<usize>(read::<usize>(wrapper))) };
        unsafe { destroy(wrapper) };
    }
}

/// Boot requests some files twice (01_070_CommandList, 01_900_Black) and opens them once; the second copy stays in
/// the repository with an already-used file object. Loading that instead of the disk file kills the game.
unsafe fn drop_repository_leftovers(system: usize, name: &str) {
    let repository: usize = unsafe { read(REPOSITORY) };
    let Some(index) = menu_index(name) else { return };
    if repository == 0 {
        return;
    }
    // the game's string object: a generous zeroed buffer, leaked (a few bytes per reload)
    let buffer = Box::leak(vec![0u8; 0x200].into_boxed_slice());
    let make_path: MenuPathFn = unsafe { std::mem::transmute(MENU_PATH) };
    unsafe { make_path(buffer.as_mut_ptr(), MENU_TABLE + index * 16) };
    let path = usize::from_le_bytes(buffer[8..16].try_into().unwrap()) as *const u16;
    if path.is_null() {
        return;
    }
    let lookup: RepositoryLookupFn = unsafe { std::mem::transmute(REPOSITORY_LOOKUP) };
    let release: RepositoryReleaseFn = unsafe { std::mem::transmute(REPOSITORY_RELEASE) };
    for _ in 0..8 {
        let entry = unsafe { lookup(repository, path) };
        if entry == 0 {
            return;
        }
        unsafe { release(system, entry) };
    }
    log(&format!("{name}: warning: couldn't clear its leftover boot copy; the reload may show the old file"));
}

/// Closes and reopens the live panels showing this movie. Their owners took handles into the old movie when they
/// were built, so text, data and input stay with the old (now invisible) one.
unsafe fn reload_panels(name: &str) -> bool {
    let panels: Vec<usize> =
        PANELS.lock().unwrap().iter().filter(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(p, _)| *p).collect();
    if panels.is_empty() {
        return false;
    }
    for panel in &panels {
        unsafe { original(&ORIG_PANEL_CLOSE)(*panel, 0, 0, 0) };
        unsafe { original(&ORIG_PANEL_OPEN)(*panel, 0, 0, 0) };
    }
    let rebuilt_by = if PERSISTENT_PANELS.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        "returning to the title screen"
    } else {
        "the next scene change (entering or leaving a mission or AC TEST)"
    };
    log(&format!(
        "{name}: panel reloaded as a PREVIEW: new shapes and colors, but its text, data and input stay with the old \
         movie until it's rebuilt by {rebuilt_by}"
    ));
    true
}

/// Closes the window with its own close, then lets the original step reopen it (no controller + factory = open), which
/// looks the movie up again: the cache is empty, so the file is read from disk.
unsafe fn reload_window(window: usize, name: &str, out: usize, p3: usize, p4: usize) -> usize {
    // The close check is set by the window's creator and the reopen never sets it again; the close would destroy it.
    // Keep it out of the close. Only a plain function pointer stored inline is known to be safe to keep.
    let close_check: usize = unsafe { read(window + WINDOW_CLOSE_CHECK) };
    if close_check != 0
        && (close_check != window + WINDOW_CLOSE_CHECK_STORAGE
            || unsafe { read::<usize>(close_check) } != FUNCTION_POINTER_FUNCTION)
    {
        log(&format!("{name}: warning: unusual close check kept through the reload; report this if it misbehaves"));
    }
    unsafe { write::<usize>(window + WINDOW_CLOSE_CHECK, 0) };
    let close: WindowFn = unsafe { std::mem::transmute(WINDOW_CLOSE) };
    unsafe { close(window, 0, 0, 0) };
    unsafe { write::<usize>(window + WINDOW_CLOSE_CHECK, close_check) };
    let result = unsafe { original(&ORIG_WINDOW_STEP)(window, out, p3, p4) };
    if unsafe { read::<usize>(window + WINDOW_CONTROLLER) } != 0 {
        log(&format!("{name}: reloaded live"));
    } else {
        log(&format!("{name}: warning: closed but didn't reopen; it should come back the next time it opens"));
    }
    result
}
