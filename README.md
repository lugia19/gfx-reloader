# ui-hot-reload

Edit a menu `.gfx` of your Armored Core VI mod, press a key, and see the change in the running game. No restart.

For game version 1.09.1, with [me3](https://github.com/garyttierney/me3).

## Setup

1. Add `ui_hot_reload.dll` to your me3 profile as a native:
   ```toml
   [[natives]]
   path = 'path\to\ui_hot_reload.dll'
   ```
2. Put `ui_hot_reload.ini` next to the DLL and point it at your me3 package (the folder that contains `menu\`):
   ```ini
   package = C:\path\to\your\mod
   ```
3. Start the game through me3 as usual.

## Use

- Edit and save any `menu\*.gfx` file in your package, then press **F7** in game.
- Or press **F8** once to turn on the file watcher: every save is reloaded automatically. F8 again turns it off.
- **F9** lists every menu movie on screen right now, marking the panels that can only be previewed (see below).
- `Game\debug-logs\ui_hot_reload.log` says what happened to each menu. If
  [AC6-Toast-Wrapper](https://github.com/lugia19/AC6-Toast-Wrapper) (`ac6_toast.dll`) is in your profile too, the
  results also show in game as toasts ("Reloaded 3 menus (1 preview only, broken until the next scene change)").

## What you'll see

- **Menus on screen** (garage menus, the HUD in a mission, fades, captions): reloaded instantly.
- **Menus not on screen**: they use the new file the next time they open.
- **Panels** (the license panel, Assembly's stat panels, some backgrounds): you see the new shapes and colors right
  away, but their text, numbers and buttons stop updating until the game rebuilds them, which happens on the next
  scene change (entering or leaving a mission or AC TEST). Two background panels, `01_MenuCommonBG` and
  `02_OnlineArena_CommonBG`, are only rebuilt when you return to the title screen. The log tells you when a panel
  was only previewed.

## Limits

- **New files need a restart.** me3 only serves files that existed when the game started. To work on a menu your
  mod doesn't override yet, put a copy of the stock file in your package first, then start the game.
- Only `.gfx` menu movies. Textures (`.tpf`) and other files aren't reloaded.

How it works: [docs/how-it-works.md](docs/how-it-works.md). Building and testing: [docs/development.md](docs/development.md).
