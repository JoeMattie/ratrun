#!/usr/bin/env python3
"""Generate Rat Run's PWA app icons from the title screen's rat plate.

Reads `assets/rat.png` (the same embedded source `src/titleart.rs` uses),
quantizes it onto a 64x64 pixel-art canvas on `#090a0d` with the title
screen's coat gradient, pink ear/tail (`palette::TAIL`), dark pixel-art
outline and facility registration corner marks (`palette::EDGE`), and writes:

  - web/icons/icon-192.png     (192x192, exact 3x pixel scale)
  - web/icons/icon-512.png     (512x512, exact 8x pixel scale)
  - web/icons/maskable-512.png (512x512, art strictly inside the 80% safe circle)

Run from the repository root or from `web/icons/`:
    python3 web/icons/make_icons.py
"""

from __future__ import annotations

import math
from pathlib import Path
from PIL import Image

# ---- House Palette (`web/canvas.js`, `src/render/palette.rs`, `menu.rs`) ---

BG = (9, 10, 13)             # #090a0d (web canvas background)
PLATE = (19, 24, 27)         # palette::PLATE
PLATE_HI = (36, 44, 48)      # subtle upper-left plate bevel
PLATE_LO = (13, 16, 19)      # subtle lower-right plate shadow
TRACK = (44, 52, 56)         # palette::TRACK
EDGE = (82, 96, 100)         # palette::EDGE (title registration brackets)
EDGE_HI = (118, 134, 138)    # lit top-left corner registration mark
OUTLINE = (16, 20, 24)       # crisp 1-px pixel-art silhouette keyline
SHADOW = (12, 15, 18)        # grounded contact shadow under paws

COAT_TOP = (248, 238, 239)   # menu.rs title rat top coat
COAT_BOT = (169, 163, 175)   # menu.rs title rat bottom coat
COAT_HI = (255, 250, 248)    # outer ear / sunlit rim
PAW_SHADE = (146, 142, 154)  # shaded under-paws
TAIL = (239, 168, 185)       # palette::TAIL (ear & curling tail)
TAIL_SHADE = (198, 124, 146) # shaded underside of tail / ear crease
EAR_INNER = (168, 86, 114)   # deep pink inner ear hollow (from dark inner ear)
EYE = (18, 14, 22)           # crisp dark specimen eye
NOSE = (224, 132, 154)       # soft pink nose tip
NOSE_DARK = (54, 40, 48)     # nose rim

GRID = 64  # gcd(192, 512) == 64 -> exact integer nearest-neighbor scaling!


def mix(a: tuple[int, int, int], b: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    t = max(0.0, min(1.0, t))
    return (
        int(round(a[0] + (b[0] - a[0]) * t)),
        int(round(a[1] + (b[1] - a[1]) * t)),
        int(round(a[2] + (b[2] - a[2]) * t)),
    )


def load_rat_cropped(repo_root: Path) -> Image.Image:
    src = Image.open(repo_root / "assets" / "rat.png").convert("L")
    w, h = src.size
    px = src.load()
    xs, ys = [], []
    for y in range(h):
        for x in range(w):
            if px[x, y] > 10:
                xs.append(x)
                ys.append(y)
    return src.crop((min(xs), min(ys), max(xs) + 1, max(ys) + 1))


def rasterize_rat(cropped: Image.Image, target_w: int) -> list[list[tuple[int, int, int] | None]]:
    """Quantize the cropped title rat into a clean pixel-art sprite."""
    cw, ch = cropped.size
    target_h = max(1, round(target_w * ch / cw))

    # Supersample each target pixel by reading a 4x4 block in source space so
    # tiny features (the dark eye, nose tip, thin curling tail and inner ear)
    # survive quantization sharply instead of blurring out.
    src_px = cropped.load()
    grid: list[list[tuple[int, int, int] | None]] = [[None] * target_w for _ in range(target_h)]
    mask = [[False] * target_w for _ in range(target_h)]
    vals = [[0.0] * target_w for _ in range(target_h)]
    mins = [[255] * target_w for _ in range(target_h)]
    maxs = [[0] * target_w for _ in range(target_h)]

    for ty in range(target_h):
        for tx in range(target_w):
            hits = 0
            s_val = 0
            lo = 255
            hi = 0
            for sy in range(4):
                yy = min(ch - 1, int((ty + (sy + 0.5) / 4.0) * ch / target_h))
                for sx in range(4):
                    xx = min(cw - 1, int((tx + (sx + 0.5) / 4.0) * cw / target_w))
                    v = src_px[xx, yy]
                    if v > 10:
                        hits += 1
                        s_val += v
                        if v < lo:
                            lo = v
                        if v > hi:
                            hi = v
            # Keep thin tail pixels at a slightly lower coverage threshold so
            # the curling tail stays unbroken at icon sizes.
            u = (tx + 0.5) / target_w
            v_norm = (ty + 0.5) / target_h
            is_tail_zone = (v_norm > 0.56 and u < 0.24) or (v_norm > 0.84 and u < 0.44)
            thresh = 4 if is_tail_zone else 7
            if hits >= thresh:
                mask[ty][tx] = True
                vals[ty][tx] = s_val / hits
                mins[ty][tx] = lo
                maxs[ty][tx] = hi

    # Classify each foreground pixel using the title screen's own ear/tail rules
    # (`src/render/menu.rs` lines 410-417) plus value cues from `assets/rat.png`.
    eye_candidates: list[tuple[int, int, int]] = []
    for ty in range(target_h):
        v_norm = (ty + 0.5) / target_h
        base_coat = mix(COAT_TOP, COAT_BOT, v_norm)
        for tx in range(target_w):
            if not mask[ty][tx]:
                continue
            u = (tx + 0.5) / target_w
            lum = vals[ty][tx]
            lo = mins[ty][tx]

            # Inner ear region (dark ~70 oval inside ear).
            in_ear_oval = ((u - 0.725) / 0.085) ** 2 + ((v_norm - 0.21) / 0.16) ** 2 < 1.0
            # Curling tail on left & lower-left (lum ~145-160 in rat.png).
            in_tail = (
                (u < 0.22 and v_norm > 0.52 and lum < 172)
                or (u < 0.42 and v_norm > 0.87 and lum < 175)
                or (u < 0.14 and v_norm > 0.58)
            )
            # Nose tip on far right.
            in_nose = u > 0.94 and 0.46 <= v_norm <= 0.68 and lo < 95
            # Eye region.
            in_eye_zone = 0.76 <= u <= 0.86 and 0.34 <= v_norm <= 0.54 and lo < 85

            if in_eye_zone:
                eye_candidates.append((lo, tx, ty))
                grid[ty][tx] = EYE
            elif in_nose:
                grid[ty][tx] = NOSE
            elif in_ear_oval and lum < 130:
                grid[ty][tx] = EAR_INNER
            elif in_ear_oval and lum < 195:
                grid[ty][tx] = TAIL
            elif in_tail:
                # Subtle top/bottom shading along the curling tail.
                grid[ty][tx] = TAIL_SHADE if v_norm > 0.86 else TAIL
            elif lum > 234:
                # Bright outer ear rim & top crest.
                grid[ty][tx] = COAT_HI
            elif lum < 185 and v_norm > 0.72:
                # Shaded lower paws & belly.
                grid[ty][tx] = PAW_SHADE
            else:
                grid[ty][tx] = base_coat

    # Ensure the eye is a crisp, readable 2x2 (or 1x1) dark pixel with a
    # subtle upper-left coat glint if room permits.
    if eye_candidates:
        eye_candidates.sort()
        _, ex, ey = eye_candidates[0]
        grid[ey][ex] = EYE
        if ey + 1 < target_h and mask[ey + 1][ex]:
            grid[ey + 1][ex] = EYE
        if ex + 1 < target_w and mask[ey][ex + 1]:
            grid[ey][ex + 1] = EYE

    # Ensure the nose tip has a crisp pink pixel on the snout.
    nose_y = int(round(target_h * 0.56))
    for tx in range(target_w - 1, max(0, target_w - 4), -1):
        if mask[nose_y][tx]:
            grid[nose_y][tx] = NOSE
            if nose_y + 1 < target_h and mask[nose_y + 1][tx]:
                grid[nose_y + 1][tx] = NOSE_DARK
            break

    return grid


def render_icon(cropped: Image.Image, *, maskable: bool) -> Image.Image:
    """Render the 64x64 master pixel-art icon."""
    img = Image.new("RGB", (GRID, GRID), BG)
    px = img.load()

    if maskable:
        # Full-bleed subtle recessed facility plate so any OS adaptive mask
        # (circle, squircle, rounded square) reveals a clean dark plate with
        # zero hard edges or cut-off borders.
        for y in range(GRID):
            for x in range(GRID):
                # Subtle scanline-free radial vignette toward PLATE in the center.
                dx = (x + 0.5 - GRID / 2) / (GRID / 2)
                dy = (y + 0.5 - GRID / 2) / (GRID / 2)
                d = math.hypot(dx, dy)
                t = max(0.0, min(1.0, 1.0 - d * 0.85))
                px[x, y] = mix(BG, PLATE, t * 0.85)

        # Entire rat + registration brackets sit strictly inside the 80% safe
        # circle (radius <= 23.0 from center (32, 32), vs 80% safe r = 25.6).
        rat = rasterize_rat(cropped, target_w=36)
        br_l, br_r = 13, 50
        br_t, br_b = 18, 45
        br_arm = 4
    else:
        # Standard icon: crisp dark steel specimen plate inset on `#090a0d`.
        pl_l, pl_r = 3, 60
        pl_t, pl_b = 3, 60
        for y in range(pl_t, pl_b + 1):
            for x in range(pl_l, pl_r + 1):
                if y == pl_t or x == pl_l:
                    px[x, y] = PLATE_HI
                elif y == pl_b or x == pl_r:
                    px[x, y] = PLATE_LO
                else:
                    px[x, y] = PLATE
        # Subtle 1px dark corner notch on the plate outer corners for clean pixel chamfer.
        for cx, cy in [(pl_l, pl_t), (pl_r, pl_t), (pl_l, pl_b), (pl_r, pl_b)]:
            px[cx, cy] = BG

        rat = rasterize_rat(cropped, target_w=48)
        br_l, br_r = 6, 57
        br_t, br_b = 13, 50
        br_arm = 5

    # Facility registration marks (`┌ ┐ └ ┘` in `palette::EDGE`) framing the rat,
    # matching the title screen (`src/render/menu.rs` lines 397-402).
    for i in range(br_arm):
        # Top-left (lit)
        px[br_l + i, br_t] = EDGE_HI
        px[br_l, br_t + i] = EDGE_HI
        # Top-right
        px[br_r - i, br_t] = EDGE
        px[br_r, br_t + i] = EDGE
        # Bottom-left
        px[br_l + i, br_b] = EDGE
        px[br_l, br_b - i] = EDGE
        # Bottom-right
        px[br_r - i, br_b] = EDGE
        px[br_r, br_b - i] = EDGE

    # Tiny tail-pink specimen index pip inside top-left registration mark.
    px[br_l + 2, br_t + 2] = TAIL

    # Place rat centered inside the registration frame.
    rh = len(rat)
    rw = len(rat[0])
    ox = (GRID - rw) // 2
    oy = (GRID - rh) // 2 + 1

    # Soft grounded shadow directly beneath the rat's lowest pixels.
    for sy in range(rh):
        for sx in range(rw):
            if rat[sy][sx] is not None:
                gy = oy + sy + 1
                gx = ox + sx + 1
                if 0 <= gx < GRID and 0 <= gy < GRID:
                    px[gx, gy] = SHADOW

    # Crisp 1-px dark pixel-art outline around the rat silhouette.
    for sy in range(rh):
        for sx in range(rw):
            if rat[sy][sx] is None:
                continue
            for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                nx, ny = sx + dx, sy + dy
                if not (0 <= nx < rw and 0 <= ny < rh and rat[ny][nx] is not None):
                    gx, gy = ox + nx, oy + ny
                    if 0 <= gx < GRID and 0 <= gy < GRID:
                        px[gx, gy] = OUTLINE

    # Stamp the pixel-art rat sprite.
    for sy in range(rh):
        for sx in range(rw):
            col = rat[sy][sx]
            if col is not None:
                px[ox + sx, oy + sy] = col

    # Safety assertion for maskable icon: every rat & bracket pixel must sit
    # inside the 80% safe circle (diameter = 0.80 * GRID, radius = 0.40 * GRID).
    if maskable:
        safe_r = 0.40 * GRID
        cx = GRID / 2.0
        cy = GRID / 2.0
        for sy in range(rh):
            for sx in range(rw):
                if rat[sy][sx] is not None:
                    d = math.hypot((ox + sx + 0.5) - cx, (oy + sy + 0.5) - cy)
                    assert d <= safe_r, f"maskable rat pixel ({ox+sx},{oy+sy}) outside 80% safe circle ({d:.2f} > {safe_r:.2f})"
        for bx, by in [(br_l, br_t), (br_r, br_t), (br_l, br_b), (br_r, br_b)]:
            d = math.hypot((bx + 0.5) - cx, (by + 0.5) - cy)
            assert d <= safe_r, f"maskable bracket ({bx},{by}) outside 80% safe circle ({d:.2f} > {safe_r:.2f})"

    return img


def main() -> None:
    out_dir = Path(__file__).resolve().parent
    repo_root = out_dir.parent.parent
    cropped = load_rat_cropped(repo_root)

    standard = render_icon(cropped, maskable=False)
    maskable = render_icon(cropped, maskable=True)

    standard.resize((192, 192), Image.Resampling.NEAREST).save(out_dir / "icon-192.png")
    standard.resize((512, 512), Image.Resampling.NEAREST).save(out_dir / "icon-512.png")
    maskable.resize((512, 512), Image.Resampling.NEAREST).save(out_dir / "maskable-512.png")
    print("Wrote icon-192.png, icon-512.png, maskable-512.png")


if __name__ == "__main__":
    main()
