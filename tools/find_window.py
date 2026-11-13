"""Finds the menu window object that owns a menu's player in the running game, by scanning writable memory for the
player pointer (taken from the probe log's last "open <name>" line).

    python -I tools/find_window.py 01_GarageTopMenu

Every hit is printed with the window candidates it implies: main-menu windows (0x140d61b3e: player +0x68, entry copy
+0x58, name pointer +0x60) and panel windows (0x140c8acf0: player +0x60, entry copy +0x48, name +0x50). For each
candidate whose name pointer matches, it dumps the first 0x150 bytes as qwords (vtable at +0).
"""
import ctypes, re, struct, subprocess, sys
from ctypes import wintypes

LOG = r'D:\SteamLibrary\steamapps\common\ARMORED CORE VI FIRES OF RUBICON\Game\debug-logs\ui_hot_reload.log'
# (description, offset of the player in the window, offset of the name pointer in the window)
LAYOUTS = [('main-menu window (player +0x68)', 0x68, 0x60), ('panel window (player +0x60)', 0x60, 0x50)]


def game_pid():
    out = subprocess.run(['tasklist', '/FI', 'IMAGENAME eq armoredcore6.exe', '/FO', 'CSV', '/NH'],
                         capture_output=True, text=True).stdout.splitlines()
    out = [line for line in out if line.startswith('"armoredcore6')]
    if not out:
        sys.exit('armoredcore6.exe is not running')
    return int(out[0].split('","')[1])


KERNEL32 = ctypes.WinDLL('kernel32', use_last_error=True)
KERNEL32.OpenProcess.restype = wintypes.HANDLE


class MemoryInfo(ctypes.Structure):
    _fields_ = [('BaseAddress', ctypes.c_uint64), ('AllocationBase', ctypes.c_uint64),
                ('AllocationProtect', ctypes.c_uint32), ('PartitionId', ctypes.c_uint16),
                ('RegionSize', ctypes.c_uint64), ('State', ctypes.c_uint32), ('Protect', ctypes.c_uint32),
                ('Type', ctypes.c_uint32)]


KERNEL32.VirtualQueryEx.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.POINTER(MemoryInfo), ctypes.c_size_t]
GAME = KERNEL32.OpenProcess(0x0010 | 0x0400, False, game_pid())


def read_memory(addr, n):
    buf = ctypes.create_string_buffer(n)
    got = ctypes.c_size_t()
    ok = KERNEL32.ReadProcessMemory(GAME, ctypes.c_void_p(addr), buf, n, ctypes.byref(got))
    return buf.raw[:got.value] if ok or got.value else b''


def qword(addr):
    data = read_memory(addr, 8)
    return struct.unpack('<Q', data)[0] if len(data) == 8 else None


def wide(addr, max_chars=64):
    data = read_memory(addr, max_chars * 2) if addr else b''
    text = data.decode('utf-16-le', 'replace')
    return text.split('\0')[0]


def writable_regions():
    addr = 0x10000
    while addr < 0x7fffffff0000:
        info = MemoryInfo()
        if not KERNEL32.VirtualQueryEx(GAME, ctypes.c_void_p(addr), ctypes.byref(info), ctypes.sizeof(info)):
            break
        if info.State == 0x1000 and info.Protect in (0x04, 0x40):
            yield info.BaseAddress, info.RegionSize
        addr = info.BaseAddress + info.RegionSize


def find_qword(value, step=64 << 20):
    needle = struct.pack('<Q', value)
    hits = []
    for base, size in writable_regions():
        at = base
        while at < base + size:
            data = read_memory(at, min(step, base + size - at))
            i = data.find(needle)
            while i != -1:
                if i % 8 == 0:
                    hits.append(at + i)
                i = data.find(needle, i + 1)
            at += step
    return hits


def last_player(name):
    pattern = re.compile(r' open ' + re.escape(name) + r' \(flag [^)]*\): player (0x[0-9a-f]+)')
    player = None
    with open(LOG, encoding='utf-8', errors='replace') as f:
        for line in f:
            m = pattern.search(line)
            if m:
                player = int(m.group(1), 16)
    return player


def main():
    name = sys.argv[1]
    player = last_player(name)
    if not player:
        sys.exit(f'no "open {name}" line in the log')
    print(f'{name}: player {player:#x}')
    for hit in find_qword(player):
        print(f'  hit {hit:#x}')
        for desc, player_off, name_off in LAYOUTS:
            window = hit - player_off
            name_ptr = qword(window + name_off)
            if name_ptr and wide(name_ptr) == name:
                print(f'    -> {desc}: window {window:#x}, vtable {qword(window):#x}')
                data = read_memory(window, 0x150)
                for off in range(0, len(data), 8):
                    v = struct.unpack_from('<Q', data, off)[0]
                    note = ''
                    if 0x140000000 <= v < 0x147000000:
                        note = '  (exe)'
                    print(f'       +{off:#05x} {v:#018x}{note}')


if __name__ == '__main__':
    main()
