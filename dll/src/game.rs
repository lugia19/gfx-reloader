//! The supported games: where each piece lives (offsets from the exe base, since Elden Ring's exe can be moved by
//! ASLR) and how its structures are laid out, plus small memory helpers. How the pieces fit together:
//! ../../docs/how-it-works.md; how the Elden Ring side was matched: ../../docs/er-port.md.

use std::sync::OnceLock;

/// A game function: offset from the exe base, and its first bytes (checked before use; another mod's `jmp` hook is
/// accepted too).
pub struct Code {
    pub rva: usize,
    pub prologue: [u8; 8],
}

/// How a scaleform map node stores its key (a menu name).
pub enum MapKey {
    /// wchar pointer and length (AC6).
    PointerAndLength { pointer: usize, length: usize },
    /// `std::wstring` (Elden Ring): the characters are inline while the capacity is below 8, else behind a pointer.
    WString { text: usize, length: usize, capacity: usize },
}

/// The game's scaleform file opener (`CS::CSScaleformFileOpener`, vtable +8) serves menu files from a store filled at
/// boot and never reads an edited file: Elden Ring's store (`CS::GfxRepositoryImp`) keeps every file and there is no
/// disk read on that path; AC6's empties as movies open and its disk read is switched off. The tool hooks the opener
/// and sends every path that isn't an archive path (a mod's file) to Scaleform's default opener, which reads the file
/// itself. Both take (this, utf-8 url, flags, mode) -> file.
pub struct FileOpener {
    pub open: Code,
    pub default_open: Code,
    /// The opener's "modification time of this file" (vtable +0x10; Scaleform's own, whole seconds): (this, utf-8
    /// url) -> time. Scaleform identifies a loaded movie by its path and this time, and hands back the movie it has
    /// when both match, without opening the file. The tool hooks it so a reloaded file never matches.
    pub modify_time: Code,
    /// Where the game's opener releases a file's store entry after opening it (AC6), so the hook can do the same for
    /// the files it reads from disk. None where the store keeps its entries anyway (Elden Ring).
    pub store: Option<Store>,
}

/// AC6's in-memory file repository: its pointer, the lookup (repository, wchar path) -> entry, and the release of
/// one reference (system, entry). The opener keeps the scaleform system at +0x18.
pub struct Store {
    pub repository: usize,
    pub lookup: Code,
    pub release: Code,
}

pub const OPENER_SYSTEM: usize = 0x18;

pub struct Game {
    pub name: &'static str,
    /// The exe's file name, lowercase.
    pub exe: &'static str,
    /// Menu table: entries of 16 bytes {int layer, byte resident (+4), wchar* name (+8)}, filled at startup.
    pub menu_table: usize,
    pub menu_count: usize,
    /// `[[scaleform_holder] + 8]` = the scaleform system.
    pub scaleform_holder: usize,
    /// The system's movie cache: std::map head node pointer, node key layout, cached movie wrapper in the node
    /// (refcount +8, vtable slot 0 destroys it).
    pub cache_map_head: usize,
    pub map_key: MapKey,
    pub node_value: usize,
    /// Active players (what gets drawn): fixed vector of {player, int open count} (16 bytes), its count and capacity.
    pub active_list: usize,
    pub active_count: usize,
    pub active_max: usize,
    /// Self-owned windows (`CS::MenuWindowJob`): per-frame step (vtable +0x10), the window's own close, and the
    /// vtable of a `std::function` holding a plain function pointer (the usual control check).
    pub window_step: Code,
    pub window_close: Code,
    pub function_pointer_function: usize,
    /// Embedded panels (`CS::MenuResource`): open; their close where the game has one; the scaleform system's player
    /// close (system, player), which every panel close goes through. AC6's open and close also keep a per-name count
    /// in the menu manager (map at +0x98), so its panels must be closed with its close; Elden Ring has no such count
    /// and closes the player inline.
    pub panel_open: Code,
    pub panel_close: Option<Code>,
    pub player_close: Code,
    pub file_opener: FileOpener,
}

impl Game {
    /// Every game function the tool hooks or calls through a `Code` (checked by `detect`).
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

// Layouts shared by both games.
/// Self-owned window: menu name pointer, open factory (`std::function` impl pointer), control check (`std::function`
/// `<MenuWindowJob::MenuCtrlType()>`: inline storage, impl pointer), controller.
pub const WINDOW_NAME: usize = 0x60;
pub const WINDOW_FACTORY: usize = 0xa8;
pub const WINDOW_CLOSE_CHECK_STORAGE: usize = 0xf0;
pub const WINDOW_CLOSE_CHECK: usize = 0x128;
pub const WINDOW_CONTROLLER: usize = 0x130;
/// Embedded panel: menu name pointer, player.
pub const PANEL_NAME: usize = 0x50;
pub const PANEL_PLAYER: usize = 0x60;
/// A player keeps its menu table entry copy at +0x20, name pointer +0x28.
const PLAYER_NAME: usize = 0x28;

static GAME: OnceLock<&'static Game> = OnceLock::new();
static BASE: OnceLock<usize> = OnceLock::new();

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
}

/// The detected game (after `detect`).
pub fn game() -> &'static Game {
    GAME.get().expect("game not detected")
}

/// Address of an offset from the exe base.
pub fn at(rva: usize) -> usize {
    BASE.get().expect("game not detected") + rva
}

/// Address of a game function.
pub fn code(code: &Code) -> usize {
    at(code.rva)
}

/// Finds the running game by its exe and checks the functions the tool hooks or calls (`Game::functions`).
pub fn detect() -> Result<&'static Game, String> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let game = GAMES.iter().find(|g| g.exe == exe).ok_or_else(|| format!("unsupported game ({exe})"))?;
    let _ = BASE.set(unsafe { GetModuleHandleW(std::ptr::null()) });
    let _ = GAME.set(game);
    for function in game.functions() {
        // another mod's hook (a `jmp`) at the start is fine
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

/// Name stored behind a wchar pointer at `addr` (window/panel name fields, menu table entries).
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

/// Menu names of every active player, i.e. every menu movie on screen (game thread).
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

/// The movie cache node for a menu (MSVC std::map: left +0, parent +8, right +0x10, is-nil byte +0x19).
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
