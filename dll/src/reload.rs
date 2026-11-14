//! Reloading menus. Requests come from the key/watcher thread; everything that touches the game runs on the game
//! thread, inside the self-owned windows' step (called every frame for every live window).
//!
//! Per requested menu:
//! 1. its cached movie is released, so the next load opens the file again, and `open_file_hook` makes that open read
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
use crate::{TOAST_LINE, TOAST_LONG_SECONDS, log, toast, toast_for};

/// Window step, window close, panel open, panel close: (object, three more arguments).
type WindowFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
/// Scaleform file open: (opener, utf-8 path, flags, mode) -> file.
type OpenFileFn = unsafe extern "system" fn(usize, *const u8, usize, usize) -> usize;
type StoreLookupFn = unsafe extern "system" fn(usize, *const u16) -> usize;
type StoreReleaseFn = unsafe extern "system" fn(usize, usize) -> usize;
type PlayerCloseFn = unsafe extern "system" fn(usize, usize) -> usize;

pub static ORIG_WINDOW_STEP: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PANEL_OPEN: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_PLAYER_CLOSE: AtomicUsize = AtomicUsize::new(0);
pub static ORIG_OPEN_FILE: AtomicUsize = AtomicUsize::new(0);

/// Whether a file opener path points into the game's archives: a named root, then `:/` (`data0:/menu/x.gfx`). A mod
/// loader replaces the path of a file it overrides (me3: a token like `\\me3??54`), so anything else is a mod's
/// file, the only kind that can have been edited. A one-letter root is a drive (`C:/...`), not an archive.
fn is_archive_path(path: &str) -> bool {
    path.split_once(":/").is_some_and(|(root, _)| root.len() >= 2 && root.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Set while there are requests or menus waiting for their window, so the per-frame hook does nothing otherwise.
static PENDING: AtomicBool = AtomicBool::new(false);
/// F9: list the menu movies on screen at the next window step.
static LIST_REQUESTED: AtomicBool = AtomicBool::new(false);
/// Menu names asked for by F7 / the watcher, not yet handled on the game thread.
static REQUESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Menus waiting for their live self-owned window to step, until every live window has had its step.
static ROUND: Mutex<Option<Round>> = Mutex::new(None);
/// Live embedded panels, added when a panel opens and dropped when its player closes (every panel close, and every
/// panel destruction, goes through the player close; Elden Ring has no separate panel close to hook).
static PANELS: Mutex<Vec<Panel>> = Mutex::new(Vec::new());

struct Panel {
    panel: usize,
    player: usize,
    name: String,
}

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
    /// For the summary toast: menus handled, and how many of them only as a panel preview.
    menus: usize,
    previews: usize,
}

pub fn request(names: Vec<String>) {
    let mut requests = REQUESTS.lock().unwrap();
    requests.extend(names);
    PENDING.store(true, Ordering::Relaxed);
}

pub fn request_list() {
    let _requests = REQUESTS.lock().unwrap(); // same lock as request(), so the idle check can't miss it
    LIST_REQUESTED.store(true, Ordering::Relaxed);
    PENDING.store(true, Ordering::Relaxed);
}

/// Logs a warning and shows it as a toast.
fn warn(text: &str) {
    log(&format!("warning: {text}"));
    toast_for(text, TOAST_LONG_SECONDS);
}

/// "Reloaded 3 menus (1 preview only, broken until the next scene change)".
fn round_summary(round: &Round) -> String {
    let menus = if round.menus == 1 { "1 menu".to_string() } else { format!("{} menus", round.menus) };
    if round.previews == 0 {
        return format!("Reloaded {menus}");
    }
    format!("Reloaded {menus} ({} preview only, broken until the next scene change)", round.previews)
}

/// F9, game thread: every menu movie on screen, panels marked, packed into numbered one-line toasts.
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
    // pack into lines, leaving room for the "UI on screen (n/m): " prefix
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

/// The original of a hooked function (its trampoline, stored by `hook`), as the function type `F`.
fn original<F: Copy>(slot: &AtomicUsize) -> F {
    assert_eq!(size_of::<F>(), size_of::<usize>());
    unsafe { std::mem::transmute_copy(&slot.load(Ordering::Relaxed)) }
}

// ---- hooks ----------------------------------------------------------------------------------------------------------

/// Self-owned window step, every live window, every frame.
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

/// The scaleform system's player close (system, player): a panel whose player closes is no longer live.
pub unsafe extern "system" fn player_close_hook(system: usize, player: usize) -> usize {
    PANELS.lock().unwrap().retain(|p| p.player != player);
    unsafe { original::<PlayerCloseFn>(&ORIG_PLAYER_CLOSE)(system, player) }
}


/// The game's scaleform file open (`FileOpener`): a mod's files (not an archive path) are read with the default opener
/// instead of the game's store, so a released movie loads the current file. Everything else is left to the game.
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

/// What the game's opener does after opening a file from its store: release the file's store entry. Done here for
/// a file read from disk instead, so the store ends up as the game expects (every entry gone once its file has been
/// opened). Nothing to do where the store keeps its entries.
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

// ---- game-thread work -----------------------------------------------------------------------------------------------

/// Game thread, while something is pending: ends the current round if every live window has stepped, handles new
/// requests, and returns the menu to reload if `window` is the live window one of them is waiting for.
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
            let previewed = unsafe { reload_panels(system, &name) };
            Waiting { name, previewed }
        })
        .collect()
}

/// Drops the cache's reference to a menu's movie (refcount at +8; at zero, vtable slot 0 destroys it) and empties the
/// entry, so the next lookup loads the file again. The old movie must really go, or Scaleform hands it back.
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

/// Closes and reopens the live panels showing this movie. Their owners took handles into the old movie when they
/// were built, so text, data and input stay with the old (now invisible) one.
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
    // Through the game's own (hooked) functions, so the registry follows: the close drops the entry, the open adds it.
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

/// Closes a panel's player: with the game's panel close (which may do its own bookkeeping, see `Game::panel_close`),
/// or, where the game has no such function (it closes panels inline), the same way by hand: close the player, clear
/// the panel's field.
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

/// Closes the window with its own close, then lets the original step reopen it (no controller + factory = open), which
/// looks the movie up again: the cache is empty, so the file is read from disk.
unsafe fn reload_window(window: usize, name: &str, out: usize, p3: usize, p4: usize) -> usize {
    // The close check is set by the window's creator and the reopen never sets it again; the close would destroy it.
    // Keep it out of the close. Only a plain function pointer stored inline is known to be safe to keep.
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
