"""Fills (or resets) the test package with every stock menu movie; me3 only serves files present at launch.

    python -I tools/make_test_package.py              # Armored Core VI -> tests/mod/menu
    python -I tools/make_test_package.py --game er    # Elden Ring       -> tests/mod-er/menu
"""
import argparse, os, shutil, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's folder out
from tint import GAMES


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', choices=GAMES, default='ac6')
    parser.add_argument('--source', help="the game's unpacked menu folder (default: the usual Steam path)")
    args = parser.parse_args()
    default_source, package_menu = GAMES[args.game]
    source = args.source or default_source
    target = os.path.normpath(package_menu)
    count = 0
    for sub in ('', 'win'):
        folder = os.path.join(source, sub)
        if not os.path.isdir(folder):
            continue
        os.makedirs(os.path.join(target, sub), exist_ok=True)
        for name in os.listdir(folder):
            if name.lower().endswith('.gfx'):
                shutil.copyfile(os.path.join(folder, name), os.path.join(target, sub, name))
                count += 1
    print(f'{count} stock menu movies copied to {target}')


if __name__ == '__main__':
    main()
