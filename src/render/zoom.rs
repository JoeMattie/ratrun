//! Mouse-wheel zoom: the ladder of view scales, its stepping and the eased
//! transition between rungs.
//!
//! A zoom factor `z` is **how much world the view shows** relative to the
//! unzoomed view: `z = 1.0` is today's view, `2.0` shows twice the width *and*
//! height of world (everything looks half size) and `0.5` shows half of it
//! (everything looks double size). It is a **render-only** setting: the
//! simulation keeps using the unzoomed logical viewport
//! ([`crate::game::world::World::viewport`]), so collision, the spawn ring,
//! culling for simulation purposes and every rng draw are untouched. The same
//! seed and inputs therefore play out identically at every zoom.
//!
//! The app owns the *setting* (persisted in `scores.json`), [`World`] owns the
//! eased value it renders with, and the wheel, the `Z`/`X` keys and the options
//! row all step the same ladder through [`step`].
//!
//! What zoom deliberately does **not** do: move the spawn ring. Spawns are
//! still placed just outside the *unzoomed* viewport, so at `z = 2` an enemy can
//! be seen popping into existence in the extra band of world the wider view
//! reveals. That is accepted: pushing the ring out with the zoom would change
//! where enemies appear — simulation state — and the same seed would then play
//! differently at different zooms (see
//! `game::world::zoom_tests::the_same_seed_plays_out_identically_at_every_zoom`).
//!
//! [`World`]: crate::game::world::World

/// The zoom ladder, closest view first: `0.5` (things look double size) …
/// `2.0` (twice the world, things look half size). `1.0` is the reference
/// view — the one this game had before the wheel did anything.
pub const STEPS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// The closest zoom in the ladder.
pub const MIN: f32 = STEPS[0];
/// The widest zoom in the ladder.
pub const MAX: f32 = STEPS[STEPS.len() - 1];
/// The unzoomed reference view (the one today's players know).
pub const DEFAULT: f32 = 1.0;

/// Seconds the eased transition takes to settle on a new rung.
///
/// Exponential ease with a time constant of `EASE_SECS / 3`, so it is ~95 %
/// there at `EASE_SECS` and framerate independent (a 15 fps step and a 144 fps
/// step land in the same place).
pub const EASE_SECS: f32 = 0.15;

/// Clamp any value into the ladder's range (a hand-edited save file can hold
/// anything; `f32::NAN` snaps to [`DEFAULT`]).
pub fn clamp(zoom: f32) -> f32 {
    if zoom.is_nan() {
        return DEFAULT;
    }
    zoom.clamp(MIN, MAX)
}

/// The ladder rung nearest `zoom` (clamped first, so out-of-range values land
/// on the closest end of the ladder instead of off it).
pub fn snap(zoom: f32) -> f32 {
    let zoom = clamp(zoom);
    let mut best = DEFAULT;
    let mut best_d = f32::INFINITY;
    for &step in STEPS.iter() {
        let d = (step - zoom).abs();
        if d < best_d {
            best_d = d;
            best = step;
        }
    }
    best
}

/// The ladder index of the rung nearest `zoom`.
pub fn index_of(zoom: f32) -> usize {
    let target = snap(zoom);
    STEPS.iter().position(|&s| s == target).unwrap_or(2)
}

/// Step one rung along the ladder. `dir > 0` widens the view (scroll down /
/// `X`, more world, smaller things) and `dir < 0` zooms in. The ends clamp: a
/// step past either end is a no-op, not a wrap.
pub fn step(zoom: f32, dir: isize) -> f32 {
    let idx = index_of(zoom) as isize;
    let next = (idx + dir.signum()).clamp(0, STEPS.len() as isize - 1) as usize;
    STEPS[next]
}

/// Ease `current` toward `target` over one frame of `dt` seconds. Snaps onto
/// the exact target once it is within a rounding error, so a settled zoom is
/// bit-identical to the setting (and the renderer takes its 1:1 fast path).
pub fn ease(current: f32, target: f32, dt: f32) -> f32 {
    let target = clamp(target);
    let current = clamp(current);
    if dt <= 0.0 || current == target {
        return target;
    }
    let k = 1.0 - (-dt.max(0.0) * (3.0 / EASE_SECS)).exp();
    let next = current + (target - current) * k;
    if (target - next).abs() < 0.004 {
        target
    } else {
        next
    }
}

/// Write a human-facing zoom read-out into `buf` (`"1x"`, `"1.5x"`, `"0.75x"`),
/// trailing zeros trimmed so the common rungs read as whole numbers.
///
/// Allocation-free: the app reuses the HUD's badge buffer.
pub fn format_into(buf: &mut String, zoom: f32) {
    let v = snap(zoom);
    let hundredths = (v * 100.0).round().max(0.0) as u32;
    let whole = hundredths / 100;
    let frac = hundredths % 100;
    if frac == 0 {
        let _ = std::fmt::Write::write_fmt(buf, format_args!("{whole}x"));
    } else if frac.is_multiple_of(10) {
        let _ = std::fmt::Write::write_fmt(buf, format_args!("{whole}.{}x", frac / 10));
    } else {
        let _ = std::fmt::Write::write_fmt(buf, format_args!("{whole}.{frac:02}x"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_runs_from_half_to_double_the_view() {
        assert_eq!(MIN, 0.5);
        assert_eq!(MAX, 2.0);
        assert_eq!(DEFAULT, 1.0);
        assert!(STEPS.windows(2).all(|w| w[0] < w[1]), "the ladder ascends");
        assert!(STEPS.contains(&1.0), "today's view is a rung");
    }

    #[test]
    fn stepping_clamps_at_both_ends_and_needs_a_direction() {
        assert_eq!(step(1.0, 1), 1.25, "scroll down widens");
        assert_eq!(step(1.0, -1), 0.75, "scroll up zooms in");
        assert_eq!(step(MAX, 1), MAX, "the widest view does not wrap");
        assert_eq!(step(MIN, -1), MIN, "the closest view does not wrap");
        assert_eq!(step(1.0, 0), 1.0, "no direction, no step");
        // Walking the whole ladder both ways visits every rung in order.
        let mut z = MIN;
        let mut seen = vec![z];
        while z < MAX {
            z = step(z, 1);
            seen.push(z);
        }
        assert_eq!(seen, STEPS.to_vec());
    }

    #[test]
    fn snapping_tolerates_junk() {
        assert_eq!(snap(1.0), 1.0);
        assert_eq!(snap(1.26), 1.25, "nearest rung wins");
        assert_eq!(snap(0.0), MIN, "below the ladder clamps to the closest view");
        assert_eq!(snap(9.0), MAX, "above the ladder clamps to the widest view");
        assert_eq!(snap(f32::NAN), DEFAULT, "a NaN save loads unzoomed");
        assert_eq!(snap(f32::INFINITY), MAX);
        assert_eq!(index_of(0.5), 0);
        assert_eq!(index_of(2.0), STEPS.len() - 1);
    }

    #[test]
    fn the_ease_settles_on_the_target_in_about_a_sixth_of_a_second() {
        let mut z = 1.0;
        let mut t = 0.0;
        while t < EASE_SECS {
            z = ease(z, 2.0, 1.0 / 60.0);
            t += 1.0 / 60.0;
            assert!(z > 1.0 && z < 2.0, "the ease never overshoots: {z}");
        }
        assert!((z - 2.0).abs() < 0.1, "95 % there after EASE_SECS, got {z}");
        // One more frame is not enough to be bit-exact, but a couple more are:
        // the tail snaps so the renderer's 1:1 fast path comes back.
        for _ in 0..30 {
            z = ease(z, 2.0, 1.0 / 60.0);
        }
        assert_eq!(z, 2.0, "the ease ends on the exact rung");
        assert_eq!(ease(z, 2.0, 1.0 / 60.0), 2.0, "and stays there");
        // Framerate independence: one big step lands in the same place as many
        // small ones.
        let big = ease(1.0, 2.0, EASE_SECS);
        assert!(big > 1.9, "one EASE_SECS step is nearly settled, got {big}");
        assert_eq!(ease(1.5, 1.5, 1.0 / 60.0), 1.5);
        assert_eq!(ease(1.5, 2.0, 0.0), 2.0, "a zero step snaps");
    }

    #[test]
    fn the_read_out_trims_trailing_zeros() {
        let mut buf = String::new();
        for (zoom, want) in [
            (0.5, "0.5x"),
            (0.75, "0.75x"),
            (1.0, "1x"),
            (1.25, "1.25x"),
            (1.5, "1.5x"),
            (2.0, "2x"),
        ] {
            buf.clear();
            format_into(&mut buf, zoom);
            assert_eq!(buf, want);
        }
    }
}
