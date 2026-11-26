#!/usr/bin/env python3
"""A line-for-line Python model of `xbr.rs` (the 1x `Smooth::Xbr` pass).

Dev tool only: nothing in the game reads it. It exists because the Rust is
integer arithmetic that is easy to mirror, so a change to the rules can be
looked at on a real frame before it is compiled:

    python3 xbr_sim.py frame.ppm out_prefix [--set quad|sextant|octant] [--crisp]

`frame.ppm` is a `RATRUN_DUMP` capture (W x 2H, one pixel a half cell). Writes
`<prefix>-before.png`, `<prefix>-after.png` (9 x 18 px a cell) and prints the
cell counts. Needs Pillow only for reading and writing the pictures; the pass
itself (`frame()`) is plain integers on lists.
"""
import sys

EDGE = 48            # subcell.rs `EDGE`
GATE = 6             # xbr.rs `GATE`: the least channel step the blended fit reads
EQ = 155             # xBR's `eq` threshold on the YUV distance
BASE = 512           # a sub-pixel's weights sum to this
BRIGHT_PEAK, SAT_PEAK, SAT_RANGE = 172, 96, 80
ISOLATED, ALIGNED, ACROSS = 30, 16, 36
E_R, E_D = 1, 2


def differs(a, b):
    return abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2]) >= EDGE


def tdiv(a, b):
    """C / Rust integer division (toward zero)."""
    q = abs(a) // b
    return q if a >= 0 else -q


def yuv(c):
    r, g, b = c
    rg, bg = r - g, b - g
    u = tdiv(-169 * rg + 500 * bg, 1000) + 128
    v = tdiv(500 * rg - 81 * bg, 1000) + 128
    start = max(-bg, -rg, 0)
    y = tdiv(299 * rg + 1000 * start + 114 * bg, 1000) + (g - start)
    return (y, u, v)


def df(a, b):
    return abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2])


def luma(c):
    return c[0] * 3 + c[1] * 6 + c[2]


def brighter(a, b):
    """Is `a` the brighter of two colours (a total order, so label-free)?"""
    return (luma(a), a) > (luma(b), b)


class Pic:
    def __init__(self, w, h, px, crisp=True):
        self.w, self.h, self.px = w, h, px
        self.yuv = [yuv(c) for c in px]
        self.crisp = crisp
        step = differs if crisp else (lambda a, b: max(abs(a[0] - b[0]), abs(a[1] - b[1]), abs(a[2] - b[2])) >= GATE)
        self.flags = [0] * (w * h)
        for y in range(h):
            for x in range(w):
                i = y * w + x
                f = 0
                if x + 1 < w and step(px[i], px[i + 1]):
                    f |= E_R
                if y + 1 < h and step(px[i], px[i + w]):
                    f |= E_D
                self.flags[i] = f

    def at(self, x, y):
        x = min(max(x, 0), self.w - 1)
        y = min(max(y, 0), self.h - 1)
        return y * self.w + x

    def edge(self, x, y, dx, dy):
        """Does pixel (x, y) differ (by EDGE) from its axial neighbour?"""
        if dx == 1:
            return x + 1 < self.w and self.flags[y * self.w + x] & E_R != 0
        if dx == -1:
            return x > 0 and self.flags[y * self.w + x - 1] & E_R != 0
        if dy == 1:
            return y + 1 < self.h and self.flags[y * self.w + x] & E_D != 0
        return y > 0 and self.flags[(y - 1) * self.w + x] & E_D != 0

    def active(self, x, y):
        r, d = self.edge(x, y, 1, 0), self.edge(x, y, 0, 1)
        l, u = self.edge(x, y, -1, 0), self.edge(x, y, 0, -1)
        return (r and d) or (d and l) or (l and u) or (u and r)

    def chan(self, i, j):
        a, b = self.px[i], self.px[j]
        return max(abs(a[0] - b[0]), abs(a[1] - b[1]), abs(a[2] - b[2]))

    def bright_isolated(self, x, y):
        c = self.px[y * self.w + x]
        hi, lo = max(c), min(c)
        if not (hi > BRIGHT_PEAK or (hi > SAT_PEAK and hi - lo > SAT_RANGE)):
            return False
        i = y * self.w + x
        n = sum(self.chan(i, self.at(x + dx, y + dy)) > ISOLATED for dx, dy in ((0, -1), (1, 0), (0, 1), (-1, 0)))
        return n >= 3

    def feature(self, x, y):
        if self.bright_isolated(x, y):
            return True
        i = y * self.w + x
        for dx, dy in ((1, 0), (0, 1), (1, 1), (1, -1)):
            if (self.chan(i, self.at(x + dx, y + dy)) < ALIGNED
                    and self.chan(i, self.at(x - dx, y - dy)) < ALIGNED
                    and self.chan(i, self.at(x - dy, y + dx)) > ACROSS
                    and self.chan(i, self.at(x + dy, y - dx)) > ACROSS):
                return True
        return False


# Direction index of an axial offset: 0 right, 1 down, 2 left, 3 up.
DIRS = ((1, 0), (0, 1), (-1, 0), (0, -1))


def rot(k, dx, dy):
    for _ in range(k):
        dx, dy = dy, -dx
    return dx, dy


def xbr_pixel(p, x, y):
    """FFmpeg `xbr=2` (FILT2) for one pixel, as weights: `w[sub][dir]` is the
    share (of BASE) sub-pixel `sub` (`sy * 2 + sx`) takes from the axial
    neighbour `dir`; the rest is the pixel's own colour."""
    w = [[0, 0, 0, 0] for _ in range(4)]
    px, yv = p.px, p.yuv
    e = y * p.w + x
    for k in range(4):
        def n(dx, dy):
            rx, ry = rot(k, dx, dy)
            return p.at(x + rx, y + ry)
        fx, fy = rot(k, 1, 0)
        hx, hy = rot(k, 0, 1)
        if not (p.edge(x, y, fx, fy) and p.edge(x, y, hx, hy)):
            continue
        f, h = n(1, 0), n(0, 1)
        i_, g, c = n(1, 1), n(-1, 1), n(1, -1)
        d, b = n(-1, 0), n(0, -1)
        f4, i4, h5, i5 = n(2, 0), n(2, 1), n(0, 2), n(1, 2)
        D = lambda a, bb: df(yv[a], yv[bb])
        eq = lambda a, bb: D(a, bb) < EQ
        ee = D(e, c) + D(e, g) + D(i_, h5) + D(i_, f4) + (D(h, f) << 2)
        ii = D(h, d) + D(h, i5) + D(f, i4) + D(f, b) + (D(e, i_) << 2)
        if ee > ii:
            continue
        # Crisp only. A stair, not a corner: the pixel's colour carries on
        # along the diagonal on at least one side. (A rectangle keeps its
        # corners.)
        if p.crisp and differs(px[e], px[c]) and differs(px[e], px[g]):
            continue
        ne = differs if p.crisp else (lambda a, bb: a != bb)
        take_f = D(e, f) <= D(e, h)
        donor, other = (f, h) if take_f else (h, f)
        ddir = DIRS.index((fx, fy) if take_f else (hx, hy))
        # A protected mark never lends its colour (nor does its twin).
        dxy = (donor % p.w, donor // p.w)
        oxy = (other % p.w, other // p.w)
        if p.feature(*dxy) or (p.chan(donor, other) < ALIGNED and p.feature(*oxy)):
            continue

        def sub(cx, cy):
            rx, ry = rot(k, cx, cy)
            return (ry + 1) // 2 * 2 + (rx + 1) // 2
        n3, n2, n1 = sub(1, 1), sub(-1, 1), sub(1, -1)

        def blend(s, num, den):
            for q in range(4):
                w[s][q] -= w[s][q] * num // den
            w[s][ddir] += BASE * num // den
        if ee < ii and ((not eq(f, b) and not eq(h, d)) or (eq(e, i_) and not eq(f, i4) and not eq(h, i5))
                        or eq(e, g) or eq(e, c)):
            ke, ki = D(f, g), D(h, c)
            left = (ke << 1) <= ki and ne(px[e], px[g]) and ne(px[d], px[g])
            up = ke >= (ki << 1) and ne(px[e], px[c]) and ne(px[b], px[c])
            if left and up:
                blend(n3, 7, 8)
                blend(n2, 1, 4)
                w[n1] = list(w[n2])
            elif left:
                blend(n3, 3, 4)
                blend(n2, 1, 4)
            elif up:
                blend(n3, 3, 4)
                blend(n1, 1, 4)
            else:
                blend(n3, 1, 2)
        else:
            blend(n3, 1, 2)
    return w


class Pair:
    def __init__(self, x):
        self.x, self.y = x, None

    def cls(self, c):
        if c == self.x:
            return False
        if self.y is not None and c == self.y:
            return True
        if not differs(c, self.x):
            return False
        if self.y is not None:
            return True if not differs(c, self.y) else None
        self.y = c
        return True


# Rows of the 2 x 4 block each sub-cell row of a grid covers, as weights on a
# 12-row mesh (a block row is 3 mesh rows).
BANDS = {
    'quad': ((3, 3, 0, 0), (0, 0, 3, 3)),
    'sextant': ((3, 1, 0, 0), (0, 2, 2, 0), (0, 0, 1, 3)),
    'octant': ((3, 0, 0, 0), (0, 3, 0, 0), (0, 0, 3, 0), (0, 0, 0, 3)),
}
FAMILIES = {'quad': ('quad',), 'sextant': ('quad', 'sextant'), 'octant': ('quad', 'sextant', 'octant')}
PLAIN = ('quad', None)


def reduce(cov, bands, x_wins_ties):
    """The pattern of one grid nearest the coverage, and its error."""
    pat, err = 0, 0
    for r, band in enumerate(bands):
        total = sum(band) * BASE
        for cidx in range(2):
            c = sum(band[j] * cov[j * 2 + cidx] for j in range(4))
            if c * 2 > total or (c * 2 == total and not x_wins_ties):
                pat |= 1 << (r * 2 + cidx)
                err += total - c
            else:
                err += c
    return pat, err


def dist(a, b):
    return (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2


def rdiv(a, b):
    return (a + b // 2) // b


def split(px):
    """Two-means of a cell's sub-cells (`encode`'s seeds and two refinement
    steps, unweighted): (mask of the first cluster, its colour, the other's),
    or None when they are all one colour."""
    n = len(px)
    mean = tuple(sum(c[k] for c in px) // n for k in range(3))
    a = max(range(n), key=lambda i: (dist(px[i], mean), -i))
    b = max(range(n), key=lambda i: (dist(px[i], px[a]), -i))
    if dist(px[b], px[a]) == 0:
        return None
    ca, cb, mask = px[a], px[b], 0
    for _ in range(2):
        nxt, sa, sb = 0, [0, 0, 0, 0], [0, 0, 0, 0]
        for i, c in enumerate(px):
            if dist(c, ca) <= dist(c, cb):
                nxt |= 1 << i
                t = sa
            else:
                t = sb
            for k in range(3):
                t[k] += c[k]
            t[3] += 1
        if sa[3] == 0 or sb[3] == 0:
            break
        mask = nxt
        ca = tuple(rdiv(sa[k], sa[3]) for k in range(3))
        cb = tuple(rdiv(sb[k], sb[3]) for k in range(3))
    if mask == 0:
        return None
    return mask, ca, cb


def cell_blend(p, cx, cy, glyph_set):
    """One cell of xBR's blended block, fitted in two colours."""
    w = p.w
    top, bot = p.px[cy * 2 * w + cx], p.px[(cy * 2 + 1) * w + cx]
    plain = (PLAIN, top, bot)
    if not (p.active(cx, cy * 2) or p.active(cx, cy * 2 + 1)):
        return plain
    if p.feature(cx, cy * 2) or p.feature(cx, cy * 2 + 1):
        return plain
    block = []
    moved = False
    for y0 in (cy * 2, cy * 2 + 1):
        ws = xbr_pixel(p, cx, y0)
        own = p.px[y0 * w + cx]
        for s in range(4):
            acc = [own[k] * BASE for k in range(3)]
            for q, (dx, dy) in enumerate(DIRS):
                if ws[s][q]:
                    moved = True
                    n = p.px[p.at(cx + dx, y0 + dy)]
                    for k in range(3):
                        acc[k] += (n[k] - own[k]) * ws[s][q]
            block.append(tuple(rdiv(v, BASE) for v in acc))
    if not moved:
        return plain

    def error(bands, pat, fg, bg):
        e = 0
        for r, band in enumerate(bands):
            for col in range(2):
                c = fg if pat >> (r * 2 + col) & 1 else bg
                for j in range(4):
                    if band[j]:
                        e += band[j] * dist(block[j * 2 + col], c)
        return e
    best = (PLAIN, top, bot)
    best_err = error(BANDS['quad'], 0b0011, top, bot)
    for fam in FAMILIES[glyph_set]:
        bands = BANDS[fam]
        sub = []
        for band in bands:
            tot = sum(band)
            for col in range(2):
                sub.append(tuple(rdiv(sum(band[j] * block[j * 2 + col][k] for j in range(4)), tot) for k in range(3)))
        sp = split(sub)
        full = (1 << len(sub)) - 1
        if sp is None:
            cand = (('quad', 0b0011), sub[0], sub[0])
            err = error(BANDS['quad'], 0, sub[0], sub[0])
        else:
            mask, ca, cb = sp
            if fam == 'quad' and mask in (0b0011, 0b1100):
                fg, bg = (ca, cb) if mask == 0b0011 else (cb, ca)
                cand = (PLAIN if (fg, bg) == (top, bot) else ('quad', 0b0011), fg, bg)
                err = error(bands, 0b0011, fg, bg)
            elif luma(ca) >= luma(cb):
                cand, err = ((fam, mask), ca, cb), error(bands, mask, ca, cb)
            else:
                cand, err = ((fam, mask ^ full), cb, ca), error(bands, mask ^ full, cb, ca)
        if err < best_err:
            best, best_err = cand, err
    return best


def cell(p, cx, cy, glyph_set):
    """One cell: ('▀'-plain marker or (family, pattern)), fg, bg."""
    if not p.crisp:
        return cell_blend(p, cx, cy, glyph_set)
    w = p.w
    t, b = cy * 2 * w + cx, (cy * 2 + 1) * w + cx
    top, bot = p.px[t], p.px[b]
    plain = (PLAIN, top, bot)
    if not (p.active(cx, cy * 2) or p.active(cx, cy * 2 + 1)):
        return plain
    if p.feature(cx, cy * 2) or p.feature(cx, cy * 2 + 1):
        return plain
    wt, wb = xbr_pixel(p, cx, cy * 2), xbr_pixel(p, cx, cy * 2 + 1)
    if not any(any(s) for s in wt) and not any(any(s) for s in wb):
        return plain
    pair = Pair(top)
    bot_y = pair.cls(bot)
    if bot_y is None:
        return plain
    cov = [0] * 8
    for half, (ws, own, y0) in enumerate(((wt, False, cy * 2), (wb, bot_y, cy * 2 + 1))):
        # Only a neighbour that lends something is classified, in a fixed
        # order; one of a third colour lends nothing.
        for s in range(4):
            share = [0, 0]  # by class
            rest = BASE
            for q, (dx, dy) in enumerate(DIRS):
                if ws[s][q] == 0:
                    continue
                k = pair.cls(p.px[p.at(cx + dx, y0 + dy)])
                if k is None:
                    continue
                share[k] += ws[s][q]
                rest -= ws[s][q]
            share[own] += rest
            cov[half * 4 + s] = share[1]
    if pair.y is None:
        return plain
    x, y = pair.x, pair.y
    x_wins = brighter(x, y)
    orig = (0b1100 if bot_y else 0)
    if glyph_set == 'quad':
        # One sub-row a pixel: it shows the block row that leaned further.
        pat = 0
        for half, own in ((0, False), (1, bot_y)):
            for col in range(2):
                a, b2 = cov[half * 4 + col], cov[half * 4 + 2 + col]
                c = min(a, b2) if own else max(a, b2)
                if c * 2 > BASE or (c * 2 == BASE and not x_wins):
                    pat |= 1 << (half * 2 + col)
        if pat == orig:
            return plain
        if pat in (0, 15):
            c = y if pat else x
            return (('quad', 0b0011), c, c)
        return (('quad', pat), y, x) if luma(y) >= luma(x) else (('quad', pat ^ 15), x, y)
    best = None
    for fam in FAMILIES[glyph_set]:
        pat, err = reduce(cov, BANDS[fam], x_wins)
        if best is None:
            best = (fam, pat, err)
        elif err < best[2] or (err == best[2] and best[0] == 'quad' and best[1] == orig):
            best = (fam, pat, err)
    fam, pat, _ = best
    if fam == 'quad' and pat == orig:
        return plain
    full = (1 << (2 * len(BANDS[fam]))) - 1
    if pat == 0 or pat == full:
        c = y if pat else x
        return (('quad', 0b0011), c, c)
    if luma(y) >= luma(x):
        return ((fam, pat), y, x)
    return ((fam, pat ^ full), x, y)


def frame(p, glyph_set, y0=0, y1=None):
    rows = p.h // 2
    y1 = rows if y1 is None else y1
    out = []
    for cy in range(rows):
        row = []
        for cx in range(p.w):
            if y0 <= cy < y1:
                row.append(cell(p, cx, cy, glyph_set))
            else:
                row.append((PLAIN, p.px[cy * 2 * p.w + cx], p.px[(cy * 2 + 1) * p.w + cx]))
        out.append(row)
    return out


def render(cells, path):
    from PIL import Image
    rows, cols = len(cells), len(cells[0])
    im = Image.new('RGB', (cols * 9, rows * 18))
    pix = im.load()
    for cy, row in enumerate(cells):
        for cx, ((fam, pat), fg, bg) in enumerate(row):
            if pat is None:
                fam, pat = 'quad', 0b0011
            k = len(BANDS[fam])
            for dy in range(18):
                r = (dy * 12 // 18) * k // 12
                for dx in range(9):
                    on = pat >> (r * 2 + dx * 2 // 9) & 1
                    pix[cx * 9 + dx, cy * 18 + dy] = fg if on else bg
    im.save(path)


def load(path, crisp=False):
    from PIL import Image
    im = Image.open(path).convert('RGB')
    return Pic(im.width, im.height, [im.getpixel((x, y)) for y in range(im.height) for x in range(im.width)], crisp)


if __name__ == '__main__':
    a = sys.argv[1:]
    glyph_set = a[a.index('--set') + 1] if '--set' in a else 'sextant'
    p = load(a[0], '--crisp' in a)
    rows = p.h // 2
    before = frame(p, glyph_set, 0, 0)
    after = frame(p, glyph_set, 1, rows - 2)   # the dump's HUD rows stay plain
    render(before, a[1] + '-before.png')
    render(after, a[1] + '-after.png')
    flat = [c for row in after for c in row]
    plain = [c for row in before for c in row]
    changed = sum(1 for c, q in zip(flat, plain) if c != q)
    shaped = sum(1 for (g, _, _) in flat if g[1] is not None and g != ('quad', 0b0011))
    fams = {}
    for (g, _, _) in flat:
        if g[1] is not None:
            fams[g[0]] = fams.get(g[0], 0) + 1
    print(a[0].split('/')[-1], 'cells', len(flat), 'changed', changed, 'reshaped', shaped, fams)
