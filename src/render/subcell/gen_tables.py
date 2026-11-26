import unicodedata as U, re
assert tuple(map(int, U.unidata_version.split('.'))) >= (16, 0, 0)
def bits(digits): return sum(1 << (int(d) - 1) for d in digits)
# --- octants: bit i-1 = OCTANT-i; 1 2 / 3 4 / 5 6 / 7 8 (row-major)
octa = {}
for cp in range(0x1CD00, 0x1CDE6):
    m = re.fullmatch(r'BLOCK OCTANT-(\d+)', U.name(chr(cp)))
    assert m, hex(cp)
    p = bits(m.group(1)); assert p not in octa; octa[p] = cp
assert len(octa) == 230
# the 26 patterns encoded elsewhere, each checked against its official name
other = {
 0x00: (0x20, 'SPACE'), 0xFF: (0x2588, 'FULL BLOCK'),
 bits('1234'): (0x2580, 'UPPER HALF BLOCK'), bits('5678'): (0x2584, 'LOWER HALF BLOCK'),
 bits('1357'): (0x258C, 'LEFT HALF BLOCK'), bits('2468'): (0x2590, 'RIGHT HALF BLOCK'),
 bits('57'): (0x2596, 'QUADRANT LOWER LEFT'), bits('68'): (0x2597, 'QUADRANT LOWER RIGHT'),
 bits('13'): (0x2598, 'QUADRANT UPPER LEFT'),
 bits('135778'.replace('77','7')+'6'): (0x2599, 'QUADRANT UPPER LEFT AND LOWER LEFT AND LOWER RIGHT'),
 bits('1368'): (0x259A, 'QUADRANT UPPER LEFT AND LOWER RIGHT'),
 bits('123457'): (0x259B, 'QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER LEFT'),
 bits('123468'): (0x259C, 'QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER RIGHT'),
 bits('24'): (0x259D, 'QUADRANT UPPER RIGHT'),
 bits('2457'): (0x259E, 'QUADRANT UPPER RIGHT AND LOWER LEFT'),
 bits('245678'): (0x259F, 'QUADRANT UPPER RIGHT AND LOWER LEFT AND LOWER RIGHT'),
 bits('12'): (0x1FB82, 'UPPER ONE QUARTER BLOCK'), bits('123456'): (0x1FB85, 'UPPER THREE QUARTERS BLOCK'),
 bits('78'): (0x2582, 'LOWER ONE QUARTER BLOCK'), bits('345678'): (0x2586, 'LOWER THREE QUARTERS BLOCK'),
 bits('1'): (0x1CEA8, 'LEFT HALF UPPER ONE QUARTER BLOCK'), bits('2'): (0x1CEAB, 'RIGHT HALF UPPER ONE QUARTER BLOCK'),
 bits('7'): (0x1CEA3, 'LEFT HALF LOWER ONE QUARTER BLOCK'), bits('8'): (0x1CEA0, 'RIGHT HALF LOWER ONE QUARTER BLOCK'),
 bits('35'): (0x1FBE6, 'MIDDLE LEFT ONE QUARTER BLOCK'), bits('46'): (0x1FBE7, 'MIDDLE RIGHT ONE QUARTER BLOCK'),
}
for p, (cp, name) in other.items():
    assert U.name(chr(cp)) == name, (hex(cp), U.name(chr(cp)), name)
    assert p not in octa, (p, name); octa[p] = cp
assert sorted(octa) == list(range(256)) and len(set(octa.values())) == 256
# --- sextants: bit i-1 = SEXTANT-i; 1 2 / 3 4 / 5 6
sext = {0: 0x20, 63: 0x2588, bits('135'): 0x258C, bits('246'): 0x2590}
for cp in range(0x1FB00, 0x1FB3C):
    m = re.fullmatch(r'BLOCK SEXTANT-(\d+)', U.name(chr(cp)))
    assert m, hex(cp)
    p = bits(m.group(1)); assert p not in sext; sext[p] = cp
assert sorted(sext) == list(range(64)) and len(set(sext.values())) == 64
# --- quadrants: bit0 UL, bit1 UR, bit2 LL, bit3 LR
qn = {1:'UPPER LEFT',2:'UPPER RIGHT',4:'LOWER LEFT',8:'LOWER RIGHT'}
quad = {0: 0x20, 15: 0x2588, 3: 0x2580, 12: 0x2584, 5: 0x258C, 10: 0x2590}
for cp in range(0x2596, 0x25A0):
    parts = U.name(chr(cp)).replace('QUADRANT ', '').split(' AND ')
    p = sum(k for k, v in qn.items() if v in parts); assert p not in quad; quad[p] = cp
assert sorted(quad) == list(range(16)) and len(set(quad.values())) == 16
# --- eighth blocks for the web canvas cell rasterizer (HUD bars). `LEFT n/8`
# covers the left n columns of eight, `LOWER n/8` the bottom n rows.
frac = {'ONE EIGHTH': 1, 'ONE QUARTER': 2, 'THREE EIGHTHS': 3, 'HALF': 4,
        'FIVE EIGHTHS': 5, 'THREE QUARTERS': 6, 'SEVEN EIGHTHS': 7}
def bit(row, col): return 1 << (row * 8 + col)
def area(rows, cols): return sum(bit(r, c) for r in rows for c in cols)
eighths = []
for cp in list(range(0x258F, 0x2588, -1)) + list(range(0x2581, 0x2588)):
    m = re.fullmatch(r'(LEFT|LOWER) (.+) BLOCK', U.name(chr(cp)))
    assert m, (hex(cp), U.name(chr(cp)))
    n = frac[m.group(2)]
    mask = area(range(8), range(n)) if m.group(1) == 'LEFT' else area(range(8 - n, 8), range(8))
    assert bin(mask).count('1') == 8 * n
    eighths.append((cp, mask))
assert len(eighths) == 14 and len({cp for cp, _ in eighths}) == 14
# --- 8 x 8 coverage masks (bit `row * 8 + col`, row 0 at the top) of every
# pattern of the three grids. A sub-pixel is four columns wide; its rows are
# the eight sample rows whose centres fall in it (sextants: 3 / 2 / 3).
def grid_masks(bands):
    cells = [area(range(r0, r1), range(c0, c0 + 4)) for (r0, r1) in bands for c0 in (0, 4)]
    assert sum(cells) == (1 << 64) - 1
    return [sum(c for i, c in enumerate(cells) if p >> i & 1) for p in range(1 << len(cells))]
quad_m = grid_masks([(0, 4), (4, 8)])
sext_m = grid_masks([(0, 3), (3, 5), (5, 8)])
octa_m = grid_masks([(0, 2), (2, 4), (4, 6), (6, 8)])
# the shapes two tables share must agree: the halves and the full block
assert quad_m[5] == sext_m[bits('135')] == octa_m[bits('1357')] == eighths[3][1]
assert quad_m[12] == octa_m[bits('5678')] == eighths[10][1] and quad_m[3] == octa_m[bits('1234')]
assert octa_m[bits('78')] == eighths[8][1] and octa_m[bits('345678')] == eighths[12][1]
def emit(name, t, n):
    out = [f'pub(super) const {name}: [char; {n}] = [']
    for i in range(0, n, 8):
        out.append('    ' + ' '.join(f"'\\u{{{t[j]:X}}}'," for j in range(i, i + 8)))
    out.append('];')
    return '\n'.join(out)
def emit_masks(name, t):
    out = [f'pub(super) const {name}: [u64; {len(t)}] = [']
    for i in range(0, len(t), 4):
        out.append('    ' + ' '.join(f'0x{v:016X},' for v in t[i:i + 4]))
    out.append('];')
    return '\n'.join(out)
print('//! Generated by `gen_tables.py` from Python `unicodedata` ' + U.unidata_version + ' character names.')
print('//! Index = bit pattern, bit `row * 2 + col` set = that sub-pixel is foreground.')
print('//! A `*_MASKS` entry is the same pattern as an 8 x 8 coverage mask, bit')
print('//! `row * 8 + col` (row 0 at the top, column 0 at the left).\n')
print(emit('QUADRANTS', quad, 16)); print(); print(emit('SEXTANTS', sext, 64)); print(); print(emit('OCTANTS', octa, 256))
print()
print('/// The eighth blocks: left 1/8 ..= 7/8 (`U+258F` down to `U+2589`), then lower')
print('/// 1/8 ..= 7/8 (`U+2581` up to `U+2587`), each with its coverage mask.')
print(f'pub(super) const EIGHTHS: [(char, u64); {len(eighths)}] = [')
for cp, mask in eighths:
    print(f"    ('\\u{{{cp:X}}}', 0x{mask:016X}), // {U.name(chr(cp))}")
print('];')
print(); print(emit_masks('QUAD_MASKS', quad_m)); print(); print(emit_masks('SEXTANT_MASKS', sext_m)); print(); print(emit_masks('OCTANT_MASKS', octa_m))
