"""Writes a visibly tinted copy of a stock menu movie into the test package, for checking whether a reload shows up.

    python -I tools/tint.py 02_coloring 00FF00     # green: RGB multiply (0, 1, 0) on every placed object
    python -I tools/tint.py 01_900_Black FFFFFF --add FF0000   # add red: shows on black art too
    python -I tools/tint.py 02_coloring --restore  # stock file back
    python -I tools/tint.py --game er 01_000_FE 00FF00   # Elden Ring (tests/mod-er)

Every PlaceObject2/3 that places a character (in the root timeline and in every sprite) gets a color transform
multiplying RGB by the given color; existing color transforms are scaled by it (add terms and alpha kept). Pure
channels (00FF00, FF00FF...) compound to the same tint through nested sprites. Objects created by ActionScript
inherit the tint of the clip they're attached to. Reads the stock file from the game's unpacked menu folder.
"""
import argparse, os, shutil, struct, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
# game -> (unpacked stock menu folder, test package menu folder)
GAMES = {
    'ac6': (r'D:\SteamLibrary\steamapps\common\ARMORED CORE VI FIRES OF RUBICON\Game\menu',
            os.path.join(HERE, '..', 'tests', 'mod', 'menu')),
    'er': (r'D:\SteamLibrary\steamapps\common\ELDEN RING\Game\menu', os.path.join(HERE, '..', 'tests', 'mod-er', 'menu')),
}


class BitReader:
    def __init__(self, data, byte_pos):
        self.data = data
        self.pos = byte_pos * 8

    def unsigned(self, n):
        value = 0
        for _ in range(n):
            value = (value << 1) | ((self.data[self.pos >> 3] >> (7 - (self.pos & 7))) & 1)
            self.pos += 1
        return value

    def signed(self, n):
        value = self.unsigned(n)
        return value - (1 << n) if n and value >> (n - 1) else value


def read_cxform(data, pos):
    """(mult or None, add or None, length in bytes) of the CXFORMWITHALPHA at data[pos]."""
    bits = BitReader(data, pos)
    has_add, has_mult, n = bits.unsigned(1), bits.unsigned(1), bits.unsigned(4)
    mult = [bits.signed(n) for _ in range(4)] if has_mult else None
    add = [bits.signed(n) for _ in range(4)] if has_add else None
    return mult, add, (bits.pos + 7) // 8 - pos


def encode_cxform(mult, add):
    values = (mult or []) + (add or [])
    n = max([1] + [(v if v >= 0 else ~v).bit_length() + 1 for v in values])
    assert n < 16, values
    out = [1 if add else 0, 1 if mult else 0] + [(n >> (3 - i)) & 1 for i in range(4)]
    for v in values:
        v &= (1 << n) - 1
        out += [(v >> (n - 1 - i)) & 1 for i in range(n)]
    out += [0] * (-len(out) % 8)
    return bytes(sum(bit << (7 - i) for i, bit in enumerate(out[k:k + 8])) for k in range(0, len(out), 8))


def cxform_position(body, code):
    """Offset in a PlaceObject2/3 body where the color transform is (or would go)."""
    flags = body[0]
    pos = 1
    if code == 70:
        flags2 = body[pos]
        pos += 1
    pos += 2  # depth
    if code == 70 and flags2 & 0x08:  # class name (the spec's "or HasImage and HasCharacter" doesn't hold here)
        pos = body.index(0, pos) + 1
    if flags & 0x02:
        pos += 2  # character id
    if flags & 0x04:  # matrix
        bits = BitReader(body, pos)
        if bits.unsigned(1):
            n = bits.unsigned(5); bits.unsigned(n * 2)
        if bits.unsigned(1):
            n = bits.unsigned(5); bits.unsigned(n * 2)
        n = bits.unsigned(5); bits.unsigned(n * 2)
        pos = (bits.pos + 7) >> 3
    return pos


def tint_place(body, code, rgb, add_rgb=None):
    """New body, or None if the tag is left alone. rgb multiplies (0..256); add_rgb (0..255) is added on top, which
    also colors black art (a multiply can't)."""
    flags = body[0]
    pos = cxform_position(body, code)
    if flags & 0x08:
        mult, add, length = read_cxform(body, pos)
        mult = mult or [256, 256, 256, 256]
        mult = [m * c // 256 for m, c in zip(mult[:3], rgb)] + [mult[3]]
        if add_rgb:
            add = add or [0, 0, 0, 0]
            add = [min(255, a + c) for a, c in zip(add[:3], add_rgb)] + [add[3]]
        return body[:pos] + encode_cxform(mult, add) + body[pos + length:]
    if flags & 0x02:
        add = list(add_rgb) + [0] if add_rgb else None
        return bytes([flags | 0x08]) + body[1:pos] + encode_cxform(list(rgb) + [256], add) + body[pos:]
    return None


def rebuild(data, start, end, rgb, nested, add_rgb=None):
    """(rebuilt tag stream, number of placements tinted)."""
    out = bytearray()
    tinted = 0
    pos = start
    while pos < end:
        header = struct.unpack_from('<H', data, pos)[0]
        code, length = header >> 6, header & 63
        long_form = length == 63
        pos += 2
        if long_form:
            length = struct.unpack_from('<I', data, pos)[0]
            pos += 4
        body = data[pos:pos + length]
        pos += length
        if code == 39:
            sprite, count = rebuild(data, pos - length + 4, pos, rgb, True, add_rgb)
            body = body[:4] + sprite
            tinted += count
        elif code in (26, 70):
            new = tint_place(body, code, rgb, add_rgb)
            if new is not None:
                body = new
                tinted += 1
        if long_form or len(body) >= 63:
            out += struct.pack('<HI', (code << 6) | 63, len(body))
        else:
            out += struct.pack('<H', (code << 6) | len(body))
        out += body
        if code == 0:
            break
    if not nested:
        out += data[pos:end]  # anything after the End tag, kept as is
    return bytes(out), tinted


def stock_path(name, game_menu):
    for folder in (game_menu, os.path.join(game_menu, 'win')):
        for candidate in os.listdir(folder):
            if candidate.lower() == name.lower() + '.gfx':
                return os.path.join(folder, candidate)
    raise SystemExit(f'{name}.gfx not found in {game_menu}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('name', help='menu name, e.g. 02_coloring')
    parser.add_argument('color', nargs='?', default='00FF00', help='RGB multiply as RRGGBB (default 00FF00)')
    parser.add_argument('--add', metavar='RRGGBB', help='also add this color (makes black art visible)')
    parser.add_argument('--restore', action='store_true', help='copy the stock file back')
    parser.add_argument('--game', choices=GAMES, default='ac6')
    args = parser.parse_args()
    game_menu, package_menu = GAMES[args.game]

    source = stock_path(args.name, game_menu)
    sub = 'win' if os.path.basename(os.path.dirname(source)).lower() == 'win' else ''
    target = os.path.normpath(os.path.join(package_menu, sub, os.path.basename(source)))
    if not os.path.exists(target):
        print(f'warning: {target} did not exist; me3 only serves files that existed at launch')
    if args.restore:
        shutil.copyfile(source, target)
        print(f'restored {target}')
        return

    data = open(source, 'rb').read()
    if data[:3] == b'CFX':
        data = b'GFX' + data[3:8] + zlib.decompress(data[8:])
    assert data[:3] == b'GFX', data[:3]
    nbits = data[8] >> 3
    tags_start = 8 + (5 + 4 * nbits + 7) // 8 + 4
    rgb = [int(args.color[i:i + 2], 16) * 256 // 255 for i in (0, 2, 4)]
    add_rgb = [int(args.add[i:i + 2], 16) for i in (0, 2, 4)] if args.add else None
    tags, tinted = rebuild(data, tags_start, len(data), rgb, False, add_rgb)
    out = bytearray(data[:tags_start] + tags)
    struct.pack_into('<I', out, 4, len(out))
    with open(target, 'wb') as f:
        f.write(out)
    added = f' + {args.add}' if args.add else ''
    print(f'{target}: {tinted} placements tinted {args.color}{added} ({len(data)} -> {len(out)} bytes)')


if __name__ == '__main__':
    main()
