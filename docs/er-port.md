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
  closes. Same in both games; AC6 needs a re-test after the change.
- **Map node keys** are read per game: AC6 pointer + length; ER `std::wstring` (inline under 8 characters).
- **Per-game table** like Toast-Wrapper's: exe name, offsets from the exe base, struct offsets, optional AC6-only
  parts (disk-fallback patch, repository purge with its path builder) set to "none" for ER if the OpenFile finding
  holds.

## Probable (one signal so far)

- **No in-memory repository in ER's OpenFile.** It checks the path with `[0x14458fb68]` (call `0x141ee7540` at
  `0x140d6ea36`): not found -> Scaleform's default file opener `0x141159f50` (plain disk read); found -> the game's
  file system `[0x143d83888]` -> `0x140cea560`. No AC6-style "repository only" gate, and `Scaleform.SwfCache` is
  only a memory container's name (set up in the lookup). If that holds, ER needs no disk-fallback patch and no
  leftover purge, just the cache release and the window reopen. To confirm in game (reload without the patch).
- The scaleform system class: `CS::CSScaleformImp` (`FD4Singleton<CS::CSScaleform>`, vtable `0x142bbe6c8`,
  constructor `0x140d6d990`); the holder `[0x143d8a1d8]` may be that singleton.

## To do

- Read-only memory check in the running game: `[[holder]+8]` -> cache map holds menu names (wstring keys); active
  list shows the open menus; menu table names read back.
- The per-game table in the DLL (see above).
- In-game test: reload a menu on screen without any OpenFile patch (confirms the OpenFile finding), one not on
  screen, the HUD while playing, a panel (`CS::MenuResource`); re-test AC6 after the panel registry change.
