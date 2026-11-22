use super::*;
use crate::game::library::all_rows;

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

/// One strip: `frames` plates across one loop, side by side.
fn strip(key: Key, w: usize, h: usize, frames: usize) -> Option<PixelBuffer> {
    let mut scene = LibraryScene::build(key)?;
    let gap = 2;
    let mut out = PixelBuffer::new(frames * (w + gap), h);
    out.clear((0, 0, 0));
    let mut art = PixelBuffer::new(w, h);
    let total = (scene.loop_secs() / TICK) as usize;
    let mut next = 0;
    for i in 0..total {
        // Sample the middle of each of `frames` equal spans.
        if next < frames && i == (next * 2 + 1) * total / (frames * 2) {
            scene.draw(&mut art);
            for y in 0..h {
                for x in 0..w {
                    out.plot((next * (w + gap) + x) as i32, y as i32, art.pixels()[y * w + x]);
                }
            }
            next += 1;
        }
        scene.step();
    }
    Some(out)
}

fn write_ansi(dir: &std::path::Path, name: &str, pb: &PixelBuffer) {
    let mut s = String::new();
    let px = pb.pixels();
    for cy in 0..pb.h / 2 {
        for x in 0..pb.w {
            let t = px[cy * 2 * pb.w + x];
            let b = px[(cy * 2 + 1) * pb.w + x];
            s.push_str(&format!(
                "\x1b[38;2;{};{};{};48;2;{};{};{}m▀",
                t.0, t.1, t.2, b.0, b.1, b.2
            ));
        }
        s.push_str("\x1b[0m\n");
    }
    let path = dir.join(format!("{}x{}-{}.ansi", pb.w, pb.h / 2, name));
    std::fs::write(path, s).expect("write the strip");
}

/// `RATRUN_LIBANIM_DIR=<dir> RATRUN_LIBANIM=<names|all|tab> cargo test
/// capture_library_filmstrips`: one `.ansi` strip per card (or, with
/// `RATRUN_LIBANIM_SHEET=n`, sheets of `n` cards stacked) for
/// `scratchpad/cheese/view.py`. A no-op without the directory.
#[test]
fn capture_library_filmstrips() {
    let Ok(dir) = std::env::var("RATRUN_LIBANIM_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create the capture directory");
    let want = std::env::var("RATRUN_LIBANIM").unwrap_or_else(|_| "all".into());
    let want: Vec<String> = want.split(',').map(slug).collect();
    let frames: usize = std::env::var("RATRUN_LIBANIM_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let sheet: usize = std::env::var("RATRUN_LIBANIM_SHEET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let (w, h) = (44, 30);
    let mut strips: Vec<(String, PixelBuffer)> = Vec::new();
    for row in all_rows() {
        let name = slug(row.name);
        let tab = slug(row.tab.title());
        let group = slug(row.group);
        if !want.iter().any(|x| x == "all" || *x == name || *x == tab || *x == group) {
            continue;
        }
        if let Some(pb) = strip(row.key, w, h, frames) {
            strips.push((name, pb));
        }
    }
    if sheet == 0 {
        for (name, pb) in &strips {
            write_ansi(&dir, name, pb);
        }
        return;
    }
    for (n, chunk) in strips.chunks(sheet).enumerate() {
        let mut out = PixelBuffer::new(chunk[0].1.w, chunk.len() * (h + 2));
        out.clear((0, 0, 0));
        for (i, (_, pb)) in chunk.iter().enumerate() {
            for y in 0..h {
                for x in 0..pb.w {
                    out.plot(x as i32, (i * (h + 2) + y) as i32, pb.pixels()[y * pb.w + x]);
                }
            }
        }
        let names: Vec<&str> = chunk.iter().map(|(n, _)| n.as_str()).collect();
        eprintln!("sheet {n}: {}", names.join(", "));
        write_ansi(&dir, &format!("sheet{n:02}-{}", want[0]), &out);
    }
}
