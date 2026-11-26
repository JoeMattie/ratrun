// The game background is an 8×8 coverage image per cell. Text sits on a
// transparent device-resolution canvas so it uses normal font rasterization.
const FONT = 'Menlo, Consolas, "DejaVu Sans Mono", monospace';
const BACKGROUND = 0x090a0d;
const BOLD = 1, DIM = 2, ITALIC = 4, UNDERLINE = 8;
const REVERSED = 64, HIDDEN = 128, STRIKE = 256;

const rgba = rgb => 0xff000000 | (rgb & 255) << 16 | (rgb & 0xff00) | rgb >>> 16;
const css = rgb => "#" + rgb.toString(16).padStart(6, "0");
const decoder = new TextDecoder();

export class CanvasRenderer {
  constructor(picture, text, masks) {
    this.picture = picture;
    this.text = text;
    this.pictureContext = picture.getContext("2d", { alpha: false });
    this.textContext = text.getContext("2d");
    this.rasterContext = document.createElement("canvas").getContext("2d", { alpha: false });
    this.masks = new Map();
    for (let i = 0; i < masks.length; i += 3) {
      this.masks.set(masks[i], [masks[i + 1], masks[i + 2]]);
    }
    this.pixelRatio = 0;
  }

  fit(width, height, ratio) {
    const fontSize = Math.max(10, Math.min(20, Math.floor(width / 76), Math.floor(height / 54)));
    const font = `${fontSize * ratio}px ${FONT}`;
    this.textContext.font = font;
    const size = this.textContext.measureText("Wg");
    const cellWidth = Math.max(1, Math.round(size.width / 2));
    // Font bounds include the whole ascent and descent across the font.
    const cellHeight = Math.ceil(size.fontBoundingBoxAscent + size.fontBoundingBoxDescent);
    const cols = Math.max(2, Math.floor(width * ratio / cellWidth));
    const rows = Math.max(1, Math.floor(height * ratio / cellHeight));
    if (this.cols === cols && this.rows === rows && this.pixelRatio === ratio && this.font === font) return false;
    this.cols = cols;
    this.rows = rows;
    this.pixelRatio = ratio;
    this.cellWidth = cellWidth;
    this.cellHeight = cellHeight;
    this.font = font;
    this.baseline = size.fontBoundingBoxAscent;

    for (const canvas of [this.picture, this.text]) {
      canvas.width = cols * cellWidth;
      canvas.height = rows * cellHeight;
      canvas.style.width = `${canvas.width / ratio}px`;
      canvas.style.height = `${canvas.height / ratio}px`;
    }
    const stride = cols * 8;
    this.rasterContext.canvas.width = stride;
    this.rasterContext.canvas.height = rows * 8;
    this.image = this.rasterContext.createImageData(stride, rows * 8);
    this.pixels = new Uint32Array(this.image.data.buffer);
    this.pixels.fill(rgba(BACKGROUND));
    this.pictureContext.imageSmoothingEnabled = false;
    // Zero means no text was drawn at this cell on the previous frame.
    this.textWidths = new Uint8Array(cols * rows);
    this.fonts = ["", "bold ", "italic ", "italic bold "].map(style => style + font);
    return true;
  }

  paint(memory, pointer, length, textPointer) {
    // A view of linear memory can be reused until Wasm grows it. Rust owns
    // the record and text vectors, and this paint consumes them synchronously.
    if (this.memory !== memory) {
      this.memory = memory;
      this.words = new Uint32Array(memory);
      this.bytes = new Uint8Array(memory);
    }
    const words = this.words;
    const start = pointer / 4, end = start + length;
    const stride = this.image.width, cols = this.cols;
    let left = cols, right = 0, top = this.rows, bottom = 0;
    // Clear old glyphs first, including any part spilling into a second cell.
    for (let i = start; i < end; i += 5) {
      const index = words[i], width = this.textWidths[index];
      if (width) {
        this.textContext.clearRect(index % cols * this.cellWidth,
          Math.floor(index / cols) * this.cellHeight, width * this.cellWidth, this.cellHeight);
        this.textWidths[index] = 0;
      }
    }
    for (let i = start; i < end; i += 5) {
      const index = words[i], col = index % cols, row = Math.floor(index / cols);
      let fg = words[i + 1], bg = words[i + 2];
      const glyph = words[i + 3], style = words[i + 4];
      if (style & REVERSED) [fg, bg] = [bg, fg];
      const mask = style & HIDDEN ? undefined : this.masks.get(glyph);
      let offset = row * 8 * stride + col * 8;
      const foreground = rgba(style & DIM ? ((fg & 0xfefefe) + (bg & 0xfefefe)) >>> 1 : fg);
      const background = rgba(bg);
      // Each checked-in block mask uses eight pixels per row. Half blocks
      // dominate gameplay and take the uniform-row path.
      for (let y = 0; y < 8; y++, offset += stride) {
        const bits = mask ? (mask[y >>> 2] >>> ((y & 3) * 8)) & 255 : 0;
        if (bits === 0 || bits === 255) {
          this.pixels.fill(bits ? foreground : background, offset, offset + 8);
        } else {
          for (let x = 0; x < 8; x++) {
            this.pixels[offset + x] = bits & (1 << x) ? foreground : background;
          }
        }
      }
      if (!(style & HIDDEN) && ((!mask && glyph !== 32) || style & (UNDERLINE | STRIKE))) {
        this.paintText(col, row, index, glyph, Boolean(mask), fg, style, textPointer);
      }
      left = Math.min(left, col);
      right = Math.max(right, col + 1);
      top = Math.min(top, row);
      bottom = Math.max(bottom, row + 1);
    }
    const width = right - left, height = bottom - top;
    this.rasterContext.putImageData(this.image, 0, 0, left * 8, top * 8, width * 8, height * 8);
    this.pictureContext.drawImage(this.rasterContext.canvas,
      left * 8, top * 8, width * 8, height * 8,
      left * this.cellWidth, top * this.cellHeight, width * this.cellWidth, height * this.cellHeight);
  }

  paintText(col, row, index, glyph, block, fg, style, pointer) {
    const ctx = this.textContext;
    const width = Math.min(this.cols - col, style >>> 9 & 255);
    const x = col * this.cellWidth, y = row * this.cellHeight, w = width * this.cellWidth;
    ctx.save();
    ctx.beginPath();
    ctx.rect(x, y, w, this.cellHeight);
    ctx.clip();
    ctx.fillStyle = css(fg);
    ctx.globalAlpha = style & DIM ? 0.5 : 1;
    if (!block && glyph !== 32) {
      const offset = pointer + (glyph & 0x7fffffff);
      const symbol = glyph >= 0x80000000
        ? decoder.decode(this.bytes.subarray(offset, offset + (style >>> 17)))
        : String.fromCodePoint(glyph);
      ctx.font = this.fonts[(style & BOLD ? 1 : 0) | (style & ITALIC ? 2 : 0)];
      ctx.fillText(symbol, x, y + this.baseline, w);
    }
    if (style & UNDERLINE) ctx.fillRect(x, y + this.baseline + 1, w, 1);
    if (style & STRIKE) ctx.fillRect(x, y + Math.floor(this.cellHeight / 2), w, 1);
    ctx.restore();
    this.textWidths[index] = width;
  }
}

