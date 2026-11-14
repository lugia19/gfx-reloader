# Elden Ring port: working notes

Elden Ring Steam build 25827938 (the installed exe is byte-identical to `AC6_tools\Research\exe_re_er\eldenring.exe`,
analyzed with RTTI in `ghidra\ER.gpr`). ER addresses below are for the unrelocated exe (base 0x140000000); the DLL
must use offsets from the exe base (ASLR), like Toast-Wrapper's per-game table.

A match counts as confirmed with two independent signals (name / vtable layout / field usage / bytes).

## Confirmed

| What | AC6 1.09.1 | Elden Ring | Evidence |
|---|---|---|---|
| Self-owned window class | vtable `0x14390e498` | `CS::MenuWindowJob`, vtable `0x142aae958` | RTTI name; same vtable shape (step at `+0x10`, six identical slots `+0x20`..`+0x48`, the close-check function vtable copied from `+0x58`); same fields (constructor `0x1407ad7b0`) |
| Window step (vtable `+0x10`) | `0x140d61af0` | `0x1407ae4f0` | vtable slot; opens when no controller (`+0x130`), factory `+0xa8` set and movie `+0x10` empty |
| Window close | `0x14641c16c` | `0x1407aed70` | same sequence: dispose controller `+0x130`, reset movie `+0x10`, close player `+0x68`, destroy close check `+0x128`; called first by the destructor `0x1407ada50` |
| Window fields | name `+0x60`, player `+0x68`, factory `+0xa8`, close check `+0xf0`/`+0x128`, controller `+0x130` | same | ER constructor: entry copy `+0x58..+0x67`, three `std::function`s (storage `+0x70`/`+0xb0`/`+0xf0`, impl pointers `+0xa8`/`+0xe8`/`+0x128`), refcounted controller `+0x130` |
| Close check `std::function` (function-pointer impl) | vtable `0x14390e4f0` | `std::_Func_impl<MenuWindowJob::MenuCtrlType (*)()>`, vtable `0x142aae9b0` | RTTI name; same slots (copy/move/call/.../delete) |
| Menu manager | `[0x144e15530]`, ui_states `+0x30` | `CSMenuManImp` `[0x143d72840]`, ui_states `+0x90` | the window close clears `ui_states[id]` (ER `+0x90`, matches fromsoftware-rs' ER `CSMenuManImp`) |
| Scaleform holder | `[0x144e28818]`, system at `+8` | `[0x143d8a1d8]`, system at `+8` | open facade `0x140d6dd80` (byte match with AC6 `0x1415f3030`) reads `holder+8` and calls open-menu; the player close `0x140d6de90` reads `holder+8`; the window close passes this holder |
| Open menu (lookup + player) | `0x1416069a0` | `0x140d7bad0` | byte match (64 bytes); called by the open facade; calls the lookup, then the player create `0x140d7e060` |
| Movie lookup | `0x141609040` | `0x140d7dad0` | called by open-menu the same way; same flow (find in cache, load through the loader, cache if resident) |
| Movie cache map head | `system+0x9c0` | `system+0x9b0` | lookup: map `system+0x9a8`, head node pointer `+0x9b0` |
| Map nodes (cache and players) | key pointer `+0x28`, length `+0x30`; value `+0x248` | key `std::wstring` at `+0x228` (inline when capacity `+0x240` is below 8, else a pointer), length `+0x238`; value `+0x250` | lookup and player close compare keys this way; value read at `+0x250` (`plVar7[0x4a]`) |
| Player map | `system+0x9d0`, head `+0x9d8` | `system+0x9c0`, head `+0x9c8` | player close |
| Movie loader | `system+0x968` | `system+0x970` | lookup |
| Active list / count / max | `+0x9e8` / `+0xdf0` / 64 | `+0x9d8` / `+0xbe0` / 32 | active-list add `0x140d7d380` (byte match with AC6 `0x141608170`), 16-byte `{player, open count}` entries |
| Player's menu entry copy | `player+0x20`, name `+0x28` | same | player close reads the name at `+0x28` and the resident byte at `+0x24` |
| Menu path builder | `0x14160a5b0`, path pointer at `+8` | `0x140d7eca0`, result is a `std::wstring` at `+8` (inline when capacity `+0x20` is below 8) | byte match (48 bytes); the lookup calls it the same way |
| File opener vtable, OpenFile (slot `+0x08`) | `0x143b06ea8`, `0x1415f40b0` | `CS::CSScaleformFileOpener` `0x142bc0748`, `0x140d6e970` | RTTI name; same slot (hud-color's notes called ER's slot 1 "container setup": it's OpenFile, the decompile stops early) |
| Player close (system, player) | `0x141606b10`, via the facade `0x1415f3150` | `0x140d7bc40`, via the facade `0x140d6de90` | both facades are the same 3-instruction stub (`mov rcx,[rcx+8]; test; jnz <close>`); same body (player map lookup, resident == 2 deferred list, else close) |
| Embedded panel class | vtable `0x1438e5238` | `CS::MenuResource`, vtable `0x142a99498`, 0x68 bytes | RTTI name; destructor `0x14074a080` closes the player at `+0x60`; same fields: entry `+0x48` (name `+0x50`), player `+0x60`, movie `+8` |
| Panel open | `0x140c8acf0` | `0x14074aa40` | same code (entry at `+0x48`, open facade, player to `+0x60`, bind the movie at `+8` with `0x14073f250`, byte match of AC6's `0x140c8a410`); called by the panel getter `0x14074a640` (byte match of AC6's `0x140c8a8c0`) when there's no player, like AC6 |
| Panel movie swap | `0x140c8a6b0` (calls the panel close `0x140c8aff0`) | `0x14074a410` (closes the player inline) | same job: close the player, store a new entry |
| Other window kind (player `+0x58`, controller `+0xa0`) | open `0x140c8aa60` | `CS::MenuWindowProxy`, vtable `0x142a99488`, 0xf8 bytes; close `0x14074ab90` | not used by the tool |
| Menu table | `0x144e28cf0`, 109 entries of 16 bytes (layer, resident `+4`, name `+8`), initializer `0x1401d44e0` | `0x143b443f0`, 106 entries, same layout, initializer `0x1400aeef0` | same initializer pattern; `01_000_FE` is entry 17 |

Prologues (first 8 bytes) for the ER functions the tool hooks or calls: step `4c 89 44 24 18 55 53 56` (same as AC6),
window close `40 55 56 57 48 8d 6c 24`, panel open `40 53 48 81 ec a0 00 00`, player close `40 55 56 57 48 81 ec 80`.
The ER exe has ASLR (DYNAMIC_BASE), so the DLL needs offsets from the exe base: subtract 0x140000000 from the
addresses here.

## Design changes for the port

- **Panel registry:** ER has no separate panel close (the destructor and the movie swap close the player inline).
  So instead of hooking the panel close, the tool hooks the **player close** (AC6 `0x141606b10`, ER `0x140d7bc40`,
  both `(system, player)`), stores `(panel, player, name)` when a panel opens, and drops an entry when its player
  closes. Same in both games.
- **Closing a panel for a reload** stays per game. AC6's panel open (tail at `0x140c8adc9`) and panel close
  `0x140c8aff0` keep a per-name count in a map at `CSMenuMan+0x98` (the close decrements it and erases the entry at
  zero, then closes the player and clears `+0x60`), so the tool calls AC6's close. ER's panel open, movie swap and
  destructor never touch the menu manager, and its closes are just "close the player, clear `+0x60`", which the tool
  does by hand there.
- **Map node keys** are read per game: AC6 pointer + length; ER `std::wstring` (inline under 8 characters).
- **Per-game table** like Toast-Wrapper's: exe name, offsets from the exe base, struct offsets, the file opener
  and default opener to hook and call (below; AC6 uses the same hook since 2026-11-14), AC6's store lookup and
  release, and the panel close where the game has one.

## Reading edited files in ER (confirmed in game)

ER has the same in-memory store and the same switch as AC6, with one difference in how the store behaves.

- OpenFile (`0x140d6e970`) reads the debug property **`Scaleform.SwfCache`** (default true): it builds that name and
  calls the property getter `0x141ee7540` on the debug properties `[0x14458fb68]` (at `0x140d6ea36`). False ->
  Scaleform's default file opener `0x141159f50` (reads the file itself). True -> the store `CS::GfxRepositoryImp`
  (`[0x143d83888]`): lookup `0x140cea560` (map at `repo+0x78`), then `0x140ceb070` builds a memory file from
  `entry+0x78`/`+0x80`; a miss there returns 0. The developers' property list describes it: set to false, the swf
  is reloaded each time the menu is launched. So ER does have a disk path; it is off by default.
- **Correction (2026-11-14):** these notes first called the property read a "path check" and the string "a memory
  container's name", and concluded that ER has no disk read. That was a misreading of unnamed code, pointed out in
  the modding Discord with a named decompile. The addresses and the tests below are unaffected; the tool's hook
  sends a mod's files to the same default opener the property's false branch uses. Not tried here: forcing the
  property to false (it would reload a menu when it is reopened, not while it is on screen).
- The store **keeps its entries** (AC6's are dropped as movies open), so a released movie was rebuilt from the boot
  copy: first test, a reloaded title screen stayed stock although me3 served the new file.
- me3 replaces the path of each file it overrides with a token (`\\me3??54`); files from the archives keep theirs
  (`data0:/menu/01_900_black.gfx`). Seen in the OpenFile hook's log: with two files in the package, 2 of 113 boot
  opens had tokens. The store is keyed by the string OpenFile gets, so a menu name alone doesn't find its entry.
- **Fix:** hook OpenFile; an archive path (a root name of 2+ characters, then `:/`) is left alone, anything else
  goes to the default opener (falls back to the original if that returns 0). So exactly the package's files are
  read from disk, every time, boot included (110 files with a full package, no failures). The default opener takes
  me3's tokens fine. The check is "not an archive path" rather than "starts with `\\me3`" so a changed token
  format or another loader doesn't silently bring the stale copies back; only `data0` has been seen as a root. If
  an exe patched to read unpacked files passes other paths for stock files, the cost is extra disk reads.
- Tested: title screen reloaded live (window + logo panel), title reloaded while in game then shown on quit to
  menu, HUD (`01_000_FE`) reloaded live while playing.
- Quirk: a reloaded HUD starts its rune counter at 0 and the game sets the real total, so the "runes gained"
  animation plays for the whole total. Display only (a second reload shows the same total).

## Probable (one signal so far)

- The scaleform system class: `CS::CSScaleformImp` (`FD4Singleton<CS::CSScaleform>`, vtable `0x142bbe6c8`,
  constructor `0x140d6d990`); the holder `[0x143d8a1d8]` may be that singleton.

## Status

- Done and merged into `main`: read-only memory check (menu table names, cache map with 110 wstring keys, active
  list); the per-game table; the ER in-game tests above; AC6 re-tested after the panel registry change (garage menu
  live, Assembly stat panels as previews and rebuilt after AC TEST, HUD live in AC TEST).
- 2026-11-14: AC6 moved to the same OpenFile hook (research-notes.md, "One file opener hook for both games"), so
  the two games no longer differ in how edited files are read.
