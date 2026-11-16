"""Builds a test kit (tool, toast DLL, me3 profile with its own save, stock + tinted HUDs) in dist/, plus a zip.

    python -I tools/make_test_kit.py                        # Armored Core VI -> dist/gfx-reloader-test
    python -I tools/make_test_kit.py --game er --no-readme  # Elden Ring       -> dist/gfx-reloader-test-er

Needs dll/target/release/gfx_reloader.dll and tests/toast-build/release/toast_wrapper.dll (docs/development.md).
"""
import argparse, os, shutil, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import tint  # noqa: E402

HUD = '01_000_fe.gfx'
# name -> (RGB multiply, RGB add)
COLORS = {
    'green': ('00FF00', None),
    'red': ('FF0000', None),
    'blue': ('0000FF', None),
    'cyan': ('00FFFF', None),
    'magenta': ('FF00FF', None),
    'yellow': ('FFFF00', None),
    'orange': ('FF8000', None),
}
# game -> what differs in its kit
KITS = {
    'ac6': {
        'name': 'gfx-reloader-test',
        'me3_game': 'armoredcore6',
        'save': 'GfxReloaderTest.sl2',
        'title': 'Armored Core VI 1.09.1',
        'show_hud': 'Load in and start an AC TEST from the garage (Assembly, RB), so the HUD is on screen.',
    },
    'er': {
        'name': 'gfx-reloader-test-er',
        'me3_game': 'eldenring',
        'save': 'GfxReloaderTestER.sl2',
        'title': 'Elden Ring',
        'show_hud': 'Load into the game, so the HUD is on screen.',
    },
}

PROFILE = """profileVersion = "v1"

# gfx-reloader test kit: the stock HUD in a package, the reload tool, and Toast-Wrapper for in-game messages.
# Uses its own save ({save}, copied from your real save the first time), so your save isn't touched.

savefile = "{save}"

[[supports]]
game = "{me3_game}"

[[packages]]
id = "{name}"
path = 'mod'

[[natives]]
path = 'gfx_reloader.dll'

[[natives]]
path = 'toast_wrapper.dll'
"""

LAUNCH = """@echo off
cd /d "%~dp0"
where me3 >nul 2>nul
if errorlevel 1 (
    echo me3 isn't on your PATH. Install it from https://github.com/garyttierney/me3, or run it by its full path:
    echo     "C:\\path\\to\\me3\\bin\\me3.exe" launch -p "%~dp0{name}.me3"
    pause
    exit /b 1
)
me3 launch -p {name}.me3
"""

README = """gfx-reloader test kit
=====================

Reloads edited menu movies (.gfx) in the running game. This kit has the stock HUD in a mod package, so you can swap
in recolored HUDs and watch them change live. {title}, needs me3 (https://github.com/garyttierney/me3).

1. Unzip anywhere and run launch.bat (or: me3 launch -p {name}.me3 from this folder; with a portable
   me3 that isn't on your PATH, use the full path to its bin\\me3.exe).
   It uses its own save ({save}, a copy of your real save), so your progress isn't touched.
2. {show_hud}
3. Copy one of the files in colored\\ over mod\\menu\\01_000_fe.gfx (it must keep the name 01_000_fe.gfx).
4. Press F7. The HUD should change color right away, and the game's announce banner says "Reloaded 1 menu".
   F8 turns on a file watcher instead (every save reloads by itself, about 1-2 s later); F8 again turns it off.
   F9 lists every menu on screen in the banner.
5. To go back to stock, copy colored\\01_000_fe_stock.gfx over it the same way.

If something goes wrong, send the log: <game folder>\\Game\\debug-logs\\gfx_reloader.log
"""


def make_hud(stock, color, add, out):
    """A tinted copy of the stock HUD (tint.py's color transform on every placed object)."""
    data = open(stock, 'rb').read()
    nbits = data[8] >> 3
    tags_start = 8 + (5 + 4 * nbits + 7) // 8 + 4
    rgb = [int(color[i:i + 2], 16) * 256 // 255 for i in (0, 2, 4)]
    add_rgb = [int(add[i:i + 2], 16) for i in (0, 2, 4)] if add else None
    tags, _ = tint.rebuild(data, tags_start, len(data), rgb, False, add_rgb)
    result = bytearray(data[:tags_start] + tags)
    struct.pack_into('<I', result, 4, len(result))
    open(out, 'wb').write(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', choices=KITS, default='ac6')
    parser.add_argument('--no-readme', action='store_true', help="leave README.txt out of the kit")
    args = parser.parse_args()
    kit_info = KITS[args.game]
    kit = os.path.join(ROOT, 'dist', kit_info['name'])

    dll = os.path.join(ROOT, 'dll', 'target', 'release', 'gfx_reloader.dll')
    toast = os.path.join(ROOT, 'tests', 'toast-build', 'release', 'toast_wrapper.dll')
    for path in (dll, toast):
        if not os.path.isfile(path):
            sys.exit(f'missing {path}: build it first (docs/development.md)')
    if os.path.isdir(kit):
        shutil.rmtree(kit)
    os.makedirs(os.path.join(kit, 'mod', 'menu'))
    os.makedirs(os.path.join(kit, 'colored'))
    shutil.copyfile(dll, os.path.join(kit, 'gfx_reloader.dll'))
    shutil.copyfile(toast, os.path.join(kit, 'toast_wrapper.dll'))
    stock = tint.stock_path('01_000_FE', tint.GAMES[args.game][0])
    shutil.copyfile(stock, os.path.join(kit, 'mod', 'menu', HUD))
    shutil.copyfile(stock, os.path.join(kit, 'colored', '01_000_fe_stock.gfx'))
    for name, (color, add) in COLORS.items():
        make_hud(stock, color, add, os.path.join(kit, 'colored', f'01_000_fe_{name}.gfx'))
    files = [(kit_info['name'] + '.me3', PROFILE), ('launch.bat', LAUNCH)]
    if not args.no_readme:
        files.append(('README.txt', README))
    for name, text in files:
        newline = '\r\n' if name.endswith(('.bat', '.txt')) else '\n'
        with open(os.path.join(kit, name), 'w', encoding='utf-8', newline=newline) as f:
            f.write(text.format(**kit_info))
    archive = shutil.make_archive(kit, 'zip', os.path.dirname(kit), os.path.basename(kit))
    print(f'{kit}\n{archive} ({os.path.getsize(archive) // 1024} KB)')


if __name__ == '__main__':
    main()
