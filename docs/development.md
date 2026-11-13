# Development

- `dll\`: the tool. `cargo +stable build --release` -> `dll\target\release\ui_hot_reload.dll`. Modules: `game.rs`
  (addresses, layouts), `reload.rs` (hooks and the game-thread work), `files.rs` (package snapshot, watcher),
  `lib.rs` (setup, config, keys), `toast_wrapper_client.rs` (copied from Toast-Wrapper's `client\`; does nothing
  without `toast_wrapper.dll`). F9 reads the scaleform system's active player list (what gets drawn).
- The test profile also loads `tests\toast-build\release\toast_wrapper.dll`, a plain build of Toast-Wrapper
  (without its F7 test hotkey, which would clash): in `system-toast\dll`,
  `cargo +stable build --release --target-dir <this folder>\tests\toast-build`.
- `probe-dll\`: the research probe the tool came from. Logs every menu lookup, load, file open, menu open/close with
  call stacks, Scaleform's own log messages; F5 hardware write watchpoint on `CSMenuMan+0x10` (needs
  `--disable-arxan`), F9 state dump, F10 break test (closes every player). Builds `ui_hot_reload_probe.dll`, logs to
  `Game\debug-logs\ui_hot_reload_probe.log`. `probe-dll\tools\find_window.py <menu>` finds a menu's window object in
  the running game (scans for the player pointer from the probe's log) and dumps it.
- Test setup: `tests\mod` holds every stock menu movie (so any can be edited mid-session; not in git, fill it with
  `python -I tools\make_test_package.py`, which also resets it to stock); profiles
  `AC6_tools\me3\ui-hot-reload.me3` (tool) and `ui-hot-reload-probe.me3` (probe), both on the alt save
  `UiHotReload.sl2`; `launch-ui-hot-reload.bat` / `launch-ui-hot-reload-probe.bat`. Both DLLs read `ui_hot_reload.ini`
  from this folder.
- `tools\tint.py <menu> [RRGGBB] [--add RRGGBB]`: writes a tinted copy of a stock menu into the test package (color
  transform on every placed object; `--add` also colors black art); `--restore` puts the stock file back.
- Screenshots of the game window: `AC6_tools\scripts\screenshot.ps1`.
- Always check changes with Arxan active (plain launch), even when debugging with `--disable-arxan`.
