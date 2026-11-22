//! `ratrun --prof-bench …`: a headless frame-cost benchmark (no TTY, no audio).
//!
//! Plays one floor forward with the bot (god mode on, so the run always
//! reaches the moment asked for), then measures a list of configurations and
//! prints one JSON object per configuration on stdout.
//!
//! ```text
//! ratrun --prof-bench --color truecolor --floor sewer --at 720 --seed 1000 \
//!     --mode direct --cfg 120x40:1.0:full:300 --cfg 200x60:2.0:full:300:free
//! ```
//!
//! * `--mode direct` — `AutoRun::step` + `World::draw` + the half-block blit +
//!   a ratatui buffer diff, with the stage accumulators of `crate::prof`.
//! * `--mode app` — the real `App::update` / `App::render` through a
//!   `Terminal<CrosstermBackend<counting sink>>`, i.e. what `main`'s loop does
//!   per frame, including the bytes and `write` calls a frame costs.
//! * `--mode title` / `--mode paused` — the idle screens through the same path.
//! * `--cfg COLSxROWS:ZOOM:TIER:FRAMES[:free]` — tier is `full|hard|coarse|
//!   compact`; `free` lets the cost controller move the tier (and reports the
//!   trace), otherwise the tier is pinned.
//! * `--sample FILE:SECS` — run the **last** config for `SECS + 2` wall seconds
//!   and attach `/usr/bin/sample` to this process for `SECS`.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::app::App;
use crate::autoplay::{AutoOpts, AutoRun, CarryPreset};
use crate::game::bot::Skill;
use crate::game::campaign;
use crate::game::level::Theme;
use crate::math::Vec2;
use crate::prof::{self, Stage, STAGE_NAMES};
use crate::render::framebuffer::PixelBuffer;
use crate::render::lighting::LightingMode;
use crate::scores::ScoreTable;

const DT: f32 = 1.0 / 60.0;
/// `--still`: the rat stands still (no bot) during the measured frames.
static STILL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--out line`: the app modes write straight through the 1 KiB `LineWriter`
/// that `io::stdout()` is (the pre-`FrameOut` path); the default is the real
/// stack, `present::FrameOut` over that `LineWriter`.
static BUFFERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--horde N`: keep at least N live enemies around the rat while measuring.
static HORDE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One measured simulation step: the bot's, or a standing rat's.
fn step(run: &mut AutoRun, frame: usize) {
    let horde = HORDE.load(std::sync::atomic::Ordering::Relaxed);
    if horde > 0 && frame.is_multiple_of(8) {
        let w = run.world_mut();
        let have = w.enemy_count();
        let kinds = crate::game::enemy::EnemyKind::all();
        let centre = w.player.pos;
        for i in have..horde {
            let kind = kinds[(i * 7 + frame) % (kinds.len() - 1)];
            if kind == crate::game::enemy::EnemyKind::Boss {
                continue;
            }
            let a = (i as f32 * 2.399_963) + frame as f32 * 0.37;
            let r = 30.0 + ((i * 37 + frame * 11) % 150) as f32;
            let pos = centre + Vec2::new(a.cos() * r, a.sin() * r * 0.7);
            w.spawn_enemy(kind, pos, 40.0);
        }
    }
    if STILL.load(std::sync::atomic::Ordering::Relaxed) {
        let w = run.world_mut();
        w.update(DT, Vec2::ZERO, false);
        w.sfx.clear();
        w.pending_levelups = 0;
    } else {
        run.step(DT);
    }
}

#[derive(Clone)]
struct Cfg {
    cols: u16,
    rows: u16,
    zoom: f32,
    tier: LightingMode,
    tier_name: String,
    frames: usize,
    free: bool,
}

fn arg<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn parse_cfg(s: &str) -> Cfg {
    let parts: Vec<&str> = s.split(':').collect();
    let (c, r) = parts[0].split_once('x').expect("COLSxROWS");
    let tier_name = parts.get(2).copied().unwrap_or("full").to_string();
    let tier = match tier_name.as_str() {
        "hard" => LightingMode::Hard,
        "coarse" => LightingMode::Coarse,
        "compact" => LightingMode::Compact,
        _ => LightingMode::Full,
    };
    Cfg {
        cols: c.parse().unwrap(),
        rows: r.parse().unwrap(),
        zoom: parts.get(1).and_then(|z| z.parse().ok()).unwrap_or(1.0),
        tier,
        tier_name,
        frames: parts.get(3).and_then(|z| z.parse().ok()).unwrap_or(300),
        free: parts.get(4).is_some_and(|f| *f == "free"),
    }
}

fn opts_for(floor: &str) -> (AutoOpts, Theme) {
    let mk = |level: usize, carry, route| AutoOpts {
        start_level: level,
        campaign: false,
        skill: Skill::Expert,
        carry,
        route,
    };
    match floor {
        "kitchen" => (mk(1, CarryPreset::Typical, None), Theme::Kitchen),
        "lab" => (mk(2, CarryPreset::Typical, None), Theme::Lab),
        "laundry" => (
            mk(1, CarryPreset::Typical, Some(Theme::Laundry)),
            Theme::Laundry,
        ),
        "printworks" => (
            mk(2, CarryPreset::Typical, Some(Theme::Printworks)),
            Theme::Printworks,
        ),
        "flatbed" => (
            mk(campaign::LEVEL_COUNT, CarryPreset::Typical, None),
            Theme::Trailer,
        ),
        _ => (mk(0, CarryPreset::None, None), Theme::Sewer),
    }
}

/// The play field of a `cols × rows` terminal, exactly as `app::game_rows`
/// lays it out: a 3-row (or 1-row) top rail and a 1-row bottom rail.
fn game_rect(cols: u16, rows: u16) -> Rect {
    let top = crate::render::hud::top_height(Rect::new(0, 0, cols, rows));
    Rect::new(0, top, cols, rows.saturating_sub(top + 1).max(1))
}

/// A sink that counts bytes and `write` calls.
#[derive(Default, Clone)]
struct Sink {
    bytes: std::rc::Rc<std::cell::Cell<u64>>,
    writes: std::rc::Rc<std::cell::Cell<u64>>,
}
impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.set(self.bytes.get() + buf.len() as u64);
        self.writes.set(self.writes.get() + 1);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn dump_stages(out: &mut String, frames: usize) {
    out.push_str("\"stage_us\":{");
    let mut first = true;
    for (i, name) in STAGE_NAMES.iter().enumerate() {
        // SAFETY of the cast: `Stage` is `repr(usize)` and dense.
        let stage: Stage = unsafe { std::mem::transmute(i) };
        let ns = prof::ns(stage);
        if name.starts_with('C') && ns == 0 {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "\"{}\":{:.2}",
            name,
            ns as f64 / 1000.0 / frames as f64
        ));
    }
    out.push_str("},\"count\":{");
    let mut first = true;
    for (i, name) in STAGE_NAMES.iter().enumerate() {
        let stage: Stage = unsafe { std::mem::transmute(i) };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "\"{}\":{:.2}",
            name,
            prof::cnt(stage) as f64 / frames as f64
        ));
    }
    out.push('}');
}

fn start_sampler(spec: Option<&str>) -> Option<(std::process::Child, f64)> {
    let (file, secs) = spec?.split_once(':')?;
    let secs_f: f64 = secs.parse().ok()?;
    let child = std::process::Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .arg(secs)
        .arg("1")
        .arg("-file")
        .arg(file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    Some((child, secs_f))
}

pub fn run(args: &[String]) {
    let floor = arg(args, "--floor").unwrap_or("sewer").to_string();
    let at: f32 = arg(args, "--at").and_then(|s| s.parse().ok()).unwrap_or(300.0);
    let seed: u64 = arg(args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1000);
    let mode = arg(args, "--mode").unwrap_or("direct").to_string();
    let god = arg(args, "--god").is_none_or(|g| g != "0");
    let sample = arg(args, "--sample").map(str::to_string);
    STILL.store(args.iter().any(|a| a == "--still"), std::sync::atomic::Ordering::Relaxed);
    HORDE.store(
        arg(args, "--horde").and_then(|s| s.parse().ok()).unwrap_or(0),
        std::sync::atomic::Ordering::Relaxed,
    );
    prof::skip_lighting(args.iter().any(|a| a == "--nolight"));
    BUFFERED.store(arg(args, "--out") != Some("line"), std::sync::atomic::Ordering::Relaxed);
    let cfgs: Vec<Cfg> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--cfg")
        .filter_map(|(i, _)| args.get(i + 1))
        .map(|s| parse_cfg(s))
        .collect();
    let (opts, theme) = opts_for(&floor);

    if mode == "audio" {
        // The music synth alone: how long one second of audio takes to make.
        use rodio::Source;
        for level in 1..=3u32 {
            let mut synth = crate::audio::MusicSynth::new(
                theme,
                std::sync::Arc::new(std::sync::atomic::AtomicU32::new(level)),
            );
            let rate = format!("{:?}", synth.sample_rate());
            let channels = format!("{:?}", synth.channels());
            let n = 44_100usize * 2 * 20;
            let t = Instant::now();
            let mut acc = 0.0f32;
            for _ in 0..n {
                acc += synth.next().unwrap_or(0.0);
            }
            let secs = t.elapsed().as_secs_f64();
            println!(
                "{{\"mode\":\"audio\",\"floor\":\"{floor}\",\"intensity\":{level},\"rate\":\"{rate}\",\"channels\":\"{channels}\",\"samples\":{n},\"secs\":{secs:.4},\"ns_per_sample\":{:.1},\"checksum\":{acc}}}",
                secs * 1e9 / n as f64
            );
        }
        return;
    }
    if mode == "encode" {
        encode_bench();
        return;
    }
    if mode == "title" {
        for (i, cfg) in cfgs.iter().enumerate() {
            let last = i + 1 == cfgs.len();
            bench_app(None, theme, cfg, &floor, at, "title", if last { sample.as_deref() } else { None });
        }
        return;
    }

    // Play forward to the moment asked for, at the first config's viewport
    // (the simulation's spawn ring follows the terminal size).
    let mut run = AutoRun::new(seed, opts);
    run.world_mut().player.god = god;
    if let Some(cfg) = cfgs.first() {
        let g = game_rect(cfg.cols, cfg.rows);
        run.world_mut().viewport = Vec2::new(g.width as f32, g.height as f32 * 2.0);
    }
    let t0 = Instant::now();
    let mut ticks = 0u64;
    while run.world().elapsed < at && !run.finished() {
        run.step(DT);
        ticks += 1;
    }
    eprintln!(
        "[prof] {floor} seed {seed}: reached {:.1}s in {} ticks, {:.2}s wall ({:.1} us/tick incl. bot), finished={} enemies={}",
        run.world().elapsed,
        ticks,
        t0.elapsed().as_secs_f64(),
        t0.elapsed().as_secs_f64() * 1e6 / ticks.max(1) as f64,
        run.finished(),
        run.world().enemy_count(),
    );

    for (i, cfg) in cfgs.iter().enumerate() {
        let last = i + 1 == cfgs.len();
        let sample = if last { sample.as_deref() } else { None };
        match mode.as_str() {
            "app" | "paused" => bench_app(Some(&mut run), theme, cfg, &floor, at, &mode, sample),
            _ => bench_direct(&mut run, cfg, &floor, at, sample),
        }
    }
}

fn configure(run: &mut AutoRun, cfg: &Cfg) -> Rect {
    let g = game_rect(cfg.cols, cfg.rows);
    let w = run.world_mut();
    w.viewport = Vec2::new(g.width as f32, g.height as f32 * 2.0);
    w.set_zoom_instant(cfg.zoom);
    w.lighting = cfg.tier;
    w.lighting_depth = crate::term::color_depth();
    // `RATRUN_SUBCELL` and `RATRUN_SMOOTH` shape a direct run the way they
    // shape the app's.
    w.subcell = crate::render::subcell::resolve(
        crate::render::subcell::Choice::from_env(),
        crate::term::block_set(),
    );
    w.smooth = crate::render::subcell::smooth_from_env();
    w.prof_reset_controller();
    prof::freeze_controller(!cfg.free);
    g
}

fn bench_direct(run: &mut AutoRun, cfg: &Cfg, floor: &str, at: f32, sample: Option<&str>) {
    let g = configure(run, cfg);
    let mut pixels = PixelBuffer::new(g.width as usize, g.height as usize * 2);
    let area = Rect::new(0, 0, cfg.cols, cfg.rows);
    let mut cur = Buffer::empty(area);
    let mut prev = Buffer::empty(area);
    // Warm the render-only caches (smoke, shadow profiles, scratch buffers).
    for f in 0..30 {
        step(run, f * 8);
        run.world_mut().draw(&mut pixels);
    }
    let sampler = start_sampler(sample);
    let wall_limit = sampler.as_ref().map(|(_, s)| Duration::from_secs_f64(s + 2.0));
    let started = Instant::now();
    prof::reset();
    prof::enable(true);
    let (mut step_ns, mut draw_ns, mut blit_ns, mut diff_ns) = (0u64, 0u64, 0u64, 0u64);
    let mut draws: Vec<u64> = Vec::with_capacity(cfg.frames);
    let mut lights: Vec<u64> = Vec::with_capacity(cfg.frames);
    let mut changed_cells = 0u64;
    let mut enemies = 0u64;
    let mut bullets = 0u64;
    let mut tier_hist = [0u32; 4];
    let mut transitions: Vec<(usize, &'static str)> = Vec::new();
    let mut last_tier = run.world().prof_tier();
    let mut frames = 0usize;
    let mut frame_hash = 0xcbf2_9ce4_8422_2325u64;
    loop {
        if let Some(limit) = wall_limit {
            if started.elapsed() >= limit {
                break;
            }
        } else if frames >= cfg.frames {
            break;
        }
        let t = Instant::now();
        step(run, frames);
        let t1 = Instant::now();
        let light_before = prof::ns(Stage::Lighting);
        run.world_mut().draw(&mut pixels);
        let t2 = Instant::now();
        for &(r, g8, b) in pixels.pixels().iter() {
            frame_hash = (frame_hash ^ u64::from(r) ^ (u64::from(g8) << 8) ^ (u64::from(b) << 16))
                .wrapping_mul(0x0000_0100_0000_01B3);
        }
        let t2b = Instant::now();
        match run.world().detail() {
            Some(detail) => detail.render_to(g, &mut cur),
            None => pixels.render_to(g, &mut cur),
        }
        let t3 = Instant::now();
        changed_cells += prev.diff(&cur).len() as u64;
        std::mem::swap(&mut prev, &mut cur);
        let t4 = Instant::now();
        step_ns += (t1 - t).as_nanos() as u64;
        draw_ns += (t2 - t1).as_nanos() as u64;
        blit_ns += (t3 - t2b).as_nanos() as u64;
        diff_ns += (t4 - t3).as_nanos() as u64;
        draws.push((t2 - t1).as_nanos() as u64);
        lights.push(prof::ns(Stage::Lighting) - light_before);
        let tier = run.world().prof_tier();
        tier_hist[LightingMode::LIVE.iter().position(|m| *m == tier).unwrap_or(0)] += 1;
        if tier != last_tier {
            transitions.push((frames, tier_label(tier)));
            last_tier = tier;
        }
        if frames.is_multiple_of(16) {
            enemies += run.world().enemy_count() as u64;
            bullets += run.world().bullet_count() as u64;
        }
        frames += 1;
    }
    prof::enable(false);
    if let Some((mut child, _)) = sampler {
        let _ = child.wait();
    }
    let n = frames.max(1);
    let per = |ns: u64| ns as f64 / 1000.0 / n as f64;
    draws.sort_unstable();
    lights.sort_unstable();
    let px = g.width as usize * g.height as usize * 2;
    let world_px = ((g.width as f32 * cfg.zoom).round() * (g.height as f32 * 2.0 * cfg.zoom).round()) as usize;
    let world_px = if (cfg.zoom - 1.0).abs() < 0.002 { px } else { world_px };
    let mut out = format!(
        "{{\"mode\":\"direct\",\"floor\":\"{floor}\",\"at\":{at},\"elapsed_end\":{:.1},\"finished\":{},\"size\":\"{}x{}\",\"zoom\":{},\"tier\":\"{}\",\"free\":{},\"frames\":{n},\"screen_px\":{px},\"world_px\":{world_px},\"light_budget_us\":{:.0},\"enemies\":{:.0},\"bullets\":{:.0},\"boss\":{},\"step_us\":{:.1},\"draw_us\":{:.1},\"draw_p50\":{:.1},\"draw_p95\":{:.1},\"draw_max\":{:.1},\"light_p50\":{:.1},\"light_p95\":{:.1},\"blit_us\":{:.1},\"diff_us\":{:.1},\"changed_cells\":{:.0},\"tier_hist\":{:?},\"transitions\":{:?},\"frame_hash\":\"{:016x}\",",
        run.world().elapsed,
        run.finished(),
        cfg.cols,
        cfg.rows,
        cfg.zoom,
        cfg.tier_name,
        cfg.free,
        160.0 + world_px as f64 * 0.035,
        enemies as f64 / (n as f64 / 16.0).ceil(),
        bullets as f64 / (n as f64 / 16.0).ceil(),
        run.world().boss_hp().is_some(),
        per(step_ns),
        per(draw_ns),
        pct(&draws, 0.5) as f64 / 1000.0,
        pct(&draws, 0.95) as f64 / 1000.0,
        pct(&draws, 1.0) as f64 / 1000.0,
        pct(&lights, 0.5) as f64 / 1000.0,
        pct(&lights, 0.95) as f64 / 1000.0,
        per(blit_ns),
        per(diff_ns),
        changed_cells as f64 / n as f64,
        tier_hist,
        transitions.iter().take(40).collect::<Vec<_>>(),
        frame_hash,
    );
    dump_stages(&mut out, n);
    out.push('}');
    println!("{out}");
}

fn tier_label(m: LightingMode) -> &'static str {
    match m {
        LightingMode::Full => "full",
        LightingMode::Hard => "hard",
        LightingMode::Coarse => "coarse",
        _ => "compact",
    }
}

/// The real App loop body (`main::run`) against a counting sink.
#[allow(clippy::too_many_arguments)]
fn bench_app(
    mut run: Option<&mut AutoRun>,
    theme: Theme,
    cfg: &Cfg,
    floor: &str,
    at: f32,
    mode: &str,
    sample: Option<&str>,
) {
    let area = Rect::new(0, 0, cfg.cols, cfg.rows);
    let mut app = App::with_options(false, None, ScoreTable::default(), false, Some(7));
    // Two sinks: one behind a 1 KiB `LineWriter` (what `io::stdout()` is), to
    // count the `write` calls a frame really makes, and the byte total.
    let sink = Sink::default();
    let writer: Box<dyn Write> = if BUFFERED.load(std::sync::atomic::Ordering::Relaxed) {
        // The real stack: the frame writer over stdout's 1 KiB `LineWriter`.
        Box::new(crate::present::FrameOut::new(io::LineWriter::with_capacity(
            1024,
            sink.clone(),
        )))
    } else {
        Box::new(io::LineWriter::with_capacity(1024, sink.clone()))
    };
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(writer),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("terminal");
    // One title frame so the app knows its area.
    terminal.draw(|f| app.render(f)).expect("draw");
    if let Some(run) = run.as_deref_mut() {
        configure(run, cfg);
        app.start_floor(theme).expect("start floor");
        app.prof_force_playing();
        // The app's own sim step is made negligible: the bot's `AutoRun` owns
        // the simulation and the world is swapped in for update + render.
        app.timescale = 1e-6;
        app.set_lighting(cfg.tier);
    }
    let swap = |app: &mut App, run: &mut AutoRun| {
        std::mem::swap(app.world_mut().expect("world"), run.world_mut());
    };
    // Exactly what `main::run` does per frame, on a simulated 60 Hz clock (the
    // pacer skips still frames by time, and the bench runs flat out).
    let mut frames = crate::present::FrameLoop::default();
    let clock0 = Instant::now();
    let mut ticks = 0u32;
    let rendered = std::cell::Cell::new(0u64);
    let mut frame = |app: &mut App,
                     run: &mut Option<&mut AutoRun>,
                     terminal: &mut Terminal<CrosstermBackend<Box<dyn Write>>>,
                     acc: &mut [u64; 5],
                     fno: &mut usize,
                     step_world: bool| {
        let t = Instant::now();
        if let (Some(run), true) = (run.as_deref_mut(), step_world) {
            step(run, *fno);
            *fno += 1;
        }
        let t1 = Instant::now();
        let tick_before = prof::ns(Stage::SimTick);
        if let Some(run) = run.as_deref_mut() {
            swap(app, run);
        }
        app.input.begin_frame();
        app.update(DT);
        let t2 = Instant::now();
        let app_tick = prof::ns(Stage::SimTick) - tick_before;
        let before = [
            prof::ns(Stage::AppRender),
            prof::ns(Stage::Sanitize),
            prof::ns(Stage::Present),
        ];
        ticks += 1;
        let now = clock0 + Duration::from_micros(16_667) * ticks;
        if frames.present(terminal, app, now).expect("present").is_some() {
            rendered.set(rendered.get() + 1);
        }
        let t3 = Instant::now();
        if let Some(run) = run.as_deref_mut() {
            swap(app, run);
        }
        let render_ns = prof::ns(Stage::AppRender) - before[0];
        let sanitize_ns = prof::ns(Stage::Sanitize) - before[1];
        let present_ns = prof::ns(Stage::Present) - before[2];
        acc[0] += (t1 - t).as_nanos() as u64; // bot + sim step
        acc[1] += (t2 - t1).as_nanos() as u64 - app_tick; // App::update minus its own (near-zero dt) tick
        acc[2] += render_ns;
        acc[3] += sanitize_ns;
        // Diff + encode + write, plus the loop's own bookkeeping.
        acc[4] += ((t3 - t2).as_nanos() as u64).saturating_sub(render_ns + sanitize_ns).max(present_ns);
    };
    let mut acc = [0u64; 5];
    let mut fno = 0usize;
    for _ in 0..30 {
        frame(&mut app, &mut run, &mut terminal, &mut acc, &mut fno, true);
    }
    if mode == "paused" {
        app.input.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        if let Some(run) = run.as_deref_mut() {
            swap(&mut app, run);
        }
        app.update(DT);
        if let Some(run) = run.as_deref_mut() {
            swap(&mut app, run);
        }
        for _ in 0..5 {
            frame(&mut app, &mut run, &mut terminal, &mut acc, &mut fno, false);
        }
    }
    let screen = app.prof_screen();
    let sampler = start_sampler(sample);
    let wall_limit = sampler.as_ref().map(|(_, s)| Duration::from_secs_f64(s + 2.0));
    let started = Instant::now();
    acc = [0; 5];
    let (bytes0, writes0) = (sink.bytes.get(), sink.writes.get());
    let rendered0 = rendered.get();
    prof::reset();
    prof::enable(true);
    let mut frames = 0usize;
    let mut totals: Vec<u64> = Vec::new();
    loop {
        if let Some(limit) = wall_limit {
            if started.elapsed() >= limit {
                break;
            }
        } else if frames >= cfg.frames {
            break;
        }
        let before: u64 = acc.iter().sum();
        frame(&mut app, &mut run, &mut terminal, &mut acc, &mut fno, mode != "paused");
        totals.push(acc.iter().sum::<u64>() - before);
        frames += 1;
    }
    prof::enable(false);
    if let Some((mut child, _)) = sampler {
        let _ = child.wait();
    }
    drop(terminal);
    let n = frames.max(1);
    let per = |ns: u64| ns as f64 / 1000.0 / n as f64;
    totals.sort_unstable();
    let (enemies, elapsed_end) = run
        .as_deref()
        .map_or((0, 0.0), |r| (r.world().enemy_count(), r.world().elapsed));
    let mut out = format!(
        "{{\"out\":\"{}\",\"mode\":\"{mode}\",\"screen\":\"{screen}\",\"floor\":\"{floor}\",\"at\":{at},\"elapsed_end\":{elapsed_end:.1},\"size\":\"{}x{}\",\"zoom\":{},\"tier\":\"{}\",\"frames\":{n},\"rendered\":{},\"enemies\":{enemies},\"step_us\":{:.1},\"app_update_us\":{:.1},\"render_us\":{:.1},\"sanitize_us\":{:.1},\"present_us\":{:.1},\"frame_p50\":{:.1},\"frame_p95\":{:.1},\"frame_max\":{:.1},\"bytes_per_frame\":{:.0},\"writes_per_frame\":{:.1},",
        if BUFFERED.load(std::sync::atomic::Ordering::Relaxed) { "buf" } else { "line" },
        cfg.cols,
        cfg.rows,
        cfg.zoom,
        cfg.tier_name,
        rendered.get() - rendered0,
        per(acc[0]),
        per(acc[1]),
        per(acc[2]),
        per(acc[3]),
        per(acc[4]),
        pct(&totals, 0.5) as f64 / 1000.0,
        pct(&totals, 0.95) as f64 / 1000.0,
        pct(&totals, 1.0) as f64 / 1000.0,
        (sink.bytes.get() - bytes0) as f64 / n as f64,
        (sink.writes.get() - writes0) as f64 / n as f64,
    );
    dump_stages(&mut out, n);
    out.push('}');
    println!("{out}");
}

/// How fast can one changed half-block cell be encoded? crossterm's
/// `SetColors` + `Print` through `core::fmt`, against a hand-rolled byte push.
fn encode_bench() {
    use crossterm::style::{Color, Colors, Print, SetColors};
    const CELLS: usize = 10_000;
    const REPS: usize = 200;
    let mut seed = 0x9E37_79B9u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as u8
    };
    let cells: Vec<[u8; 6]> = (0..CELLS)
        .map(|_| [next(), next(), next(), next(), next(), next()])
        .collect();
    let mut out: Vec<u8> = Vec::with_capacity(CELLS * 48);
    let t = Instant::now();
    for _ in 0..REPS {
        out.clear();
        for c in &cells {
            let _ = crossterm::queue!(
                out,
                SetColors(Colors::new(
                    Color::Rgb { r: c[0], g: c[1], b: c[2] },
                    Color::Rgb { r: c[3], g: c[4], b: c[5] },
                )),
                Print("\u{2580}")
            );
        }
    }
    let fmt_ns = t.elapsed().as_nanos() as f64 / (CELLS * REPS) as f64;
    let fmt_len = out.len();
    // Hand-rolled: a 256-entry decimal table and plain byte pushes.
    let table: Vec<Vec<u8>> = (0..=255u16).map(|n| n.to_string().into_bytes()).collect();
    let t = Instant::now();
    for _ in 0..REPS {
        out.clear();
        for c in &cells {
            out.extend_from_slice(b"\x1b[38;2;");
            out.extend_from_slice(&table[c[0] as usize]);
            out.push(b';');
            out.extend_from_slice(&table[c[1] as usize]);
            out.push(b';');
            out.extend_from_slice(&table[c[2] as usize]);
            out.extend_from_slice(b";48;2;");
            out.extend_from_slice(&table[c[3] as usize]);
            out.push(b';');
            out.extend_from_slice(&table[c[4] as usize]);
            out.push(b';');
            out.extend_from_slice(&table[c[5] as usize]);
            out.extend_from_slice(b"m\xe2\x96\x80");
        }
    }
    let raw_ns = t.elapsed().as_nanos() as f64 / (CELLS * REPS) as f64;
    println!(
        "{{\"mode\":\"encode\",\"crossterm_ns_per_cell\":{fmt_ns:.1},\"crossterm_bytes_per_cell\":{:.1},\"handrolled_ns_per_cell\":{raw_ns:.1},\"handrolled_bytes_per_cell\":{:.1}}}",
        fmt_len as f64 / CELLS as f64,
        out.len() as f64 / CELLS as f64
    );
}
