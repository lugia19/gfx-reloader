"""Fills tests/mod/menu with every stock menu movie from the game's unpacked menu folder, so any menu can be edited
mid-session (me3 only serves package files that existed at launch). Also restores everything to stock.

    python -I tools/make_test_package.py [<Game\menu folder>]
"""
import os, shutil, sys

HERE = os.path.dirname(os.path.abspath(__file__))
TARGET = os.path.join(HERE, '..', 'tests', 'mod', 'menu')
DEFAULT_SOURCE = r'D:\SteamLibrary\steamapps\common\ARMORED CORE VI FIRES OF RUBICON\Game\menu'


def main():
    source = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SOURCE
    count = 0
    for sub in ('', 'win'):
        folder = os.path.join(source, sub)
        if not os.path.isdir(folder):
            continue
        os.makedirs(os.path.join(TARGET, sub), exist_ok=True)
        for name in os.listdir(folder):
            if name.lower().endswith('.gfx'):
                shutil.copyfile(os.path.join(folder, name), os.path.join(TARGET, sub, name))
                count += 1
    print(f'{count} stock menu movies copied to {os.path.normpath(TARGET)}')


if __name__ == '__main__':
    main()
