# gfx-reloader: research notes (2026-11-13)

Everything found while building the tool, in the order it was found. Short version: [how-it-works.md](how-it-works.md).

Goal: edit a menu `.gfx` in an me3 package, press a key, and the game uses the edited movie without restarting.
Starting point: `AC6_Projects\hud-color\notes.md` (how menu movies load, the file repository, the disk fallback).
AC6 1.09.1, me3 0.13.0. Most of this was found with the research probe (`probe-dll\`, see
[development.md](development.md)); "the probe" below means it. Note: me3 overriding `02_coloring.gfx` breaks the Paint
pattern thumbnails (`AC6_tools\Research\paint-gfx-path`), unrelated to the reload.

## What the code says about players (Ghidra, 2026-11-13)

- Menu open `0x1416069a0(system, entry, flag)`: lookup, then `0x140e60235`: the player map (`system+0x9d0`, head
  `+0x9d8`, same node layout, player at `node+0x248`) is searched by name; **an existing player is reused and the
  freshly looked-up movie is just unreferenced**; only a missing one gets a new 0xe8 player built from it. Then
  `0x141608170` adds the player to the active list (`system+0x9e8`, {player, open count}, count `+0xdf0`, max 64)
  or bumps its count, and `0x1415fb660(player, 0)` starts it once (flag `player+0x78`).
- So a reload only shows when the menu's player is gone from the player map. Expected: the HUD's is (rebuilt each
  sortie), long-lived menus' aren't. Who removes players from the map is the next thing to find.

## Results (2026-11-13, first session, `disk_fallback = on_reload`)

- Boot: 109 requests, every movie opened from the repository; only miss `02_gaitemstatus_general_minimum.gfx`
  (fallback off then). Fine with the probe's hooks.
- **Menus that close and reopen reload fully**: F7, back out and re-enter -> release (cache held the last
  reference, wrapper destroyed), OpenFile from disk, new MovieDef, shown tinted. Tested: Assembly `02_010_EquipTop`
  (also carries the 3D AC view: the AC turns green), `02_058_DetailStatus_Player_2P` (compact AC SPECS),
  `02_059_DetailStatus_Player_3P` (full specs, Y). `02_050_DetailStatus_Player` and
  `02_DetailStatus_Player_AssemInfo` reloaded too but show nothing visible on that screen.
- **HUD** `01_000_FE`: tinted red, F7, AC TEST -> red HUD.
- **Long-lived menus don't reload until really reopened**: `01_GarageTopMenu` stays open the whole garage session
  (refcount 2: cache + its live player). F7 released only the cache's reference ("1 other owner keeps it alive"); going
  into Paint/Assembly and back reopens only the submenus. After AC TEST and back to the garage it was reopened ->
  read from disk -> cyan. So the cache side is solved; what's left is forcing a live menu to be rebuilt.
- Probe bug: the player map read at `system+0x9d8` always comes back empty (0 entries, "map had none" on every open,
  though the active list has 18 players), so the "new/reused" label means nothing yet. Players come back at the
  same addresses on reopen (pool reuse or real reuse: unknown). Fix the map location before relying on it.

## Live menus: window close + reopen (2026-11-13, works for main-menu windows)

- Players are owned by **menu windows**. Two families seen:
  - **Main-menu windows** (vtable `0x14390e498`; `01_GarageTopMenu`, `02_010_EquipTop`, `02_ColoringTop`,
    `01_GarageSubMenu`...): entry copy `+0x58` (name `+0x60`), player `+0x68`, movie instance `+0x10`, controller
    `+0x130` (built by the open factory `+0xa8`), command object `+0x128`. Slot `+0x10` `0x140d61af0` is the per-frame
    step: no controller + factory -> open (`0x140d61b3e`); controller done -> close. Close `0x14641c16c(window)`
    disposes the controller, closes the player, destroys `+0x128`, keeps the factory.
  - **Panel windows** (vtable `0x1438e5238`; the `02_0xx_DetailStatus_*` panels): entry `+0x48` (name `+0x50`),
    player `+0x60`, movie `+0x8`; open `0x140c8acf0`, close `0x140c8aff0`. Not handled yet (they reopen with their
    parent anyway).
- The probe hooks the step: a live window with a pending reload is closed with its own close, then the original step
  reopens it in the same frame (fresh lookup -> release -> read from disk). Tested on `01_GarageTopMenu` while it was
  on screen: cyan instantly, cursor / submenus / system menu all still work. `+0x128` stays 0 after the reopen and
  nothing seems to miss it.
- **The close check at `+0xf0..+0x130`** is an MSVC `std::function` (inline storage `+0xf0`, vtable `0x14390e4f0` =
  copy/move/call/type/delete; callable `+0xf8`; impl pointer `+0x128`), set by the job that builds the window (the
  garage job `0x140d79670` passes `0x140d62120`). The step calls it each frame; on 1 it sends the controller event 3
  (`0x140d61410`, controller vfunc `0xb8`). `0x140d62120` = `CSMenuMan(0x144e15530)->0x140cd2a70() == 0`, i.e. true
  when `CSMenuMan+0x748` is set and `CSMenuMan+0x10` is 0. Polled live: `+0x10`/`+0x748` are 1/1 in the garage,
  system menu, license menu, photo mode, Assembly; `+0x10` dropped to 0 for ~2 s only during the AC TEST sortie
  load (between `02_NowLoading_Sortie` opening and closing). So: "menus may stay open" flag, a safety net for live
  windows caught by a scene transition.
  **Confirmed (watchpoint + code + singleton names):** `+0x10` is written by the setter `0x140cd3510`, called every
  frame by `0x141276e40` (part of the world update `0x1412742d0`) from **CSRemo** (`0x144e17a80`, the real-time
  cutscene system; name from `Research\Characters and positions\singletons_1.09.1.txt`, which also names
  `0x144e15530` CSMenuMan): no cutscene (CSRemo's current-cutscene flags `+0x150 & 0x100001` clear) -> 1; cutscene
  playing -> `0x1411dd510()`: 1 while its timer `+0x144` is inside the `+0x148` window or flag bit 7 is set, else 0.
  `+0x11` gets the same value. `0x14125be50` and `0x141284ea9` also set it to 1 on state start-up. Hardware
  watchpoint (DR0, 1-byte write) caught 1 -> 0 -> 1 around the AC TEST load, matching the poll. So the close check
  means "a cutscene is taking over: live menus get out of the way".
  The window close destroys it and the reopen never recreates it (it belongs to the job), so the reload keeps it:
  `+0x128` is nulled before the close and restored after. Verified: reload, then AC TEST and back, all normal.
- `CSMenuMan` (`[0x144e15530]`, vtable `0x1438f2e88`) matches ER's `CSMenuManImp` (fromsoftware-rs): `ui_states`
  bytes at `+0x30` (bit 0 created, bit 1 visible; window open sets bit 0, close clears it), a per-name open-window
  count map at `+0x98`.
- `probe-dll\tools\find_window.py <menu>` finds a menu's window in the running game (scans for the player pointer from the log).
- Break test F10 (close every active player): menus vanish (the active list is the draw list), their windows keep
  running blind, nothing reopens; pause menu dead. Not a reload path.
- The player map (`system+0x9d8`) only holds resident == 2 menus; ordinary menus get a new player on every open (so
  the probe's empty map was right).

## Which live menus use which window class (2026-11-13, from the open call stacks)

(Better names, used from here on: **self-owned windows** = what the code calls "main-menu windows" below, nothing to
do with the main menu: each builds its own controller on open, so close + reopen fully reloads it; **embedded
panels** = panel windows, owned by some other C++ object that keeps handles into the movie.)

- **Main-menu windows** (open from `0x140d61b8c`): `01_000_FE` (HUD), `01_010_MessageBox`, `01_060_Caption`,
  `01_090_SummonMessage`, `01_900_Black`, `01_FadeScreen_*`, `01_GarageSubMenu`, `01_GarageTerminal` (garage frame:
  title, COAM bar, description strip), `01_GarageTopMenu`, `01_LetterBox`, `01_MainMenu`, `01_SystemAnnounce`,
  `01_SystemToast`, `01_VirtualBG`, `02_010_EquipTop`, the loading screens, `05_000_Title`.
  - Seen in game after a live reload with an obvious tint (`tint.py --add` colors black art too): `01_900_Black` =
    the full-screen black used when leaving AC TEST (showed fully red). `01_VirtualBG` is NOT the grid behind
    Assembly (tinted it, no change; tiny 1.2 KB movie, boot log "Failed to load image MENU_DummyVirtualBG.tga": likely
    a placeholder the engine fills with a texture); the grid is probably `01_MenuCommonBG` (panel). AC TEST's
    entrance black, and its "MAIN SYSTEM" boot sequence, are none of the persistent windows. Fades, letterbox,
    captions and toast never showed in the garage / AC TEST flow (need a cutscene or comms to check). The step hook reloads them live: tested GarageTopMenu, GarageTerminal, Caption, VirtualBG at once
  (each read from disk, new MovieDef). Caption and VirtualBG carry a different close check, callable
  `0x140d626f0` (also an inline function pointer; not looked at yet).
- **Panel windows** (open `0x140c8acf0` from `0x140c8ad75`): `01_002_FE_SaveIcon`, `01_FE_Log`, `01_MenuCommonBG`,
  `01_TextEffect`, `02_PilotData` (license panel), `02_OnlineArena_CommonBG`, the DetailStatus panels,
  `05_001_Title_Logo`. **Reloaded as a preview only**: the probe registers live panel windows (open/close B hooks)
  and runs the panel's own close + open (`0x140c8aff0` / `0x140c8acf0`; the game swaps a panel's movie the same way
  in `0x140c8a6b0`, and reaches it through `window+8` via `0x140c8a8c0`). The new art shows, but a panel's owner is
  a C++ view object that took handles into the movie when it was built (`02_PilotData`: view class vtable
  `0x1439e6030`, built in `0x14109a93e`, keeps `"PilotData"` at `+0xe0`, `"ArenaRankIcon"` at `+0x2e8`...), so text,
  data and input stay with the old, invisible movie (tested: magenta license panel with placeholder name/ranks, edits
  in the license menu no longer show; the rest of the garage works). The real rebuild happens when the owner is
  rebuilt; owners vary per panel (PilotData's is built behind an obfuscated jump), so no generic fix. The log says
  so on every panel reload.
- **What survives a mission load** (one AC TEST round trip, 2026-11-13; a real sortie not checked): closed only on
  return to the title screen: `01_VirtualBG`, `01_060_Caption`, `01_SystemToast`, `01_900_Black`,
  `01_FadeScreen_Low/Middle`, `01_LetterBox` (main-menu windows: live reload works) and the panels
  `01_MenuCommonBG`, `02_OnlineArena_CommonBG` (preview only; rebuilt only via the title screen). Everything else
  seen was rebuilt by the AC TEST transition.

Final checks with Arxan on (no debugging features): the reload with the close-check fix, Assembly, AC TEST and back
all normal (run B); the three-menu reload above also ran with Arxan on.

## Menu table, files and the leftover-repository crash (2026-11-13)

- The menu table (`0x144e28cf0`) is filled by the static initializer `0x1401d44e0`: name + layer per entry
  (`01_900_Black` 0x1a, `01_FadeScreen_Middle` 8, `01_FadeScreen_Low` 2, `01_SystemToast` 0x1e, `01_LetterBox` 0x1f,
  `01_SystemAnnounce` 0x17...). Name -> file through `0x14160a5b0`: `menu:/%s.gfx` (or `menu:/Win/%s.gfx`).
  `01_070_CommandList` is listed twice; `02_gaitemstatus_general_minimum` has no file; `win\02_textinput.gfx` is the
  Win variant. A movie can still load other things by name at runtime (TPF images, fonts).
- **Crash fixed:** boot requests `01_070_CommandList` (listed twice) and `01_900_Black` (second request from
  somewhere else, not found) twice and opens them once, so one entry stays in the file repository with the
  already-used file object (the hud-color notes' "boot files left in the list"). A reload of `01_900_Black` got that
  entry instead of a disk miss and the game died silently in the load (no dump). The release step now drops leftover
  entries with the game's own release (`0x1416040c0`) until the repository lookup misses
  (`purge_repository`). After the fix all seven persistent windows reload live from disk.

## Debug registers and Arxan (2026-11-13)

- The probe's F5 arms a 1-byte write watchpoint (`probe-dll\src\watch.rs`) on `CSMenuMan+0x10` and installs the
  exception handler that logs the writers. It used to fail on every thread: windows-sys 0.59 declares the x86-64
  `CONTEXT` without `align(16)`, so the stack copy was misaligned and Get/SetThreadContext failed. Fixed with an
  aligned wrapper; now arms on all threads (142/142, checked from outside too). (The Steam overlay hooks
  `ResumeThread` in the game; harmless.)
- With Arxan active: ntdll's context functions are unpatched and debug registers set from outside stay set, but the
  game crashed ~20 s later (main thread jumped to `0x180fe7430`; dump in `%LOCALAPPDATA%\CrashDumps`): Arxan
  detects debug registers and sabotages, it doesn't block them.
- With me3 `--disable-arxan`: armed on all threads for 5+ minutes through a sortie, no crash. Runtime debugging
  needs that option; results must still be re-checked with Arxan on.

## Final checks (2026-11-13)

- Patching the disk fallback at load is safe: all 109 boot files still come from the repository, the one file-less
  entry `02_gaitemstatus_general_minimum` takes the disk path and fails cleanly as before; garage, Assembly and AC TEST
  normal (Arxan on). `05_termofservice` wasn't requested that boot.
- AC TEST goes through the real sortie path (`02_NowLoading_Sortie`, HUD built the same way), so it stands in for a
  sortie; a story mission would only add cutscene / comms windows (not checked).
- The second close check `0x140d626f0` is `xor eax,eax; ret`: a "never close" default installed by a generic
  window-job constructor (around `0x140d619f0`); all seven persistent windows carry it.
- The finished tool, tested with Arxan on: F7 with a live menu, a panel and a menu not on screen; the F8 watcher; the
  HUD reloaded live during AC TEST (still fully working, back to the garage normally); F9 and the toasts; the test kit
  from a fresh unzip.
- `01_SystemToast` (how toasts are driven) was worked out in its own project, Toast-Wrapper (`system-toast`).

## Elden Ring support (2026-11-14)

One DLL for both games (game detected at load by exe name, addresses in a per-game table, shared logic): see
[er-port.md](er-port.md) for what was matched, with evidence, and how ER reads edited files.

## One file opener hook for both games (2026-11-14)

The ER port needed a hook on the file opener (ER has no disk read to re-enable). AC6 then moved to the same hook,
which replaced the disk-fallback patch and the leftover purge described above.

- AC6 has the same Scaleform default file opener as ER: `0x141a7bfb0`, the only match for the first 40 bytes of
  ER's `0x141159f50` (they differ in relative call offsets only). AC6's own disk read can't be called: it's inline
  code in the opener `0x1415f40b0` (after the `jnz` at `0x1415f427d`), not a function.
- AC6's opener gets the same two kinds of path as ER's. Boot with the full test package: 112 opens, 107 with me3
  tokens (all read by the default opener, none failed) and 5 archive paths (three fonts and
  `data0:/menu/02_gaitemstatus_general_minimum.gfx`, twice), left to the game.
- The mod's files skip the repository, so on its own the hook would leave their entries unopened and unreleased
  for the whole session (tested that way first: boot, garage, Assembly and AC TEST were normal, Arxan on). The hook
  now does the game's bookkeeping itself. The stock opener (instructions `0x1415f41f4`..`0x1415f4254`): lookup
  `0x14153d3d0`(repository `[0x144e245b0]`, a wide copy of the path it was given), take the file from the entry
  (`0x14153dc50`), then, if `0x14171da90` returns true, release `0x1416040c0`(system = opener `+0x18`, entry). The
  same call gates the disk read on a miss (the `jnz`). `0x14171da90` is a stub that returns 1 (thunk to
  `0x14120759a`: `return 1`), called from eleven places in the scaleform system (movie lookup, opener, the boot
  request code): a build-time setting compiled to a constant, probably "use the file repository" (a development
  build would return false: entries kept, misses read from disk; not confirmed). So in this exe the release always
  happens, and the hook releasing unconditionally matches the game.
- The hook's lookup with the me3 token finds the entries. Boot with the full package: 107 disk reads, 104 entries
  gone after one release, 2 still there (`01_070_CommandList` and `01_900_Black`, requested twice: their second
  copies, as in the stock game). One later open (157 s) had no entry left, as expected.
- The leftover crash can't happen any more: a reloaded file never comes from the repository. Reloading
  `01_900_Black` read it from disk, found its second copy and released it (gone afterwards). Also tested:
  `01_GarageTopMenu` live; `02_058_DetailStatus_Player_2P` as a panel preview; `01_000_FE` reloaded while off screen
  and read from disk on entering AC TEST.
- Removed: the `jnz` patch (the tool no longer changes game code, it only hooks), the path builder call and the
  purge loop, and the per-game choice between two mechanisms. Kept, AC6 only: the lookup and release, now called
  once per disk read.
