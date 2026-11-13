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

## Probable (one signal so far)

- Scaleform holder: ER `[0x143d8a1d8]` (the window close passes it to the player close `0x140d6de90`, like AC6's
  facade `0x1415f3150(holder, player)` which reads the system at `holder+8`). To check: `0x140d6de90` reads `+8`.
- The scaleform system class: `CS::CSScaleformImp` (`FD4Singleton<CS::CSScaleform>`, vtable `0x142bbe6c8`,
  constructor `0x140d6d990`). The holder above may be the singleton itself.

## To do

- Scaleform system: movie cache map head, node key/value offsets, active list + count, player name offset.
- File repository (`CS::CSFileRepository`), its lookup and release, the menu path builder (`menu:/%s.gfx` at
  `0x142bc25e8`, used at `0x140d7ee5f`).
- File opener OpenFile and its disk-fallback `jnz` (`Scaleform.SwfCache` at `0x142bbe7d0`, used in `0x140d7dad0` and
  `0x140d6e970`).
- Menu table (refs to `L"01_000_FE"`: `0x1400aeef0`, `0x1408d40c0`).
- Panel windows: open/close and their offsets (AC6 `0x140c8acf0` / `0x140c8aff0`; candidate ER class `CS::MenuWindow`).
- Prologue bytes for every hooked function; then the per-game table in the DLL.
