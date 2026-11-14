//! gfx-reloader: reload edited menu movies (.gfx) in the running game, for Armored Core VI and Elden Ring.
//! Usage: ../../README.md; how it works: ../../docs/how-it-works.md.
//!
//! F7 reloads the menu files that changed in the package since the last reload; F8 turns the file watcher on/off
//! (off at start); F9 lists the menu movies on screen. Log: Game\debug-logs\gfx_reloader.log. With
//! toast_wrapper.dll (Toast-Wrapper) loaded, results also show in the game's announce banner.

mod files;
mod game;
mod reload;
mod toast_wrapper_client;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use retour::RawDetour;

const VK_F7: i32 = 0x76;
const VK_F8: i32 = 0x77;
const VK_F9: i32 = 0x78;
/// Characters per announce-banner message: Toast-Wrapper says about 150 fit before it scrolls, minus its 18-space
/// indent to be safe.
pub const TOAST_LINE: usize = 130;
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

/// Seconds a message stays in the announce banner: status lines, and lines that take reading (warnings, lists).
const TOAST_SECONDS: f32 = 4.0;
pub const TOAST_LONG_SECONDS: f32 = 8.0;

/// Shows a one-line status message in the game's announce banner if toast_wrapper.dll is loaded (does nothing
/// otherwise). Messages queue, so keep them few.
pub fn toast(text: &str) {
    toast_for(text, TOAST_SECONDS);
}

/// `toast` with its time on screen. A toast_wrapper.dll older than 0.4 has no durations: the game's timing (about
/// 13 s) is used then.
pub fn toast_for(text: &str, seconds: f32) {
    if !toast_wrapper_client::show_for(text, seconds) {
        toast_wrapper_client::show(text);
    }
}

/// The package folder to watch, first found of: `package = <folder>` from a gfx_reloader.ini next to the DLL; the
/// `mod` folder next to the DLL (the usual mod layout); a gfx_reloader.ini in a folder above the DLL (development,
/// where the DLL sits in dll\target\release).
fn package_from_config(dll: &Path) -> Option<PathBuf> {
    let folder = dll.parent().unwrap_or(Path::new("."));
    let ini_next_to_dll = folder.join("gfx_reloader.ini");
    let ini = if ini_next_to_dll.is_file() {
        ini_next_to_dll
    } else if folder.join("mod").is_dir() {
        return Some(folder.join("mod"));
    } else if let Some(ini) = folder.ancestors().skip(1).map(|d| d.join("gfx_reloader.ini")).find(|p| p.is_file()) {
        ini
    } else {
        log(&format!(
            "no mod folder next to the DLL ({}) and no gfx_reloader.ini pointing elsewhere; F7/F8 do nothing",
            folder.join("mod").display()
        ));
        return None;
    };
    let text = std::fs::read_to_string(&ini).unwrap_or_default();
    let settings: Vec<(&str, &str)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.starts_with(';'))
        .filter_map(|l| l.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect();
    // `package_<exe name>` (e.g. package_eldenring) wins over `package`, so one ini can serve both games
    let per_game = format!("package_{}", game::game().exe.trim_end_matches(".exe"));
    let setting = |wanted: &str| settings.iter().find(|(key, _)| *key == wanted).map(|(_, value)| *value);
    let package = setting(&per_game)
        .or_else(|| setting("package"))
        // a relative path is relative to the ini's folder
        .map(|value| ini.parent().unwrap_or(Path::new(".")).join(value));
    if package.is_none() {
        log(&format!("{}: no `package = ...` line; F7/F8 do nothing", ini.display()));
    }
    package
}

/// Detours a game function (its bytes were checked by `game::detect`).
unsafe fn hook(function: &game::Code, detour: *const (), original: &AtomicUsize) -> Result<(), String> {
    let d = unsafe { RawDetour::new(game::code(function) as *const (), detour) }.map_err(|e| e.to_string())?;
    original.store(d.trampoline() as *const () as usize, Ordering::Relaxed);
    unsafe { d.enable() }.map_err(|e| e.to_string())?;
    std::mem::forget(d);
    Ok(())
}

fn install() -> Result<&'static game::Game, String> {
    let game = game::detect()?;
    unsafe {
        hook(&game.file_opener.open, reload::open_file_hook as *const (), &reload::ORIG_OPEN_FILE)?;
        hook(&game.file_opener.modify_time, reload::modify_time_hook as *const (), &reload::ORIG_MODIFY_TIME)?;
        hook(&game.window_step, reload::window_step_hook as *const (), &reload::ORIG_WINDOW_STEP)?;
        hook(&game.panel_open, reload::panel_open_hook as *const (), &reload::ORIG_PANEL_OPEN)?;
        hook(&game.player_close, reload::player_close_hook as *const (), &reload::ORIG_PLAYER_CLOSE)?;
    }
    Ok(game)
}

fn reload_changes(files: &mut files::MenuFiles) {
    let names = files.take_changes();
    if names.is_empty() {
        log("no changed menu files");
        toast("No changed menu files");
        return;
    }
    log(&format!("reloading {}", names.join(", ")));
    reload::request(names);
}

/// Own thread: reads the config and snapshots the package (kept out of DllMain), then polls F7/F8 and the watcher.
fn key_loop(dll: PathBuf) {
    let Some(package) = package_from_config(&dll) else { return };
    let mut files = files::MenuFiles::new(&package);
    log("ready: F7 reloads changed menu files, F8 turns the file watcher on/off (off now), F9 lists the menu movies on screen");
    let mut was_down = [false; 3];
    let mut watching = false;
    let mut last_poll = Instant::now();
    loop {
        for (i, key) in [VK_F7, VK_F8, VK_F9].into_iter().enumerate() {
            let down = unsafe { GetAsyncKeyState(key) } as u16 & 0x8000 != 0;
            if down && !was_down[i] {
                match key {
                    VK_F7 => reload_changes(&mut files),
                    VK_F8 => {
                        watching = !watching;
                        log(if watching { "file watcher ON" } else { "file watcher OFF" });
                        toast(if watching { "UI file watcher on" } else { "UI file watcher off" });
                    }
                    _ => reload::request_list(),
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
        if let Ok(file) = std::fs::File::create(logs.join("gfx_reloader.log")) {
            let _ = LOG.set(Mutex::new(file));
        }
        match install() {
            Ok(game) => {
                log(&format!("hooked {}", game.name));
                let dll = dll_path(module);
                std::thread::spawn(move || key_loop(dll));
            }
            Err(e) => log(&format!("not started: {e}")),
        }
    }
    1
}
