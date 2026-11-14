# How ui-hot-reload works

Described for Armored Core VI 1.09.1; Elden Ring's differences are at the end. The full investigation, with every
test and dead end, is in [research-notes.md](research-notes.md). Class names come from Elden Ring's RTTI (same
engine; AC6's exe has none), each matched to AC6 by layout and code: see [er-port.md](er-port.md).

## How the game loads menu movies

- Every menu is an entry in the **menu table** (`0x144e28cf0`, filled at startup by `0x1401d44e0`): a name, a layer
  and a "resident" flag. Name to file: `menu:/<name>.gfx` (lowercased on disk: `menu\01_garagetopmenu.gfx`).
- At boot the game **requests all menu files** into an in-memory file repository (`[0x144e245b0]`). The game's
  Scaleform file opener (`0x1415f40b0`) only reads from there, and each file is dropped from it once its movie is
  built. After boot, the files are gone from memory, and the opener's own disk read is switched off (a `jnz` at
  `0x1415f427d`), so a menu file can't be opened a second time.
- me3 replaces the path of every file it overrides with a token (`\\me3??54`); stock files keep their archive path
  (`data0:/menu/01_900_black.gfx`).
- Built movies are **cached** by the scaleform system (`[[0x144e28818]+8]`, cache map at `+0x9b8`; probably ER's
  `CS::CSScaleformImp`, not confirmed yet). Opening a menu looks the movie up there first.
- A movie on screen is a **player** made from the cached movie. Players belong to **windows**, and a window is
  driven by a C++ controller that grabs handles into the movie (text fields, clips) and fills it in.

## What the tool does

1. **Read mod files from disk.** The tool hooks the file opener: every path that isn't an archive path (a root
   name, then `:/`) is a mod's file and goes to Scaleform's default opener (`0x141a7bfb0`), which reads the file
   itself, through me3, so it's the package's current file. Archive paths are left to the game. So exactly the
   package's files are read from disk, on every open, boot included. After such a read the hook releases the file's
   repository entry with the game's own lookup and release (`0x14153d3d0`, `0x1416040c0`), as the game's opener
   does after opening from the repository, so the repository empties exactly as in the stock game.
2. **Release.** For each changed menu it releases the cached movie (properly, so Scaleform frees it and doesn't hand
   it back). The next open of that menu loads the file again, through the hook.
3. **Reopen what's on screen.** Two kinds of windows:
   - **Self-owned windows** (`CS::MenuWindowJob`: garage menus, HUD, fades, captions...): each builds its own
     controller when it opens, through a factory `std::function` its creator passed in (`+0x70`/`+0xa8`). The tool
     hooks their per-frame step (`0x140d61af0`): a window with a pending reload is closed with its own close
     (`0x14641c16c`) and the step reopens it in the same frame, which builds a new player and a new controller from
     the reloaded movie. Fully working.
     The window's creator also gives it a control check, a `std::function<MenuWindowJob::MenuCtrlType()>` at
     `+0xf0`/`+0x128` (for the garage menus it means "a cutscene is taking over", read from `CSMenuMan+0x10`, which
     `CSRemo` sets). The close would destroy it and the reopen never recreates it, so the tool keeps it through the
     reload.
   - **Embedded panels** (the license panel, Assembly's stat panels, some backgrounds): windows owned by some other
     C++ object that grabbed handles into the movie when it was built. The tool closes and reopens the panel, which
     shows the new art, but the owner keeps writing to the old movie: text, data and input stop updating. Only
     rebuilding the owner fixes that, and owners differ for every panel, so the tool leaves it to the game (next
     scene change; the two background panels `01_MenuCommonBG` and `02_OnlineArena_CommonBG` only on return to the
     title screen).
4. Menus that aren't on screen need nothing more: their next open reads the new file.

## Elden Ring

Same engine, same four steps, same hook (addresses in [er-port.md](er-port.md)). What differs is the store the hook
bypasses: ER's (`CS::GfxRepositoryImp`) **keeps** the boot copy of every menu file, and its file opener
(`CS::CSScaleformFileOpener`, `0x140d6e970`) has no disk read for menu files at all, so without the hook a released
movie is rebuilt from the boot copy. The default opener is `0x141159f50`. ER's store is left alone (it keeps its
entries in the stock game too).

- AC6's panel open and close also keep a per-name count in the menu manager; ER's don't, so there the tool closes a
  panel by hand (close the player, clear the panel's field).
- Quirk: reloading the HUD replays the "runes gained" animation for your whole total (display only).

## Limits found

- me3 only serves package files that existed when the game started.
- Runtime debugging (hardware breakpoints) needs me3's `--disable-arxan`: Arxan crashes the game ~20 s after it
  sees debug registers. The tool itself uses none and runs with Arxan active.
