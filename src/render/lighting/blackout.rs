//! Scheduled fixture outages. The curve clock drives the picture and the
//! crossing of its edges drives the App's cues.

use crate::audio::Sfx;
use crate::game::level::Theme;

const FADE_DOWN: f32 = 0.6;
const FADE_UP: f32 = 1.0;
const DARK: f32 = 8.0;
const FIRST: f32 = 5.0 * 60.0;
const SECOND: f32 = 11.0 * 60.0;
const LAB_SECOND: f32 = 7.5 * 60.0;
const LAB_DARK: f32 = 18.0;

fn windows(theme: Theme) -> impl Iterator<Item = (f32, f32)> {
    let second = if theme == Theme::Lab {
        (LAB_SECOND, LAB_DARK)
    } else {
        (SECOND, DARK)
    };
    [(FIRST, DARK), second]
        .into_iter()
        .filter(move |_| theme != Theme::Trailer)
}

/// Fraction of fixture light lost, from 0 (powered) to 1 (fully dark).
pub(crate) fn factor(theme: Theme, clock: f32) -> f32 {
    for (start, dark) in windows(theme) {
        let down = start + FADE_DOWN;
        let up = down + dark;
        let end = up + FADE_UP;
        if (start..down).contains(&clock) {
            return (clock - start) / (down - start);
        }
        if (down..up).contains(&clock) {
            return 1.0;
        }
        if (up..end).contains(&clock) {
            return (end - clock) / (end - up);
        }
    }
    0.0
}

/// Reuse the press's low servo drop and the mower dock's rising hum. Compare
/// clock edges so pausing or rendering a second time never repeats a sound.
pub(crate) fn cues(theme: Theme, before: f32, now: f32) -> impl Iterator<Item = Sfx> {
    windows(theme).flat_map(move |(start, dark)| {
        [(start, Sfx::NightBed), (start + FADE_DOWN + dark, Sfx::MowerDockHum)]
            .into_iter()
            .filter_map(move |(at, cue)| (before < at && at <= now).then_some(cue))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blackout_fades_holds_and_recovers_at_every_appointment() {
        fn check(theme: Theme, start: f32, dark: f32) {
            let down = start + 0.6;
            let up = down + dark;
            assert_eq!(factor(theme, start - 0.01), 0.0);
            assert_eq!(factor(theme, start), 0.0);
            assert!((factor(theme, start + 0.3) - 0.5).abs() < 0.001);
            for t in [down, down + dark * 0.5, up - 0.01, up] {
                assert_eq!(factor(theme, t), 1.0);
            }
            assert_eq!(factor(theme, up + 0.5), 0.5);
            assert_eq!(factor(theme, up + 1.0), 0.0);
        }

        for theme in Theme::all() {
            check(theme, 300.0, 8.0);
            if theme == Theme::Lab {
                check(theme, 450.0, 18.0);
                assert_eq!(factor(theme, 664.0), 0.0);
            } else {
                check(theme, 660.0, 8.0);
                assert_eq!(factor(theme, 454.0), 0.0);
            }
        }
        for t in [304.0, 454.0, 664.0] {
            assert_eq!(factor(Theme::Trailer, t), 0.0);
        }
    }

    #[test]
    fn blackout_cues_fire_once_when_the_clock_crosses_an_edge() {
        assert_eq!(cues(Theme::Sewer, 299.9, 300.0).next(), Some(Sfx::NightBed));
        assert_eq!(cues(Theme::Sewer, 300.0, 300.1).next(), None);
        assert_eq!(cues(Theme::Lab, 455.0, 455.0).next(), None);
        assert_eq!(cues(Theme::Lab, 468.5, 468.7).next(), Some(Sfx::MowerDockHum));
        assert_eq!(cues(Theme::Lab, 468.7, 469.6).next(), None);
        assert_eq!(cues(Theme::Lab, 659.0, 670.0).next(), None);
        assert_eq!(cues(Theme::Trailer, 299.0, 670.0).next(), None);
    }
}
