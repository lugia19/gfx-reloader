//! Per-game addresses (offsets from the exe base, ER has ASLR) and layouts, plus memory helpers.
//! See ../../docs/how-it-works.md and ../../docs/er-port.md.

use std::sync::OnceLock;

/// Offset from the exe base and expected first bytes.
pub struct Code {
    pub rva: usize,
    pub prologue: [u8; 8],
}

pub enum MapKey {
    /// AC6: wchar pointer + length.
    PointerAndLength { pointer: usize, length: usize },
    /// ER: `std::wstring`, inline while capacity < 8.
    WString { text: usize, length: usize, capacity: usize },
}

/// `CS::CSScaleformFileOpener` (vtable +8) only serves files from a store filled at boot (`Scaleform.SwfCache` on).
/// Mod files go to Scaleform's default opener instead. Both: (this, utf-8 url, flags, mode) -> file.
pub struct FileOpener {
    pub open: Code,
    pub default_open: Code,
    /// vtable +0x10, (this, url) -> time. Scaleform reuses a loaded movie when path and time match; hooked to break that.
    pub modify_time: Code,
    /// AC6 only: its store drops entries after opening, so we release them too.
    pub store: Option<Store>,
}

/// AC6 file repository: lookup (repository, wchar path) -> entry, release (system, entry). System at opener+0x18.
pub struct Store {
    pub repository: usize,
    pub lookup: Code,
    pub release: Code,
}

pub const OPENER_SYSTEM: usize = 0x18;

pub struct Game {
    pub name: &'static str,
    /// Lowercase.
    pub exe: &'static str,
    /// 16-byte entries {int layer, byte resident (+4), wchar* name (+8)}.
    pub menu_table: usize,
    pub menu_count: usize,
    /// `[[holder] + 8]` = scaleform system.
    pub scaleform_holder: usize,
    /// Movie cache std::map; node value = movie wrapper (refcount +8, vtable slot 0 destroys).
    pub cache_map_head: usize,
    pub map_key: MapKey,
    pub node_value: usize,
    /// Drawn players: fixed vector of 16-byte {player, int open count}.
    pub active_list: usize,
    pub active_count: usize,
    pub active_max: usize,
    /// `CS::MenuWindowJob`: step (vtable +0x10), close, and the vtable of a plain-function `std::function`.
    pub window_step: Code,
    pub window_close: Code,
    pub function_pointer_function: usize,
    /// `CS::MenuResource`. AC6 keeps a per-name count (menu manager +0x98), so its panels need its own close;
    /// ER has none and closes the player inline.
    pub panel_open: Code,
    pub panel_close: Option<Code>,
    pub player_close: Code,
    pub file_opener: FileOpener,
}

impl Game {
    fn functions(&self) -> Vec<&Code> {
        let opener = &self.file_opener;
        let mut functions = vec![&self.window_step, &self.window_close, &self.panel_open, &self.player_close];
        functions.extend([&opener.open, &opener.default_open, &opener.modify_time]);
        if let Some(store) = &opener.store {
            functions.extend([&store.lookup, &store.release]);
        }
        functions.extend(self.panel_close.as_ref());
        functions
    }
}

const GAMES: [Game; 2] = [
    Game {
        name: "Armored Core VI 1.09.1",
        exe: "armoredcore6.exe",
        menu_table: 0x4e28cf0,
        menu_count: 109,
        scaleform_holder: 0x4e28818,
        cache_map_head: 0x9c0,
        map_key: MapKey::PointerAndLength { pointer: 0x28, length: 0x30 },
        node_value: 0x248,
        active_list: 0x9e8,
        active_count: 0xdf0,
        active_max: 64,
        window_step: Code { rva: 0xd61af0, prologue: [0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56] },
        window_close: Code { rva: 0x641c16c, prologue: [0x40, 0x55, 0x56, 0x57, 0x48, 0x8b, 0xec, 0x48] },
        function_pointer_function: 0x390e4f0,
        panel_open: Code { rva: 0xc8acf0, prologue: [0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xc0] },
        panel_close: Some(Code { rva: 0xc8aff0, prologue: [0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b] }),
        player_close: Code { rva: 0x1606b10, prologue: [0x40, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55] },
        file_opener: FileOpener {
            open: Code { rva: 0x15f40b0, prologue: [0x40, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57] },
            default_open: Code { rva: 0x1a7bfb0, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c] },
            modify_time: Code { rva: 0x1a7c080, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74] },
            store: Some(Store {
                repository: 0x4e245b0,
                lookup: Code { rva: 0x153d3d0, prologue: [0x48, 0x83, 0xec, 0x38, 0x48, 0x89, 0x54, 0x24] },
                release: Code { rva: 0x16040c0, prologue: [0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83] },
            }),
        },
    },
    Game {
        name: "Elden Ring (Steam build 25827938)",
        exe: "eldenring.exe",
        menu_table: 0x3b443f0,
        menu_count: 106,
        scaleform_holder: 0x3d8a1d8,
        cache_map_head: 0x9b0,
        map_key: MapKey::WString { text: 0x228, length: 0x238, capacity: 0x240 },
        node_value: 0x250,
        active_list: 0x9d8,
        active_count: 0xbe0,
        active_max: 32,
        window_step: Code { rva: 0x7ae4f0, prologue: [0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56] },
        window_close: Code { rva: 0x7aed70, prologue: [0x40, 0x55, 0x56, 0x57, 0x48, 0x8d, 0x6c, 0x24] },
        function_pointer_function: 0x2aae9b0,
        panel_open: Code { rva: 0x74aa40, prologue: [0x40, 0x53, 0x48, 0x81, 0xec, 0xa0, 0x00, 0x00] },
        panel_close: None,
        player_close: Code { rva: 0xd7bc40, prologue: [0x40, 0x55, 0x56, 0x57, 0x48, 0x81, 0xec, 0x80] },
        file_opener: FileOpener {
            open: Code { rva: 0xd6e970, prologue: [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41] },
            default_open: Code { rva: 0x1159f50, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c] },
            modify_time: Code { rva: 0x115a020, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74] },
            store: None,
        },
    },
];

// Shared by both games. Window: name, factory and close check (`std::function` storage / impl), controller.
pub const WINDOW_NAME: usize = 0x60;
pub const WINDOW_FACTORY: usize = 0xa8;
pub const WINDOW_CLOSE_CHECK_STORAGE: usize = 0xf0;
pub const WINDOW_CLOSE_CHECK: usize = 0x128;
pub const WINDOW_CONTROLLER: usize = 0x130;
pub const PANEL_NAME: usize = 0x50;
pub const PANEL_PLAYER: usize = 0x60;
/// In the player's menu table entry copy (+0x20).
const PLAYER_NAME: usize = 0x28;

static GAME: OnceLock<&'static Game> = OnceLock::new();
static BASE: OnceLock<usize> = OnceLock::new();

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
}

pub fn game() -> &'static Game {
    GAME.get().expect("game not detected")
}

pub fn at(rva: usize) -> usize {
    BASE.get().expect("game not detected") + rva
}

pub fn code(code: &Code) -> usize {
    at(code.rva)
}

pub fn detect() -> Result<&'static Game, String> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let game = GAMES.iter().find(|g| g.exe == exe).ok_or_else(|| format!("unsupported game ({exe})"))?;
    let _ = BASE.set(unsafe { GetModuleHandleW(std::ptr::null()) });
    let _ = GAME.set(game);
    for function in game.functions() {
        // another mod's jmp hook is fine
        if unsafe { read::<u8>(code(function)) } != 0xe9 {
            check_bytes(code(function), function.prologue)?;
        }
    }
    Ok(game)
}

fn check_bytes<const N: usize>(addr: usize, expected: [u8; N]) -> Result<(), String> {
    let actual: [u8; N] = unsafe { read(addr) };
    if actual != expected {
        return Err(format!("unexpected code at {addr:#x} (not {}?): {actual:02x?}", game().name));
    }
    Ok(())
}

// ---- memory helpers -------------------------------------------------------------------------------------------------

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

pub unsafe fn name_at(addr: usize) -> String {
    unsafe { wide_str(read::<usize>(addr) as *const u16, 128) }
}

fn menu_entry(index: usize) -> usize {
    at(game().menu_table) + index * 16
}

pub fn menu_names() -> Vec<String> {
    (0..game().menu_count).map(|i| unsafe { name_at(menu_entry(i) + 8) }).collect()
}

pub fn scaleform_system() -> usize {
    let holder: usize = unsafe { read(at(game().scaleform_holder)) };
    if holder == 0 { 0 } else { unsafe { read(holder + 8) } }
}

/// Game thread.
pub unsafe fn active_menu_names(system: usize) -> Vec<String> {
    let game = game();
    let count: usize = unsafe { read(system + game.active_count) };
    let base = (system + game.active_list + 7) & !7;
    (0..count.min(game.active_max))
        .map(|i| unsafe { read::<usize>(base + i * 16) })
        .filter(|player| *player != 0)
        .map(|player| unsafe { name_at(player + PLAYER_NAME) })
        .collect()
}

unsafe fn node_key(node: usize) -> String {
    match game().map_key {
        MapKey::PointerAndLength { pointer, length } => {
            let len: usize = unsafe { read(node + length) };
            unsafe { wide_str(read::<usize>(node + pointer) as *const u16, len.min(256)) }
        }
        MapKey::WString { text, length, capacity } => {
            let len: usize = unsafe { read(node + length) };
            let inline = unsafe { read::<usize>(node + capacity) } < 8;
            let chars = if inline { node + text } else { unsafe { read::<usize>(node + text) } };
            unsafe { wide_str(chars as *const u16, len.min(256)) }
        }
    }
}

/// MSVC std::map node: left +0, parent +8, right +0x10, is-nil +0x19.
pub unsafe fn cache_node(system: usize, name: &str) -> Option<usize> {
    let head: usize = unsafe { read(system + game().cache_map_head) };
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
        if unsafe { node_key(node) } == name {
            return Some(node);
        }
        stack.push(unsafe { read(node) });
        stack.push(unsafe { read(node + 0x10) });
    }
    None
}
