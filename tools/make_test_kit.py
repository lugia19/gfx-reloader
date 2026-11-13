"""Builds a self-contained test kit for someone else: the tool, the toast DLL, an me3 profile with its own save, a
package with the stock HUD, and colored HUD variants to swap in. Output: dist/gfx-reloader-test/ and its zip.

    python -I tools/make_test_kit.py

Needs the built DLLs: dll/target/release/ui_hot_reload.dll and tests/toast-build/release/ac6_toast.dll
(docs/development.md says how to build the toast one).
"""
import os, shutil, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import tint  # noqa: E402

KIT = os.path.join(ROOT, 'dist', 'gfx-reloader-test')
HUD = '01_000_fe.gfx'
# name -> (RGB multiply, RGB add)
COLORS = {
    'green': ('00FF00', None),
    'red': ('FF0000', None),
    'cyan': ('00FFFF', None),
    'magenta': ('FF00FF', None),
    'yellow': ('FFFF00', None),
    'orange': ('FF8000', None),
}

PROFILE = """profileVersion = "v1"

# gfx-reloader test kit: the stock HUD in a package, the reload tool, and the toast DLL for in-game messages.
# Uses its own save (GfxReloaderTest.sl2, copied from your real save the first time), so your save isn't touched.

savefile = "GfxReloaderTest.sl2"

[[supports]]
game = "armoredcore6"

[[packages]]
id = "gfx-reloader-test"
path = 'mod'

[[natives]]
path = 'ui_hot_reload.dll'

[[natives]]
path = 'ac6_toast.dll'
"""

LAUNCH = """@echo off
cd /d "%~dp0"
where me3 >nul 2>nul
if errorlevel 1 (
    echo me3 isn't on your PATH. Install it from https://github.com/garyttierney/me3, or run it by its full path:
    echo     "C:\\path\\to\\me3\\bin\\me3.exe" launch -p "%~dp0gfx-reloader-test.me3"
    pause
    exit /b 1
)
me3 launch -p gfx-reloader-test.me3
"""

README = """gfx-reloader test kit
=====================

Reloads edited menu movies (.gfx) in the running game. This kit has the stock HUD in a mod package, so you can swap
in recolored HUDs and watch them change live. Armored Core VI 1.09.1, needs me3 (https://github.com/garyttierney/me3).

1. Unzip anywhere and run launch.bat (or: me3 launch -p gfx-reloader-test.me3 from this folder; with a portable
   me3 that isn't on your PATH, use the full path to its bin\\me3.exe).
   It uses its own save (GfxReloaderTest.sl2, a copy of your real save), so your progress isn't touched.
2. Load in and start an AC TEST from the garage (Assembly, RB), so the HUD is on screen.
3. Copy one of the files in colored\\ over mod\\menu\\01_000_fe.gfx (it must keep the name 01_000_fe.gfx).
4. Press F7. The HUD should change color right away, and a toast in the top left says "Reloaded 1 menu".
   F8 turns on a file watcher instead (every save reloads by itself, about 1-2 s later); F8 again turns it off.
   F9 lists every menu on screen in toasts.
5. To go back to stock, copy colored\\01_000_fe_stock.gfx over it the same way.

Also worth trying: reload while in the garage (the HUD isn't on screen; the next AC TEST should show the new color),
and keep playing the AC TEST after a reload (boost, lock on, fire: the HUD should keep working).

If something goes wrong, send the log: <game folder>\\Game\\debug-logs\\ui_hot_reload.log
"""


def make_hud(color, add, out):
    """A tinted copy of the stock HUD (tint.py's color transform on every placed object)."""
    data = open(tint.stock_path('01_000_FE'), 'rb').read()
    nbits = data[8] >> 3
    tags_start = 8 + (5 + 4 * nbits + 7) // 8 + 4
    rgb = [int(color[i:i + 2], 16) * 256 // 255 for i in (0, 2, 4)]
    add_rgb = [int(add[i:i + 2], 16) for i in (0, 2, 4)] if add else None
    tags, _ = tint.rebuild(data, tags_start, len(data), rgb, False, add_rgb)
    result = bytearray(data[:tags_start] + tags)
    struct.pack_into('<I', result, 4, len(result))
    open(out, 'wb').write(result)


def main():
    dll = os.path.join(ROOT, 'dll', 'target', 'release', 'ui_hot_reload.dll')
    toast = os.path.join(ROOT, 'tests', 'toast-build', 'release', 'ac6_toast.dll')
    for path in (dll, toast):
        if not os.path.isfile(path):
            sys.exit(f'missing {path}: build it first (docs/development.md)')
    if os.path.isdir(KIT):
        shutil.rmtree(KIT)
    os.makedirs(os.path.join(KIT, 'mod', 'menu'))
    os.makedirs(os.path.join(KIT, 'colored'))
    shutil.copyfile(dll, os.path.join(KIT, 'ui_hot_reload.dll'))
    shutil.copyfile(toast, os.path.join(KIT, 'ac6_toast.dll'))
    stock = tint.stock_path('01_000_FE')
    shutil.copyfile(stock, os.path.join(KIT, 'mod', 'menu', HUD))
    shutil.copyfile(stock, os.path.join(KIT, 'colored', '01_000_fe_stock.gfx'))
    for name, (color, add) in COLORS.items():
        make_hud(color, add, os.path.join(KIT, 'colored', f'01_000_fe_{name}.gfx'))
    for name, text in (('gfx-reloader-test.me3', PROFILE), ('launch.bat', LAUNCH), ('README.txt', README)):
        newline = '\r\n' if name.endswith(('.bat', '.txt')) else '\n'
        with open(os.path.join(KIT, name), 'w', encoding='utf-8', newline=newline) as f:
            f.write(text)
    archive = shutil.make_archive(KIT, 'zip', os.path.dirname(KIT), os.path.basename(KIT))
    print(f'{KIT}\n{archive} ({os.path.getsize(archive) // 1024} KB)')


if __name__ == '__main__':
    main()
