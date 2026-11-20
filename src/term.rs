//! Terminal capability detection — one place that decides what this terminal
//! can be trusted with, and the fallbacks every other module asks for.
//!
//! The game is developed against Ghostty, which supports everything: 24-bit
//! colour, the Kitty keyboard protocol, and every glyph we draw. Other
//! terminals are less generous, so this module answers three questions once at
//! startup and exposes the answers as cheap global lookups:
//!
//! * [`color_depth`] — 24-bit, 256-colour or 16-colour. When it is not 24-bit,
//!   [`map_rgb`] folds every RGB the game emits into the xterm palette
//!   (6×6×6 cube plus the grey ramp, nearest by a perceptual distance, cached).
//! * [`glyph_set`] — [`GlyphSet::Unicode`] (the default art) or
//!   [`GlyphSet::Safe`] (box drawing, block elements and ASCII only).
//!   [`sanitize`] rewrites a finished frame's cells into the safe set.
//! * [`is_multiplexer`] — tmux/screen, reported in the support table and used
//!   only for diagnostics; nothing changes behaviour on it.
//!
//! Everything is overridable: `RATRUN_COLOR=truecolor|256|16` and
//! `RATRUN_GLYPHS=unicode|safe` in the environment, or `--color` / `--glyphs`
//! on the command line. `--caps` prints the resolved answer and exits.
//!
//! The startup keyboard query lives here too: crossterm's own
//! `supports_keyboard_enhancement` waits a full 2 s for an answer, which a
//! terminal that never replies (or one that only answers primary device
//! attributes) charges to startup. [`query_keyboard_enhancement`] sends the
//! same two queries and gives up after [`KITTY_QUERY_TIMEOUT`].

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::render::palette::Rgb;

// ---------------------------------------------------------------------------
// Colour depth and glyph set
// ---------------------------------------------------------------------------

/// How much colour the terminal is trusted with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorDepth {
    /// 24-bit SGR (`38;2;r;g;b`). Ghostty, Kitty, WezTerm, iTerm2, most modern
    /// terminals.
    Truecolor,
    /// The xterm 256-colour palette (`38;5;n`).
    Ansi256,
    /// The 16 ANSI colours (`30`–`37`, `90`–`97`). Last resort.
    Ansi16,
}

impl ColorDepth {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorDepth::Truecolor => "truecolor",
            ColorDepth::Ansi256 => "256",
            ColorDepth::Ansi16 => "16",
        }
    }

    /// Parse an override value. Accepts the names a user is likely to type.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "24-bit" | "rgb" | "tc" => Some(ColorDepth::Truecolor),
            "256" | "256color" | "256-colour" | "256-color" | "ansi256" => Some(ColorDepth::Ansi256),
            "16" | "16color" | "16-colour" | "16-color" | "ansi" | "ansi16" => {
                Some(ColorDepth::Ansi16)
            }
            _ => None,
        }
    }

    #[inline]
    fn code(self) -> u8 {
        match self {
            ColorDepth::Truecolor => 1,
            ColorDepth::Ansi256 => 2,
            ColorDepth::Ansi16 => 3,
        }
    }
}

/// Which glyph inventory the frame is drawn from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlyphSet {
    /// The shipped art: geometric shapes, arrows, guillemets and the
    /// typographic dash. Every character here is single-width in a UTF-8
    /// locale and present in Menlo, which is what the game is drawn for.
    Unicode,
    /// Box drawing, block elements and ASCII only. Selected when the terminal
    /// is likely to mis-size ambiguous-width glyphs (a CJK locale, or the Linux
    /// console font) or when the user asks for it.
    Safe,
}

impl GlyphSet {
    pub fn as_str(self) -> &'static str {
        match self {
            GlyphSet::Unicode => "unicode",
            GlyphSet::Safe => "safe",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "unicode" | "wide" | "full" | "default" => Some(GlyphSet::Unicode),
            "safe" | "ascii" | "narrow" | "plain" => Some(GlyphSet::Safe),
            _ => None,
        }
    }

    #[inline]
    fn code(self) -> u8 {
        match self {
            GlyphSet::Unicode => 1,
            GlyphSet::Safe => 2,
        }
    }
}

/// Which rule produced the colour answer (for the support table and `--caps`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorSource {
    CommandLine,
    Environment,
    Colorterm,
    TermSuffix,
    TermProgram,
    Default,
}

impl ColorSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorSource::CommandLine => "flag",
            ColorSource::Environment => "RATRUN_COLOR",
            ColorSource::Colorterm => "COLORTERM",
            ColorSource::TermSuffix => "TERM",
            ColorSource::TermProgram => "TERM_PROGRAM",
            ColorSource::Default => "default",
        }
    }
}

/// Which rule produced the glyph answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlyphSource {
    CommandLine,
    Environment,
    Locale,
    TermSuffix,
    TermProgram,
    Default,
}

impl GlyphSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GlyphSource::CommandLine => "flag",
            GlyphSource::Environment => "RATRUN_GLYPHS",
            GlyphSource::Locale => "locale",
            GlyphSource::TermSuffix => "TERM",
            GlyphSource::TermProgram => "TERM_PROGRAM",
            GlyphSource::Default => "default",
        }
    }
}

/// The resolved capability set for this run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub color: ColorDepth,
    pub color_source: ColorSource,
    pub glyphs: GlyphSet,
    pub glyph_source: GlyphSource,
    pub multiplexer: bool,
    /// `TERM_PROGRAM` as reported by the terminal (empty when unset).
    pub term_program: String,
    /// `TERM` as reported by the terminal (empty when unset).
    pub term: String,
}

impl Capabilities {
    /// One line for `--caps` and the support table.
    pub fn describe(&self) -> String {
        format!(
            "color={} ({}) glyphs={} ({}) multiplexer={} TERM={} TERM_PROGRAM={}",
            self.color.as_str(),
            self.color_source.as_str(),
            self.glyphs.as_str(),
            self.glyph_source.as_str(),
            if self.multiplexer { "yes" } else { "no" },
            if self.term.is_empty() { "-" } else { &self.term },
            if self.term_program.is_empty() {
                "-"
            } else {
                &self.term_program
            },
        )
    }
}

/// Terminal programs that emit 24-bit colour.
const TRUECOLOR_PROGRAMS: &[&str] = &[
    "ghostty",
    "iTerm.app",
    "WezTerm",
    "wezterm",
    "vscode",
    "WarpTerminal",
    "Tabby",
    "Hyper",
    "mintty",
    "rio",
    "contour",
    "kitty",
    "Alacritty",
    "MacTerm",
    "Termius",
    "zed",
];

/// Terminal programs that are 256-colour only. Apple's Terminal.app parses
/// `38;2;…` as a 256-colour sequence and renders the wrong colour, so it must
/// never be handed truecolor.
const ANSI256_PROGRAMS: &[&str] = &["Apple_Terminal"];

/// `TERM` values that mean "8/16 colours".
const LOW_COLOR_TERMS: &[&str] = &["dumb", "vt100", "vt220", "ansi", "linux", "screen", "tmux"];

/// CJK language tags. In these locales Unicode's "ambiguous" East Asian Width
/// class is drawn double-width, which tears every box, bar and arrow apart.
const CJK_LANGUAGES: &[&str] = &["ja", "ko", "zh", "wja", "jpn", "kor", "zho"];

/// Terminal programs whose default font cannot draw the game's geometric
/// glyphs, so the safe set is chosen for them.
///
/// Measured on this machine (see `TERMINALS.md`): Apple's Terminal.app ships
/// `SFMonoTerminal-Regular`, a variant of SF Mono, which covers 21 of the 23
/// non-ASCII characters the shipped UI uses but not `◆ ◇ ▪ ▫ ► ◄ ◂ ▸ ▲ ▼ ◤ ◥ ◢ ◣
/// ✦ ♥ ♦ ▰ ▱ ∈ ⇒`. Those fall back to a proportional font, which is exactly
/// the "looks like garbage" case.
const SAFE_GLYPH_PROGRAMS: &[&str] = &["Apple_Terminal"];

// ---------------------------------------------------------------------------
// Resolved state
// ---------------------------------------------------------------------------

/// 0 = not detected yet. The game's own default (truecolor, unicode) is what
/// every unit test and every `TestBackend` render sees, so the harness never
/// has to know about this module.
const UNRESOLVED: u8 = 0;

static DEPTH: AtomicU8 = AtomicU8::new(UNRESOLVED);
static GLYPHS: AtomicU8 = AtomicU8::new(UNRESOLVED);
static MULTIPLEXER: AtomicU8 = AtomicU8::new(UNRESOLVED);

/// The colour depth to render for. `Truecolor` until [`init`] has run.
#[inline]
pub fn color_depth() -> ColorDepth {
    match DEPTH.load(Ordering::Relaxed) {
        2 => ColorDepth::Ansi256,
        3 => ColorDepth::Ansi16,
        _ => ColorDepth::Truecolor,
    }
}

/// The glyph inventory to render for. `Unicode` until [`init`] has run.
#[inline]
pub fn glyph_set() -> GlyphSet {
    match GLYPHS.load(Ordering::Relaxed) {
        2 => GlyphSet::Safe,
        _ => GlyphSet::Unicode,
    }
}

/// True when the terminal is behind tmux or screen. Diagnostics only.
#[inline]
pub fn is_multiplexer() -> bool {
    MULTIPLEXER.load(Ordering::Relaxed) == 2
}

/// Detect the capabilities and install them as the process-wide answer.
///
/// Called once from `main`, before the first frame. Returns the resolved set so
/// the caller can print it (`--caps`) or log it.
pub fn init(args: &[String]) -> Capabilities {
    let caps = detect(args);
    DEPTH.store(caps.color.code(), Ordering::Relaxed);
    GLYPHS.store(caps.glyphs.code(), Ordering::Relaxed);
    MULTIPLEXER.store(if caps.multiplexer { 2 } else { 1 }, Ordering::Relaxed);
    caps
}

/// Detect without installing anything. Pure: reads the environment and the
/// command line only, so tests can drive it with a fake environment.
pub fn detect(args: &[String]) -> Capabilities {
    let cli = CliOverrides::parse(args);
    let env = |key: &str| std::env::var(key).ok().unwrap_or_default();
    detect_with(&cli, &env)
}

/// Detect against an arbitrary environment lookup. Used by [`detect`] and by
/// the tests.
pub fn detect_with(cli: &CliOverrides, env: &dyn Fn(&str) -> String) -> Capabilities {
    let term = env("TERM");
    let term_program = env("TERM_PROGRAM");
    let colorterm = env("COLORTERM");
    let multiplexer = !env("TMUX").is_empty()
        || !env("STY").is_empty()
        || term.starts_with("screen")
        || term.starts_with("tmux");

    let (color, color_source) = detect_color(
        cli.color,
        &env("RATRUN_COLOR"),
        &term,
        &term_program,
        &colorterm,
    );
    let (glyphs, glyph_source) = detect_glyphs(
        cli.glyphs,
        &env("RATRUN_GLYPHS"),
        &term,
        &term_program,
        &env("LC_ALL"),
        &env("LC_CTYPE"),
        &env("LANG"),
    );

    Capabilities {
        color,
        color_source,
        glyphs,
        glyph_source,
        multiplexer,
        term_program,
        term,
    }
}

/// Command-line overrides, already validated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CliOverrides {
    pub color: Option<ColorDepth>,
    pub glyphs: Option<GlyphSet>,
}

impl CliOverrides {
    /// Pick `--color <v>` / `--glyphs <v>` (and the `=` forms) out of an
    /// argument list. Unknown values are ignored rather than fatal: a typo in
    /// an override should not stop the game from starting.
    pub fn parse(args: &[String]) -> Self {
        let mut out = CliOverrides::default();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (arg, None),
            };
            match name {
                "--color" | "--colour" => {
                    let value = inline.map(str::to_string).or_else(|| {
                        i += 1;
                        args.get(i).cloned()
                    });
                    if let Some(v) = value {
                        if let Some(depth) = ColorDepth::parse(&v) {
                            out.color = Some(depth);
                        }
                    }
                }
                "--glyphs" => {
                    let value = inline.map(str::to_string).or_else(|| {
                        i += 1;
                        args.get(i).cloned()
                    });
                    if let Some(v) = value {
                        if let Some(set) = GlyphSet::parse(&v) {
                            out.glyphs = Some(set);
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out
    }
}

fn detect_color(
    cli: Option<ColorDepth>,
    env_override: &str,
    term: &str,
    term_program: &str,
    colorterm: &str,
) -> (ColorDepth, ColorSource) {
    if let Some(depth) = cli {
        return (depth, ColorSource::CommandLine);
    }
    if let Some(depth) = ColorDepth::parse(env_override) {
        return (depth, ColorSource::Environment);
    }
    // `dumb` never gets colour beyond the 16 base ones, whatever else is set.
    if term == "dumb" || LOW_COLOR_TERMS.contains(&term) {
        return (ColorDepth::Ansi16, ColorSource::TermSuffix);
    }
    let ct = colorterm.to_ascii_lowercase();
    if ct == "truecolor" || ct == "24bit" {
        return (ColorDepth::Truecolor, ColorSource::Colorterm);
    }
    if term.contains("truecolor") || term.contains("direct") {
        return (ColorDepth::Truecolor, ColorSource::TermSuffix);
    }
    if TRUECOLOR_PROGRAMS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(term_program))
    {
        return (ColorDepth::Truecolor, ColorSource::TermProgram);
    }
    if term.contains("256") {
        return (ColorDepth::Ansi256, ColorSource::TermSuffix);
    }
    if ANSI256_PROGRAMS.contains(&term_program) {
        return (ColorDepth::Ansi256, ColorSource::TermProgram);
    }
    // Conservative default: 256 colours are safe on anything that speaks SGR,
    // and 24-bit is the one that can render as garbage.
    (ColorDepth::Ansi256, ColorSource::Default)
}

fn detect_glyphs(
    cli: Option<GlyphSet>,
    env_override: &str,
    term: &str,
    term_program: &str,
    lc_all: &str,
    lc_ctype: &str,
    lang: &str,
) -> (GlyphSet, GlyphSource) {
    if let Some(set) = cli {
        return (set, GlyphSource::CommandLine);
    }
    if let Some(set) = GlyphSet::parse(env_override) {
        return (set, GlyphSource::Environment);
    }
    // The most specific locale wins, as in POSIX.
    let locale = [lc_all, lc_ctype, lang]
        .iter()
        .find(|s| !s.is_empty())
        .copied()
        .unwrap_or("");
    if locale_is_cjk(locale) {
        return (GlyphSet::Safe, GlyphSource::Locale);
    }
    // The Linux console font has no geometric shapes at all, and its width
    // table is the kernel's, not the locale's.
    if term == "linux" {
        return (GlyphSet::Safe, GlyphSource::TermSuffix);
    }
    // A terminal whose own default font cannot draw the art.
    if SAFE_GLYPH_PROGRAMS.iter().any(|p| p.eq_ignore_ascii_case(term_program)) {
        return (GlyphSet::Safe, GlyphSource::TermProgram);
    }
    (GlyphSet::Unicode, GlyphSource::Default)
}

/// True when a locale tag names a CJK language (`ja_JP.UTF-8`, `zh-Hans`,
/// `ko_KR`, ...). East Asian Width "ambiguous" is double-width in these.
fn locale_is_cjk(locale: &str) -> bool {
    let tag = locale.split(['.', '@']).next().unwrap_or("");
    let language = tag
        .split(['_', '-'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    CJK_LANGUAGES.contains(&language.as_str())
}

// ---------------------------------------------------------------------------
// Colour mapping
// ---------------------------------------------------------------------------

/// The 16 xterm base colours, as the ANSI names ratatui renders them.
///
/// These are the *standard* xterm values. Terminals remap them (Solarized,
/// Dracula, …), so a 16-colour frame is a best-effort approximation — but it is
/// far better than emitting `38;2;…` at a terminal that renders it as garbage.
const ANSI16: [Rgb; 16] = [
    (0, 0, 0),       // 0 black
    (128, 0, 0),     // 1 maroon
    (0, 128, 0),     // 2 green
    (128, 128, 0),   // 3 olive
    (0, 0, 128),     // 4 navy
    (128, 0, 128),   // 5 purple
    (0, 128, 128),   // 6 teal
    (192, 192, 192), // 7 silver
    (128, 128, 128), // 8 grey
    (255, 0, 0),     // 9 red
    (0, 255, 0),     // 10 lime
    (255, 255, 0),   // 11 yellow
    (0, 0, 255),     // 12 blue
    (255, 0, 255),   // 13 fuchsia
    (0, 255, 255),   // 14 aqua
    (255, 255, 255), // 15 white
];

/// The ratatui colour that renders as ANSI index `i`.
const ANSI16_NAMES: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

/// The six cube levels used by the xterm 256-colour palette.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// xterm 256-colour palette entries 16..=255: 216 cube colours then 24 greys.
const PALETTE_256: [Rgb; 240] = build_palette_256();

const fn build_palette_256() -> [Rgb; 240] {
    let mut out = [(0u8, 0u8, 0u8); 240];
    let mut i = 0;
    while i < 216 {
        let r = CUBE_LEVELS[i / 36];
        let g = CUBE_LEVELS[(i / 6) % 6];
        let b = CUBE_LEVELS[i % 6];
        out[i] = (r, g, b);
        i += 1;
    }
    let mut g = 0;
    while g < 24 {
        let v = 8 + 10 * g as u8;
        out[216 + g] = (v, v, v);
        g += 1;
    }
    out
}

/// Squared "redmean" distance — a cheap perceptual metric (the weighted RGB
/// approximation from Compuphase's colour-metrics note). Green dominates, and
/// the red/blue weights shift with the mean red, which is what makes the
/// difference between "dark navy" and "black" survive the round trip.
#[inline]
fn redmean(a: Rgb, b: Rgb) -> u32 {
    let rmean = (a.0 as i32 + b.0 as i32) / 2;
    let dr = a.0 as i32 - b.0 as i32;
    let dg = a.1 as i32 - b.1 as i32;
    let db = a.2 as i32 - b.2 as i32;
    let r = (((512 + rmean) * dr * dr) >> 8) as u32;
    let g = (4 * dg * dg) as u32;
    let bl = (((767 - rmean) * db * db) >> 8) as u32;
    r + g + bl
}

/// Index into [`PALETTE_256`] (0..240) of the nearest cube/grey colour.
fn nearest_cube(c: Rgb) -> u8 {
    let mut best = 0usize;
    let mut best_d = u32::MAX;
    for (i, p) in PALETTE_256.iter().enumerate() {
        let d = redmean(c, *p);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    // 16 + i: the first 16 slots are the terminal's own base colours.
    16 + best as u8
}

/// Index 0..16 of the nearest base ANSI colour.
fn nearest_base(c: Rgb) -> u8 {
    let mut best = 0usize;
    let mut best_d = u32::MAX;
    for (i, p) in ANSI16.iter().enumerate() {
        let d = redmean(c, *p);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best as u8
}

/// Direct-mapped cache from packed RGB to palette index. 4096 entries covers
/// every colour the game draws several times over, so the per-frame cost is a
/// load and a compare per pixel. Atomics only because the table is a static;
/// the game renders on one thread.
const CACHE_BITS: usize = 12;
const CACHE_LEN: usize = 1 << CACHE_BITS;
static CACHE_TAG: [AtomicU32; CACHE_LEN] = [const { AtomicU32::new(0) }; CACHE_LEN];
static CACHE_VAL: [AtomicU8; CACHE_LEN] = [const { AtomicU8::new(0) }; CACHE_LEN];

#[inline]
fn cache_slot(key: u32) -> usize {
    (key.wrapping_mul(0x9E37_79B1) >> (32 - CACHE_BITS)) as usize
}

/// Map an RGB to the palette index for `depth`, through the cache.
#[inline]
fn cached_index(depth: ColorDepth, c: Rgb) -> u8 {
    let key = ((depth.code() as u32) << 24)
        | ((c.0 as u32) << 16)
        | ((c.1 as u32) << 8)
        | c.2 as u32;
    let tag = key | 0x0100_0000; // never 0: 0 means "empty slot"
    let slot = cache_slot(key);
    if CACHE_TAG[slot].load(Ordering::Relaxed) == tag {
        return CACHE_VAL[slot].load(Ordering::Relaxed);
    }
    let value = match depth {
        ColorDepth::Ansi16 => nearest_base(c),
        _ => nearest_cube(c),
    };
    CACHE_VAL[slot].store(value, Ordering::Relaxed);
    CACHE_TAG[slot].store(tag, Ordering::Relaxed);
    value
}

/// Fold one RGB into the terminal's palette. The hot path: called once per
/// pixel per frame by the half-block renderer.
#[inline]
pub fn map_rgb_with(depth: ColorDepth, c: Rgb) -> Color {
    match depth {
        ColorDepth::Truecolor => Color::Rgb(c.0, c.1, c.2),
        ColorDepth::Ansi256 => Color::Indexed(cached_index(depth, c)),
        ColorDepth::Ansi16 => ANSI16_NAMES[cached_index(depth, c) as usize],
    }
}

/// Fold one RGB into the resolved palette for this run.
#[inline]
pub fn map_rgb(c: Rgb) -> Color {
    map_rgb_with(color_depth(), c)
}

// ---------------------------------------------------------------------------
// Glyph substitution
// ---------------------------------------------------------------------------

/// The replacement for a risky glyph in the safe set, or `None` when the
/// glyph is already safe (box drawing, block elements, ASCII).
///
/// Every replacement is exactly one cell wide, so a sanitised frame keeps the
/// same layout — the design goal is "no redesign, just safer ink".
fn safe_symbol(symbol: &str) -> Option<&'static str> {
    // The one emoji the source ever had: dagger + VARIATION SELECTOR-16.
    // Matched as a whole because it is two codepoints.
    if symbol.starts_with('\u{1F5E1}') {
        return Some("/");
    }
    let first = symbol.chars().next()?;
    Some(match first {
        // Box drawing and block elements are part of the safe set: the frames,
        // the bars and the pixel half-block are all still drawn from them.
        '\u{2500}'..='\u{257F}' | '\u{2580}'..='\u{259F}' => return None,
        // Typographic punctuation.
        '\u{2014}' | '\u{2013}' | '\u{2212}' => "-",
        '\u{00D7}' => "x",
        '\u{00B7}' | '\u{2022}' | '\u{2026}' => ".",
        '\u{00B0}' => "o",
        '\u{00B5}' => "u",
        '\u{00A7}' => "S",
        '\u{00F7}' => "/",
        '\u{00B1}' => "+",
        '\u{00E9}' => "e",
        '\u{2248}' => "~",
        '\u{2264}' => "<",
        '\u{2265}' => ">",
        '\u{2208}' => "e",
        '\u{03C4}' => "t",
        '\u{00B2}' => "2",
        '\u{00AB}' | '\u{2039}' | '\u{2190}' | '\u{2194}' => "<",
        '\u{00BB}' | '\u{203A}' | '\u{2192}' | '\u{21D2}' => ">",
        '\u{2191}' => "^",
        '\u{2193}' => "v",
        // Arrows and triangles.
        '\u{25B2}' | '\u{25B4}' => "^",
        '\u{25BC}' | '\u{25BE}' => "v",
        '\u{25C4}' | '\u{25C2}' => "<",
        '\u{25BA}' | '\u{25B8}' => ">",
        // Quadrant blocks used as 8-way compass pointers: the two on each
        // diagonal read as that diagonal in ASCII.
        '\u{25E4}' | '\u{25E2}' => "\\",
        '\u{25E5}' | '\u{25E3}' => "/",
        // Dingbats, diamonds and stars.
        '\u{25C6}' | '\u{25C8}' | '\u{2666}' | '\u{2726}' | '\u{2605}' => "*",
        '\u{25C7}' | '\u{25CB}' | '\u{25CF}' | '\u{25CE}' | '\u{2665}' => "o",
        '\u{25AA}' | '\u{25AB}' | '\u{25B1}' => ".",
        '\u{25B0}' => "=",
        '\u{2713}' => "v",
        _ => return None,
    })
}

/// Rewrite a finished frame into the resolved glyph set and colour depth.
///
/// Called once per frame, after the app has drawn and before the backend
/// diffs. In the default (truecolor, unicode) configuration this is a single
/// branch and an immediate return; otherwise it walks the cells once, which at
/// 120×40 is 4 800 cells and well under a millisecond.
///
/// Only the *output* is touched: layouts, hit boxes and the app's own state
/// still hold the original glyphs, and every replacement is one cell wide so
/// nothing moves.
pub fn sanitize(buf: &mut Buffer) {
    sanitize_with(buf, glyph_set(), color_depth());
}

/// [`sanitize`] against an explicit capability set, so the tests can drive the
/// fallbacks without mutating the process-wide answer.
pub fn sanitize_with(buf: &mut Buffer, glyphs: GlyphSet, depth: ColorDepth) {
    if glyphs == GlyphSet::Unicode && depth == ColorDepth::Truecolor {
        return;
    }
    for cell in buf.content.iter_mut() {
        if glyphs == GlyphSet::Safe {
            if let Some(replacement) = safe_symbol(cell.symbol()) {
                cell.set_symbol(replacement);
            }
        }
        if depth != ColorDepth::Truecolor {
            if let Color::Rgb(r, g, b) = cell.fg {
                cell.fg = map_rgb_with(depth, (r, g, b));
            }
            if let Color::Rgb(r, g, b) = cell.bg {
                cell.bg = map_rgb_with(depth, (r, g, b));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The startup keyboard query
// ---------------------------------------------------------------------------

/// How long the keyboard-protocol query waits for an answer.
///
/// A terminal that answers at all does so in a millisecond or two; 300 ms is
/// the give-up budget for one that never answers (crossterm's own query waits
/// 2 s, which is charged to every startup on a terminal that stays silent).
pub const KITTY_QUERY_TIMEOUT: Duration = Duration::from_millis(300);

/// Ask the terminal whether it speaks the Kitty keyboard protocol.
///
/// Sends the protocol's own query (`CSI ? u`) plus primary device attributes
/// (`CSI c`) — the recommended pair, because a terminal that supports neither
/// still answers DA1, and DA1 alone proves "not supported". Stops as soon as
/// either answer arrives, so a supporting terminal costs one round trip and a
/// silent one costs [`KITTY_QUERY_TIMEOUT`].
///
/// No environment sniffing: tmux and ssh forward the query, so asking is always
/// correct and never needs to be skipped.
pub fn query_keyboard_enhancement() -> bool {
    query_keyboard().supported()
}

/// Everything one keyboard-protocol query learned, for `--probe` and the
/// support table.
#[derive(Clone, Debug, Default)]
pub struct KeyboardProbe {
    /// `Some(flags)` when the protocol query was answered (`0` means the
    /// terminal replied but supports no flags). `None` when only DA1 came back
    /// or nothing did.
    pub flags: Option<u32>,
    /// The bytes the terminal sent back, verbatim.
    pub raw: Vec<u8>,
}

impl KeyboardProbe {
    pub fn supported(&self) -> bool {
        // An answer to the protocol query means the protocol is supported, even
        // when the flags are 0: that is what kitty reports when no flags are
        // currently pushed. A DA1-only reply leaves `flags` as `None`.
        self.flags.is_some()
    }
}

/// Ask the terminal and return the whole answer, not just the verdict.
pub fn query_keyboard() -> KeyboardProbe {
    query_reply(KITTY_QUERY_TIMEOUT)
}

/// Enter raw mode for the `--probe` diagnostic. Failure is ignored: the probe
/// only needs the terminal to stop echoing, and a terminal that cannot enter
/// raw mode still answers the query (or times out) the same way.
pub fn enable_raw_mode_for_probe() {
    let _ = crossterm::terminal::enable_raw_mode();
}

/// Leave raw mode after the `--probe` diagnostic.
pub fn disable_raw_mode_for_probe() {
    let _ = crossterm::terminal::disable_raw_mode();
}

/// How long to keep waiting for a protocol answer after a DA1-only reply.
///
/// A terminal that speaks the protocol answers the two queries together, so a
/// DA1 reply that is not followed by a protocol answer within a few dozen
/// milliseconds means "not supported". This bounds the *answered* case to one
/// round trip while still letting a protocol answer that arrived second win.
const DA1_GRACE: Duration = Duration::from_millis(60);

/// Ask the terminal for its keyboard protocol. Reads the reply into a
/// [`KeyboardProbe`].
#[cfg(unix)]
fn query_reply(timeout: Duration) -> KeyboardProbe {
    use std::io::Write;

    let mut probe = KeyboardProbe::default();
    let mut out = std::io::stdout();
    // ESC [ ? u  — progressive keyboard enhancement flags
    // ESC [ c    — primary device attributes
    if out.write_all(b"\x1b[?u\x1b[c").is_err() || out.flush().is_err() {
        return probe;
    }

    let deadline = std::time::Instant::now() + timeout;
    let mut seen = [0u8; 128];
    let mut len = 0usize;
    let mut da1_at: Option<std::time::Instant> = None;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        if let Some(at) = da1_at {
            if now.duration_since(at) >= DA1_GRACE {
                break;
            }
        }
        if !stdin_readable(deadline - now) {
            break;
        }
        let want = seen.len() - len;
        if want == 0 {
            break;
        }
        let n = stdin_read(&mut seen[len..]);
        if n == 0 {
            break;
        }
        len += n;
        match scan_reply(&seen[..len]) {
            Reply::Protocol(flags) => {
                probe.flags = Some(flags);
                break;
            }
            Reply::Da1 => {
                // Keep listening: a protocol answer may still be in flight.
                da1_at.get_or_insert(std::time::Instant::now());
            }
            Reply::None => {}
        }
    }
    probe.raw = seen[..len].to_vec();
    probe
}

#[cfg(not(unix))]
fn query_reply(_timeout: Duration) -> KeyboardProbe {
    // crossterm's Windows implementation answers immediately (it never queries
    // the console), so there is no timeout to bound here.
    let supported = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    KeyboardProbe {
        flags: if supported { Some(1) } else { None },
        raw: Vec::new(),
    }
}

/// What the bytes read so far contain.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    /// A protocol answer carrying these flags.
    Protocol(u32),
    /// A primary-device-attributes answer and no protocol answer yet.
    Da1,
    /// Nothing conclusive yet.
    None,
}

/// Scan the bytes read so far for a protocol answer.
///
/// A protocol answer wins over DA1 even when DA1 arrived first: terminals send
/// both replies to the pair of queries, and the order is not specified.
fn scan_reply(bytes: &[u8]) -> Reply {
    let mut da1 = false;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' && bytes[i + 2] == b'?' {
            let mut j = i + 3;
            let mut value = 0u32;
            let mut any = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                value = value.saturating_mul(10).saturating_add((bytes[j] - b'0') as u32);
                any = true;
                j += 1;
            }
            if j < bytes.len() {
                match bytes[j] {
                    b'u' => return Reply::Protocol(value),
                    // DA1 also carries `;`-separated parameters.
                    b';' | b'c' => {
                        da1 = true;
                        let mut k = j;
                        while k < bytes.len() && (bytes[k].is_ascii_digit() || bytes[k] == b';') {
                            k += 1;
                        }
                        if k < bytes.len() && bytes[k] == b'c' {
                            i = k + 1;
                            continue;
                        }
                        i = k;
                        continue;
                    }
                    _ => {}
                }
            }
            if !any {
                i += 1;
                continue;
            }
        }
        i += 1;
    }
    if da1 {
        Reply::Da1
    } else {
        Reply::None
    }
}

/// Block until stdin has bytes to read, up to `timeout`. Returns false on
/// timeout or error.
#[cfg(unix)]
fn stdin_readable(timeout: Duration) -> bool {
    let mut fd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: `poll` is given one initialised `pollfd` and a matching count.
    let rc = unsafe { libc::poll(&mut fd, 1, ms) };
    rc > 0
}

/// Read whatever is already buffered on stdin. Returns 0 on EOF or error.
#[cfg(unix)]
fn stdin_read(buf: &mut [u8]) -> usize {
    // SAFETY: `buf` is a valid writable slice of `buf.len()` bytes.
    let n = unsafe { libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n <= 0 {
        0
    } else {
        n as usize
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> String {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| {
            owned
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        }
    }

    fn caps(pairs: &[(&str, &str)], cli: CliOverrides) -> Capabilities {
        detect_with(&cli, &env_of(pairs))
    }

    #[test]
    fn ghostty_is_truecolor_and_unicode() {
        let c = caps(
            &[
                ("TERM", "xterm-ghostty"),
                ("TERM_PROGRAM", "ghostty"),
                ("COLORTERM", "truecolor"),
                ("LANG", "en_US.UTF-8"),
            ],
            CliOverrides::default(),
        );
        assert_eq!(c.color, ColorDepth::Truecolor);
        assert_eq!(c.color_source, ColorSource::Colorterm);
        assert_eq!(c.glyphs, GlyphSet::Unicode);
        assert!(!c.multiplexer);
    }

    #[test]
    fn colorterm_wins_over_the_term_program() {
        let c = caps(
            &[
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "Apple_Terminal"),
                ("COLORTERM", "truecolor"),
            ],
            CliOverrides::default(),
        );
        assert_eq!(c.color, ColorDepth::Truecolor);
        assert_eq!(c.color_source, ColorSource::Colorterm);
    }

    #[test]
    fn apple_terminal_never_gets_truecolor() {
        let c = caps(
            &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Apple_Terminal")],
            CliOverrides::default(),
        );
        assert_eq!(c.color, ColorDepth::Ansi256);
        assert_eq!(c.color_source, ColorSource::TermSuffix);
    }

    #[test]
    fn apple_terminal_without_a_256_term_is_still_256() {
        let c = caps(
            &[("TERM", "xterm"), ("TERM_PROGRAM", "Apple_Terminal")],
            CliOverrides::default(),
        );
        assert_eq!(c.color, ColorDepth::Ansi256);
        assert_eq!(c.color_source, ColorSource::TermProgram);
    }

    #[test]
    fn a_bare_term_is_conservatively_256() {
        let c = caps(&[("TERM", "xterm")], CliOverrides::default());
        assert_eq!(c.color, ColorDepth::Ansi256);
        assert_eq!(c.color_source, ColorSource::Default);
    }

    #[test]
    fn dumb_terminals_get_16_colours() {
        let c = caps(&[("TERM", "dumb")], CliOverrides::default());
        assert_eq!(c.color, ColorDepth::Ansi16);
        assert_eq!(c.color_source, ColorSource::TermSuffix);
    }

    #[test]
    fn the_environment_override_beats_detection() {
        let c = caps(
            &[("TERM", "xterm-ghostty"), ("COLORTERM", "truecolor"), ("RATRUN_COLOR", "16")],
            CliOverrides::default(),
        );
        assert_eq!(c.color, ColorDepth::Ansi16);
        assert_eq!(c.color_source, ColorSource::Environment);
    }

    #[test]
    fn the_flag_beats_the_environment() {
        let c = caps(
            &[("RATRUN_COLOR", "16")],
            CliOverrides {
                color: Some(ColorDepth::Truecolor),
                glyphs: None,
            },
        );
        assert_eq!(c.color, ColorDepth::Truecolor);
        assert_eq!(c.color_source, ColorSource::CommandLine);
    }

    #[test]
    fn a_cjk_locale_selects_the_safe_glyphs() {
        for lang in ["ja_JP.UTF-8", "zh_CN.UTF-8", "ko_KR.UTF-8", "zh-Hans-CN"] {
            let c = caps(&[("LANG", lang)], CliOverrides::default());
            assert_eq!(c.glyphs, GlyphSet::Safe, "locale {lang}");
            assert_eq!(c.glyph_source, GlyphSource::Locale);
        }
    }

    #[test]
    fn a_western_locale_keeps_the_unicode_glyphs() {
        for lang in ["en_US.UTF-8", "de_DE.UTF-8", "C.UTF-8", "en_GB.UTF-8"] {
            let c = caps(&[("LANG", lang)], CliOverrides::default());
            assert_eq!(c.glyphs, GlyphSet::Unicode, "locale {lang}");
        }
    }

    #[test]
    fn lc_all_beats_lang() {
        let c = caps(
            &[("LC_ALL", "ja_JP.UTF-8"), ("LANG", "en_US.UTF-8")],
            CliOverrides::default(),
        );
        assert_eq!(c.glyphs, GlyphSet::Safe);
    }

    #[test]
    fn the_linux_console_gets_the_safe_glyphs() {
        let c = caps(&[("TERM", "linux")], CliOverrides::default());
        assert_eq!(c.glyphs, GlyphSet::Safe);
        assert_eq!(c.glyph_source, GlyphSource::TermSuffix);
    }

    #[test]
    fn glyph_overrides_beat_the_locale() {
        let c = caps(
            &[("LANG", "ja_JP.UTF-8"), ("RATRUN_GLYPHS", "unicode")],
            CliOverrides::default(),
        );
        assert_eq!(c.glyphs, GlyphSet::Unicode);
        assert_eq!(c.glyph_source, GlyphSource::Environment);

        let c = caps(
            &[],
            CliOverrides {
                color: None,
                glyphs: Some(GlyphSet::Safe),
            },
        );
        assert_eq!(c.glyphs, GlyphSet::Safe);
        assert_eq!(c.glyph_source, GlyphSource::CommandLine);
    }

    #[test]
    fn multiplexers_are_recognised() {
        assert!(caps(&[("TMUX", "/tmp/tmux-1/default,1,0")], CliOverrides::default()).multiplexer);
        assert!(caps(&[("TERM", "screen-256color")], CliOverrides::default()).multiplexer);
        assert!(caps(&[("STY", "1234.pts-0.host")], CliOverrides::default()).multiplexer);
        assert!(!caps(&[("TERM", "xterm-kitty")], CliOverrides::default()).multiplexer);
    }

    #[test]
    fn cli_overrides_are_parsed_in_both_forms() {
        let a = vec!["--color".to_string(), "256".to_string()];
        assert_eq!(CliOverrides::parse(&a).color, Some(ColorDepth::Ansi256));
        let a = vec!["--color=16".to_string()];
        assert_eq!(CliOverrides::parse(&a).color, Some(ColorDepth::Ansi16));
        let a = vec!["--glyphs".to_string(), "safe".to_string()];
        assert_eq!(CliOverrides::parse(&a).glyphs, Some(GlyphSet::Safe));
        let a = vec!["--glyphs=unicode".to_string()];
        assert_eq!(CliOverrides::parse(&a).glyphs, Some(GlyphSet::Unicode));
        // Unknown values are ignored, not fatal.
        let a = vec!["--color=mauve".to_string()];
        assert_eq!(CliOverrides::parse(&a).color, None);
        // Unrelated flags are untouched.
        let a = vec!["--ai".to_string(), "4".to_string(), "--once".to_string()];
        assert_eq!(CliOverrides::parse(&a), CliOverrides::default());
    }

    #[test]
    fn truecolor_passes_rgb_through() {
        assert_eq!(
            map_rgb_with(ColorDepth::Truecolor, (12, 34, 56)),
            Color::Rgb(12, 34, 56)
        );
    }

    #[test]
    fn the_cube_holds_exact_colours() {
        // 0,0,0 is cube entry 0 -> palette index 16.
        assert_eq!(nearest_cube((0, 0, 0)), 16);
        // 255,255,255 is the last cube entry -> 16 + 215 = 231.
        assert_eq!(nearest_cube((255, 255, 255)), 231);
        // 95,0,0 is cube r=1,g=0,b=0 -> 16 + 36 = 52.
        assert_eq!(nearest_cube((95, 0, 0)), 52);
    }

    #[test]
    fn the_grey_ramp_wins_for_greys() {
        // 128,128,128 is nearest to grey 128 (index 232 + 12 = 244).
        assert_eq!(nearest_cube((128, 128, 128)), 244);
    }

    #[test]
    fn ansi16_keeps_the_base_hues() {
        assert_eq!(nearest_base((0, 0, 0)), 0);
        assert_eq!(nearest_base((255, 255, 255)), 15);
        assert_eq!(nearest_base((255, 0, 0)), 9);
        assert_eq!(nearest_base((0, 0, 255)), 12);
        assert_eq!(nearest_base((0, 128, 0)), 2);
    }

    #[test]
    fn the_256_mapping_renders_as_an_indexed_colour() {
        match map_rgb_with(ColorDepth::Ansi256, (200, 30, 40)) {
            Color::Indexed(i) => assert!(i >= 16, "index {i} must not be a base colour"),
            other => panic!("expected Indexed, got {other:?}"),
        }
    }

    #[test]
    fn the_16_mapping_renders_as_a_named_colour() {
        match map_rgb_with(ColorDepth::Ansi16, (255, 0, 0)) {
            Color::LightRed => {}
            other => panic!("expected LightRed, got {other:?}"),
        }
    }

    #[test]
    fn the_cache_returns_the_same_answer_as_a_cold_lookup() {
        for c in [(0, 0, 0), (255, 255, 255), (17, 200, 63), (200, 30, 40)] {
            let first = cached_index(ColorDepth::Ansi256, c);
            let second = cached_index(ColorDepth::Ansi256, c);
            assert_eq!(first, second);
            assert_eq!(first, nearest_cube(c));
        }
    }

    #[test]
    fn the_safe_set_keeps_box_drawing_and_blocks() {
        for keep in ['─', '│', '┌', '┘', '═', '╭', '╯', '▀', '▄', '█', '▏', '▉', '░', '▐'] {
            assert_eq!(safe_symbol(&keep.to_string()), None, "dropped {keep:?}");
        }
    }

    #[test]
    fn the_safe_set_replaces_the_risky_glyphs() {
        for (risky, safe) in [
            ('◆', "*"),
            ('◇', "o"),
            ('▸', ">"),
            ('◄', "<"),
            ('▲', "^"),
            ('▼', "v"),
            ('×', "x"),
            ('—', "-"),
            ('→', ">"),
            ('←', "<"),
            ('«', "<"),
            ('»', ">"),
            ('★', "*"),
            ('♥', "o"),
            ('▰', "="),
        ] {
            assert_eq!(safe_symbol(&risky.to_string()), Some(safe), "glyph {risky:?}");
        }
    }

    #[test]
    fn the_emoji_with_a_variation_selector_is_replaced() {
        assert_eq!(safe_symbol("\u{1F5E1}\u{FE0F}"), Some("/"));
        assert_eq!(safe_symbol("\u{1F5E1}"), Some("/"));
    }

    #[test]
    fn ascii_is_never_touched() {
        for s in ["a", " ", "#", "1", "~"] {
            assert_eq!(safe_symbol(s), None);
        }
    }

    #[test]
    fn every_replacement_is_one_cell_wide() {
        // Guards the "no redesign" rule: a sanitised frame must keep its layout.
        for c in 0x20u32..0x10000 {
            if let Some(ch) = char::from_u32(c) {
                if let Some(replacement) = safe_symbol(&ch.to_string()) {
                    assert_eq!(
                        replacement.chars().count(),
                        1,
                        "U+{c:04X} {ch:?} -> {replacement:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn scan_reply_reads_the_kitty_answer() {
        assert_eq!(scan_reply(b"\x1b[?1u"), Reply::Protocol(1));
        assert_eq!(scan_reply(b"\x1b[?3u"), Reply::Protocol(3));
        assert_eq!(scan_reply(b"\x1b[?0u"), Reply::Protocol(0));
        // A DA1-only answer means "answered, no protocol".
        assert_eq!(scan_reply(b"\x1b[?1;2c"), Reply::Da1);
        assert_eq!(scan_reply(b"\x1b[?62;c"), Reply::Da1);
        // Nothing conclusive yet.
        assert_eq!(scan_reply(b"\x1b[?"), Reply::None);
        assert_eq!(scan_reply(b"abc"), Reply::None);
        assert_eq!(scan_reply(b""), Reply::None);
    }

    #[test]
    fn a_protocol_answer_wins_when_da1_arrives_first() {
        // Ghostty sends DA1 before the protocol flags in one read; the flags
        // must still be found, or a supporting terminal is misdetected.
        assert_eq!(
            scan_reply(b"\x1b[?62;c\x1b[?1u"),
            Reply::Protocol(1)
        );
        assert_eq!(scan_reply(b"\x1b[?1;2c\x1b[?3u"), Reply::Protocol(3));
    }

    #[test]
    fn scan_reply_handles_a_partial_then_complete_read() {
        let mut bytes = Vec::new();
        for chunk in [b"\x1b[".as_slice(), b"?1".as_slice(), b"u".as_slice()] {
            bytes.extend_from_slice(chunk);
            let found = scan_reply(&bytes);
            if bytes.len() < 5 {
                assert_eq!(found, Reply::None);
            } else {
                assert_eq!(found, Reply::Protocol(1));
            }
        }
    }

    #[test]
    fn the_timeout_is_short_enough_to_be_invisible() {
        assert!(KITTY_QUERY_TIMEOUT <= Duration::from_millis(300));
    }
    #[test]
    fn apple_terminal_gets_the_safe_glyphs() {
        // Measured: SFMonoTerminal-Regular cannot draw the geometric glyphs.
        let c = caps(&[("TERM_PROGRAM", "Apple_Terminal")], CliOverrides::default());
        assert_eq!(c.glyphs, GlyphSet::Safe);
        assert_eq!(c.glyph_source, GlyphSource::TermProgram);
        // ...but an explicit override still wins.
        let c = caps(
            &[("TERM_PROGRAM", "Apple_Terminal")],
            CliOverrides {
                color: None,
                glyphs: Some(GlyphSet::Unicode),
            },
        );
        assert_eq!(c.glyphs, GlyphSet::Unicode);
    }

    #[test]
    fn sanitize_rewrites_glyphs_and_colours_together() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf.set_string(0, 0, "\u{25C6}\u{2580}ab", Color::Rgb(200, 30, 40));
        sanitize_with(&mut buf, GlyphSet::Safe, ColorDepth::Ansi16);
        assert_eq!(buf[(0, 0)].symbol(), "*");
        // The half block used by the pixel renderer is untouched.
        assert_eq!(buf[(1, 0)].symbol(), "\u{2580}");
        assert_eq!(buf[(2, 0)].symbol(), "a");
        assert!(matches!(buf[(0, 0)].fg, Color::Red | Color::LightRed));
    }

    #[test]
    fn sanitize_is_a_no_op_in_the_default_configuration() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf.set_string(0, 0, "\u{25C6}\u{25B8}", Color::Rgb(1, 2, 3));
        sanitize_with(&mut buf, GlyphSet::Unicode, ColorDepth::Truecolor);
        assert_eq!(buf[(0, 0)].symbol(), "\u{25C6}");
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(1, 2, 3));
    }

    /// Every non-ASCII character the shipped UI draws must be either kept
    /// deliberately (box drawing / block elements) or mapped in the safe set.
    /// This is the audit table from `TERMINALS.md`, as a test: a new glyph
    /// added to the UI without a safe replacement fails here.
    #[test]
    fn every_ui_glyph_is_kept_or_replaced() {
        const UI_GLYPHS: &str = "\u{2550}\u{2500}\u{2501}\u{2502}\u{250C}\u{2510}\u{2514}\u{2518}\
             \u{256D}\u{256E}\u{256F}\u{2570}\u{251C}\u{2524}\u{252C}\u{2534}\u{2571}\u{2572}\
             \u{2580}\u{2581}\u{2582}\u{2583}\u{2584}\u{2585}\u{2586}\u{2587}\u{2588}\u{2589}\
             \u{258A}\u{258B}\u{258C}\u{258D}\u{258E}\u{258F}\u{2590}\u{2591}\u{2592}\u{2593}\
             \u{25C6}\u{25C7}\u{25C8}\u{25CB}\u{25CF}\u{25CE}\u{25AA}\u{25AB}\u{25B2}\u{25B4}\
             \u{25BC}\u{25BE}\u{25B0}\u{25B1}\u{25B8}\u{25BA}\u{25C2}\u{25C4}\u{25E2}\u{25E3}\
             \u{25E4}\u{25E5}\u{2605}\u{2665}\u{2666}\u{2726}\u{00D7}\u{00B7}\u{2014}\u{2013}\
             \u{2190}\u{2192}\u{2191}\u{2193}\u{2194}\u{00AB}\u{00BB}\u{2039}\u{203A}\u{2026}\
             \u{00B0}\u{00F7}\u{2022}\u{2713}\u{2192}\u{2726}";
        for ch in UI_GLYPHS.chars() {
            let replacement = safe_symbol(&ch.to_string());
            let kept = matches!(ch, '\u{2500}'..='\u{257F}' | '\u{2580}'..='\u{259F}');
            assert!(
                kept || replacement.is_some(),
                "U+{:04X} {ch:?} has no safe replacement",
                ch as u32
            );
        }
    }

}
