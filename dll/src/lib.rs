//! ui-hot-reload: reload edited Armored Core VI menu movies (.gfx) in the running game. Usage: ../../README.md;
//! how it works: ../../docs/how-it-works.md.
//!
//! F7 reloads the menu files that changed in the package since the last reload; F8 turns the file watcher on/off
//! (off at start). Log: Game\debug-logs\ui_hot_reload.log.

mod files;
mod game;
mod reload;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use retour::RawDetour;

const VK_F7: i32 = 0x76;
const VK_F8: i32 = 0x77;
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

static START: OnceLock<Instant> = OnceLock::new();
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetModuleFileNameW(module: *mut u8, buffer: *mut u16, size: u32) -> u32;
}

pub fn log(line: &str) {
    let seconds = START.get().map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
    if let Some(file) = LOG.get() {
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "[{seconds:>8.1}s] {line}");
            let _ = f.flush();
        }
    }
}

/// `package = <folder>` from ui_hot_reload.ini, next to the DLL or in one of its parent folders.
fn package_from_config(dll: &Path) -> Option<PathBuf> {
    let Some(ini) = dll.ancestors().skip(1).map(|d| d.join("ui_hot_reload.ini")).find(|p| p.is_file()) else {
        log("no ui_hot_reload.ini found next to the DLL (or in a folder above it); F7/F8 do nothing");
        return None;
    };
    let text = std::fs::read_to_string(&ini).unwrap_or_default();
    let package = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.starts_with(';'))
        .filter_map(|l| l.split_once('='))
        .find(|(key, _)| key.trim() == "package")
        .map(|(_, value)| PathBuf::from(value.trim()));
    if package.is_none() {
        log(&format!("{}: no `package = ...` line; F7/F8 do nothing", ini.display()));
    }
    package
}

unsafe fn hook(addr: usize, prologue: &[u8], detour: *const (), original: &AtomicUsize) -> Result<(), String> {
    game::check_bytes(addr, prologue)?;
    let d = unsafe { RawDetour::new(addr as *const (), detour) }.map_err(|e| e.to_string())?;
    original.store(d.trampoline() as *const () as usize, Ordering::Relaxed);
    unsafe { d.enable() }.map_err(|e| e.to_string())?;
    std::mem::forget(d);
    Ok(())
}

fn install() -> Result<(), String> {
    game::check_bytes(game::WINDOW_CLOSE, &game::WINDOW_CLOSE_PROLOGUE)?;
    game::patch_disk_fallback()?;
    unsafe {
        hook(game::WINDOW_STEP, &game::WINDOW_STEP_PROLOGUE, reload::window_step_hook as *const (), &reload::ORIG_WINDOW_STEP)?;
        hook(game::PANEL_OPEN, &game::PANEL_OPEN_PROLOGUE, reload::panel_open_hook as *const (), &reload::ORIG_PANEL_OPEN)?;
        hook(game::PANEL_CLOSE, &game::PANEL_CLOSE_PROLOGUE, reload::panel_close_hook as *const (), &reload::ORIG_PANEL_CLOSE)?;
    }
    Ok(())
}

fn reload_changes(files: &mut files::MenuFiles) {
    let names = files.take_changes();
    if names.is_empty() {
        log("no changed menu files");
        return;
    }
    log(&format!("reloading {}", names.join(", ")));
    reload::request(names);
}

/// Own thread: reads the config and snapshots the package (kept out of DllMain), then polls F7/F8 and the watcher.
fn key_loop(dll: PathBuf) {
    let Some(package) = package_from_config(&dll) else { return };
    let mut files = files::MenuFiles::new(&package);
    log("ready: F7 reloads changed menu files, F8 turns the file watcher on/off (off now)");
    let mut was_down = [false; 2];
    let mut watching = false;
    let mut last_poll = Instant::now();
    loop {
        for (i, key) in [VK_F7, VK_F8].into_iter().enumerate() {
            let down = unsafe { GetAsyncKeyState(key) } as u16 & 0x8000 != 0;
            if down && !was_down[i] {
                if key == VK_F7 {
                    reload_changes(&mut files);
                } else {
                    watching = !watching;
                    log(if watching { "file watcher ON" } else { "file watcher OFF" });
                }
            }
            was_down[i] = down;
        }
        if watching && last_poll.elapsed() >= WATCH_INTERVAL {
            last_poll = Instant::now();
            if files.changes_settled() {
                reload_changes(&mut files);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn dll_path(module: *mut u8) -> PathBuf {
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buffer[..len]))
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        let _ = START.set(Instant::now());
        let logs = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("debug-logs")))
            .unwrap_or_else(|| "debug-logs".into());
        let _ = std::fs::create_dir_all(&logs);
        if let Ok(file) = std::fs::File::create(logs.join("ui_hot_reload.log")) {
            let _ = LOG.set(Mutex::new(file));
        }
        match install() {
            Ok(()) => {
                let dll = dll_path(module);
                std::thread::spawn(move || key_loop(dll));
            }
            Err(e) => log(&format!("not started: {e}")),
        }
    }
    1
}
