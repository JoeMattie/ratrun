//! Title-screen rat, rendered from an embedded image into ASCII via RASCII
//! (`rascii_art`). We render uncolored (shape only) and apply our own color
//! gradient in the menu so it matches the game's palette.

use rascii_art::{charsets, render_image_to, RenderOptions};

/// The rat source art, embedded so there's no runtime file dependency.
const RAT_PNG: &[u8] = include_bytes!("../assets/rat.png");

/// Render the rat to ASCII lines at the given character width. Returns `None`
/// if the image can't be decoded/rendered (the title falls back to text).
pub fn rat_lines(width: u32) -> Option<Vec<String>> {
    let img = image::load_from_memory(RAT_PNG).ok()?;
    let mut buf = String::new();
    render_image_to(
        &img,
        &mut buf,
        &RenderOptions::new()
            .width(width)
            .colored(false)
            .charset(charsets::BLOCK),
    )
    .ok()?;
    let lines: Vec<String> = buf
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_rat_ascii() {
        let lines = rat_lines(44).expect("rat art should render");
        assert!(lines.len() >= 8, "expected a few rows of art");
        // Should contain block glyphs, not just spaces.
        assert!(lines.iter().any(|l| l.contains('█') || l.contains('▓')));

        if std::env::var("RATRUN_DUMP").is_ok() {
            std::fs::write("/tmp/rat_ascii.txt", lines.join("\n")).unwrap();
        }
    }
}
