//! Platform-independent maths for the **simulation**.
//!
//! # Why this module exists
//!
//! Rust's `f32::sin`, `cos`, `sin_cos`, `tan`, `atan2`, `acos`, `asin`, `atan`,
//! `powf`, `exp`, `ln`, `log2`, `log10`, `hypot`, `tanh` and `cbrt` are *not*
//! specified bit-for-bit: they forward to the platform's maths library (the
//! system libm on Linux, the Accelerate/system libraries on macOS, the UCRT on
//! Windows), and those libraries disagree in the last bit or two. One such
//! call in simulation code is enough to make a run diverge a tick later, which
//! is exactly what the scripted traces (`game/ecs/harness.rs`) caught when the
//! traces recorded on macOS/arm64 were replayed on Linux and Windows CI: all
//! six differed.
//!
//! [`libm`] is a pure-Rust implementation of the same functions with one
//! implementation on every target, so a call routed through it is
//! bit-identical on macOS, Linux, Windows, arm64 and x86-64 alike. Every
//! maths call that can change simulated state or an rng draw must go through
//! this module; `clippy.toml` (`disallowed-methods`) makes a new `f32` std
//! maths call an error unless the file is explicitly marked render-only.
//!
//! # What deliberately stays on `std`
//!
//! These are already exactly specified by IEEE 754 (Rust's `std` requires the
//! platform to provide them exactly; they compile to a single instruction on
//! every target we ship), so they cannot differ between platforms and there is
//! no reason to pay for a software implementation:
//!
//! - `sqrt`, `abs`, `floor`, `ceil`, `round`, `trunc`, `fract`,
//!   `copysign`, `signum`, `to_bits`/`from_bits`
//! - `min`, `max`, `clamp`
//! - the arithmetic operators `+ - * /` (correctly rounded, no fused
//!   operations are emitted: the build does not enable `fast-math`, and `mul_add`
//!   is never used in simulation code)
//!
//! `powi` is *also* not a platform risk (LLVM lowers it to a chain of
//! multiplies), but it is disallowed by the lint alongside `powf` so that an
//! exponent change can never quietly turn into a libm call, and because a
//! literal exponent is clearer written out. See [`powi`].
//!
//! # How call sites change
//!
//! By one letter. [`DetF32`] is implemented for `f32` with a `d`-prefixed
//! method for each wrapper, so `a.sin()` becomes `a.dsin()` and a chain such as
//! `(x * 0.2).cos().abs()` becomes `(x * 0.2).dcos().abs()`. Free functions
//! ([`sin`], [`cos`], …) exist for the rarer shapes (`Vec2::new(angle.sin(),
//! angle.cos())` reads better as [`sin_cos`]).
//!
//! f64: the simulation uses no f64 trigonometry at all (its only f64 maths is
//! [`DetF64::dpowi`], in `game/player.rs`'s XP curve and the batch report's
//! sample-size formula), so only the f64 wrapper that is actually needed is
//! provided. Add f64 wrappers here the day simulation code needs one — do not
//! reach for `f64::sin` in the meantime.

/// `sin(x)`, bit-identical on every platform.
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// `cos(x)`, bit-identical on every platform.
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// `(sin(x), cos(x))` — two `libm` calls, never the platform's fused
/// `sincos`, so both halves are the same numbers [`sin`] and [`cos`] return.
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    (libm::sinf(x), libm::cosf(x))
}

/// `tan(x)`, bit-identical on every platform.
#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}

/// `atan2(y, x)`, bit-identical on every platform.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// `acos(x)`, bit-identical on every platform.
#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}

/// `asin(x)`, bit-identical on every platform.
#[inline]
pub fn asin(x: f32) -> f32 {
    libm::asinf(x)
}

/// `atan(x)`, bit-identical on every platform.
#[inline]
pub fn atan(x: f32) -> f32 {
    libm::atanf(x)
}

/// `x^y`, bit-identical on every platform.
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// `e^x`, bit-identical on every platform.
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

/// Natural logarithm, bit-identical on every platform.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

/// Base-2 logarithm, bit-identical on every platform.
#[inline]
pub fn log2(x: f32) -> f32 {
    libm::log2f(x)
}

/// Base-10 logarithm, bit-identical on every platform.
#[inline]
pub fn log10(x: f32) -> f32 {
    libm::log10f(x)
}

/// `sqrt(x² + y²)`, bit-identical on every platform.
#[inline]
pub fn hypot(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
}

/// `tanh(x)`, bit-identical on every platform.
#[inline]
pub fn tanh(x: f32) -> f32 {
    libm::tanhf(x)
}

/// Cube root, bit-identical on every platform.
#[inline]
pub fn cbrt(x: f32) -> f32 {
    libm::cbrtf(x)
}

/// `x` raised to an integer power by plain left-to-right repeated
/// multiplication.
///
/// Every partial product is one correctly rounded `*`, so the result depends
/// only on the operand and the exponent — unlike a library `powf`, and unlike
/// a `powi` that a future optimiser might lower differently. `n == 0` is
/// `1.0` (also for `x == 0`), a negative `n` is the reciprocal of the positive
/// loop, and the accumulate order is `((x·x)·x)·x…`, i.e. exactly what the
/// written-out `x * x * x` does for `n == 2`/`3`.
///
/// Simulation code should prefer the written-out multiplication for a literal
/// 2 or 3 (`x * x`, `x * x * x`) and this function for anything else.
#[inline]
pub fn powi(x: f32, n: i32) -> f32 {
    let mut acc = 1.0f32;
    let mut i = 0u32;
    while i < n.unsigned_abs() {
        acc *= x;
        i += 1;
    }
    if n < 0 {
        1.0 / acc
    } else {
        acc
    }
}

/// `x` raised to an integer power, the f64 form of [`powi`] (same loop).
///
/// The simulation's XP curve (`player::base_xp_for_level`) reads
/// `XP_GROWTH.powi(n)` on f64; this keeps that exact and identical everywhere.
#[inline]
pub fn powi64(x: f64, n: i32) -> f64 {
    let mut acc = 1.0f64;
    let mut i = 0u32;
    while i < n.unsigned_abs() {
        acc *= x;
        i += 1;
    }
    if n < 0 {
        1.0 / acc
    } else {
        acc
    }
}

/// The `d`-prefixed, platform-independent form of every `f32` maths method the
/// simulation uses. `use crate::detmath::DetF32;` and change one letter:
/// `a.sin()` → `a.dsin()`.
///
/// The trait is not sealed, but do not implement it elsewhere: an impl outside
/// this module would defeat the lint.
pub trait DetF32 {
    /// [`sin`]
    fn dsin(self) -> f32;
    /// [`cos`]
    fn dcos(self) -> f32;
    /// [`sin_cos`]
    fn dsin_cos(self) -> (f32, f32);
    /// [`tan`]
    fn dtan(self) -> f32;
    /// [`atan2`], with `self` as the `y` argument: `y.datan2(x)`.
    fn datan2(self, other: f32) -> f32;
    /// [`atan`]
    fn datan(self) -> f32;
    /// [`acos`]
    fn dacos(self) -> f32;
    /// [`asin`]
    fn dasin(self) -> f32;
    /// [`powf`]
    fn dpowf(self, e: f32) -> f32;
    /// [`powi`]
    fn dpowi(self, n: i32) -> f32;
    /// [`exp`]
    fn dexp(self) -> f32;
    /// [`ln`]
    fn dln(self) -> f32;
    /// [`log2`]
    fn dlog2(self) -> f32;
    /// [`log10`]
    fn dlog10(self) -> f32;
    /// [`hypot`], with `self` as the first argument: `x.dhypot(y)`.
    fn dhypot(self, other: f32) -> f32;
    /// [`tanh`]
    fn dtanh(self) -> f32;
    /// [`cbrt`]
    fn dcbrt(self) -> f32;
}

impl DetF32 for f32 {
    #[inline]
    fn dsin(self) -> f32 {
        sin(self)
    }
    #[inline]
    fn dcos(self) -> f32 {
        cos(self)
    }
    #[inline]
    fn dsin_cos(self) -> (f32, f32) {
        sin_cos(self)
    }
    #[inline]
    fn dtan(self) -> f32 {
        tan(self)
    }
    #[inline]
    fn datan2(self, other: f32) -> f32 {
        atan2(self, other)
    }
    #[inline]
    fn datan(self) -> f32 {
        atan(self)
    }
    #[inline]
    fn dacos(self) -> f32 {
        acos(self)
    }
    #[inline]
    fn dasin(self) -> f32 {
        asin(self)
    }
    #[inline]
    fn dpowf(self, e: f32) -> f32 {
        powf(self, e)
    }
    #[inline]
    fn dpowi(self, n: i32) -> f32 {
        powi(self, n)
    }
    #[inline]
    fn dexp(self) -> f32 {
        exp(self)
    }
    #[inline]
    fn dln(self) -> f32 {
        ln(self)
    }
    #[inline]
    fn dlog2(self) -> f32 {
        log2(self)
    }
    #[inline]
    fn dlog10(self) -> f32 {
        log10(self)
    }
    #[inline]
    fn dhypot(self, other: f32) -> f32 {
        hypot(self, other)
    }
    #[inline]
    fn dtanh(self) -> f32 {
        tanh(self)
    }
    #[inline]
    fn dcbrt(self) -> f32 {
        cbrt(self)
    }
}

/// The `d`-prefixed form of the one `f64` maths call simulation code makes.
/// See the module docs: f64 trigonometry is not used by the simulation, so
/// only `powi` is mirrored here.
pub trait DetF64 {
    /// [`powi64`]
    fn dpowi(self, n: i32) -> f64;
}

impl DetF64 for f64 {
    #[inline]
    fn dpowi(self, n: i32) -> f64 {
        powi64(self, n)
    }
}

#[cfg(test)]
// The reference values here *are* `std`'s: each wrapper is checked against the
// platform function it replaces, so the lint's own subject is the test's point.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    /// Two ulps of a value near 1.0 is ~2e-7; 2e-6 is loose enough that a
    /// correct wiring never trips it and tight enough that being wired to the
    /// *wrong* function (say `cos` for `sin`) always does.
    const TOL: f32 = 2e-6;

    /// Inputs spread across the ranges the simulation actually uses (angles in
    /// radians, dot products in `[-1, 1]`, small positive bases for `pow`).
    const ANGLES: [f32; 9] = [
        -6.283_185_5,
        -3.0,
        -1.0,
        -0.5,
        -0.0,
        0.0,
        0.25,
        1.0,
        3.0,
    ];
    const UNIT: [f32; 7] = [-1.0, -0.9, -0.5, -0.2, 0.0, 0.3, 1.0];
    const POWS: [(f32, f32); 6] = [
        (0.5, 0.5),
        (2.0, 0.5),
        (1.2, 1.2),
        (0.0, 2.0),
        (7.5, 1.45),
        (0.25, 2.0),
    ];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= TOL * (1.0 + b.abs())
    }

    #[test]
    fn every_wrapper_agrees_with_std() {
        for x in ANGLES {
            assert!(close(sin(x), x.sin()), "sin({x})");
            assert!(close(cos(x), x.cos()), "cos({x})");
            assert!(close(tan(x), x.tan()), "tan({x})");
            assert!(close(atan(x), x.atan()), "atan({x})");
            assert!(close(exp(x), x.exp()), "exp({x})");
            for y in ANGLES {
                assert!(close(atan2(x, y), x.atan2(y)), "atan2({x}, {y})");
                assert!(close(hypot(x, y), x.hypot(y)), "hypot({x}, {y})");
            }
        }
        for x in UNIT {
            assert!(close(acos(x), x.acos()), "acos({x})");
            assert!(close(asin(x), x.asin()), "asin({x})");
            assert!(close(tanh(x), x.tanh()), "tanh({x})");
        }
        for x in [0.5f32, 1.0, 2.0, 7.5, 12.0, 90.0] {
            assert!(close(ln(x), x.ln()), "ln({x})");
            assert!(close(log2(x), x.log2()), "log2({x})");
            assert!(close(log10(x), x.log10()), "log10({x})");
            assert!(close(cbrt(x), x.cbrt()), "cbrt({x})");
        }
        for (b, e) in POWS {
            assert!(close(powf(b, e), b.powf(e)), "powf({b}, {e})");
        }
    }

    #[test]
    fn the_trait_methods_reach_the_same_numbers_as_the_free_functions() {
        for x in ANGLES {
            assert_eq!(x.dsin().to_bits(), sin(x).to_bits());
            assert_eq!(x.dcos().to_bits(), cos(x).to_bits());
            assert_eq!(x.dsin_cos(), sin_cos(x));
            assert_eq!(x.dtan().to_bits(), tan(x).to_bits());
            assert_eq!(x.datan().to_bits(), atan(x).to_bits());
            assert_eq!(x.dexp().to_bits(), exp(x).to_bits());
            assert_eq!(x.dpowf(1.2).to_bits(), powf(x, 1.2).to_bits());
            assert_eq!(x.dhypot(0.7).to_bits(), hypot(x, 0.7).to_bits());
            assert_eq!(1.5f32.dpowi(4).to_bits(), powi(1.5, 4).to_bits());
            assert_eq!(1.5f64.dpowi(6).to_bits(), powi64(1.5, 6).to_bits());
        }
        for x in UNIT {
            assert_eq!(x.dacos().to_bits(), acos(x).to_bits());
            assert_eq!(x.dasin().to_bits(), asin(x).to_bits());
            assert_eq!(x.dtanh().to_bits(), tanh(x).to_bits());
        }
        for x in [0.5f32, 2.0, 7.5] {
            assert_eq!(x.dln().to_bits(), ln(x).to_bits());
            assert_eq!(x.dlog2().to_bits(), log2(x).to_bits());
        }
        for x in [0.5f32, 7.5, 90.0] {
            assert_eq!(x.dlog10().to_bits(), log10(x).to_bits());
            assert_eq!(x.dcbrt().to_bits(), cbrt(x).to_bits());
        }
    }

    #[test]
    fn sin_cos_is_sin_then_cos() {
        for x in ANGLES {
            let (s, c) = sin_cos(x);
            assert_eq!(s.to_bits(), sin(x).to_bits(), "sin half of sin_cos({x})");
            assert_eq!(c.to_bits(), cos(x).to_bits(), "cos half of sin_cos({x})");
        }
        // The identity still holds numerically, so the pair is not transposed.
        for x in ANGLES {
            let (s, c) = sin_cos(x);
            assert!((s * s + c * c - 1.0).abs() < 1e-5, "sin²+cos² at {x}");
        }
    }

    #[test]
    fn powi_matches_written_out_multiplication() {
        for x in [1.12f32, 1.5, -0.5, 2.0, 0.3] {
            assert_eq!(powi(x, 2).to_bits(), (x * x).to_bits(), "{x}²");
            assert_eq!(powi(x, 3).to_bits(), (x * x * x).to_bits(), "{x}³");
            assert_eq!(powi(x, 1).to_bits(), x.to_bits(), "{x}¹");
            assert_eq!(powi(x, 0), 1.0, "{x}⁰");
            // Left-to-right accumulation, i.e. the same product the longhand
            // expression forms: (((x·x)·x)·x·x·x).
            let longhand = x * x * x * x * x * x;
            assert_eq!(powi(x, 6).to_bits(), longhand.to_bits(), "{x}⁶");
        }
        // A negative exponent is the reciprocal of the positive loop.
        for x in [2.0f32, 1.5, -0.5] {
            assert_eq!(powi(x, -3).to_bits(), (1.0 / (x * x * x)).to_bits(), "{x}⁻³");
        }
        // The f64 form is the same loop.
        for x in [1.10f64, 1.5, 0.3] {
            let longhand = x * x * x * x * x * x;
            assert_eq!(powi64(x, 6).to_bits(), longhand.to_bits(), "{x}⁶ (f64)");
            assert_eq!(powi64(x, 0), 1.0);
        }
    }

    // ---- the pinned table the coordinator fills in -------------------------
    //
    // These eight input → output bit patterns pin the *exact* rounding of the
    // libm implementations we depend on. They are the whole point of this
    // module: if a libm bump, a Cargo feature or an accidental `std` call
    // changes any of them, the traces move. The coordinator fills them
    // from the first run (see `print_the_pinned_table`) and from then on a
    // mismatch names the function and the input.
    //
    // `0x0000_0000` below are placeholders, not values: the test is
    // `#[ignore]`d until they are filled.

    /// `(input, sin bits, cos bits)`.
    const PINNED_SIN_COS: [(f32, u32, u32); 8] = [
        (0.0, 0x0000_0000, 0x0000_0000), // TODO coordinator: fill from first run
        (0.25, 0x0000_0000, 0x0000_0000), // TODO coordinator: fill from first run
        (1.0, 0x0000_0000, 0x0000_0000), // TODO coordinator: fill from first run
        (
            1.0 / 60.0,
            0x0000_0000,
            0x0000_0000,
        ), // TODO coordinator: fill from first run
        (
            core::f32::consts::PI,
            0x0000_0000,
            0x0000_0000,
        ), // TODO coordinator: fill from first run
        (
            core::f32::consts::FRAC_PI_2,
            0x0000_0000,
            0x0000_0000,
        ), // TODO coordinator: fill from first run
        (-2.5, 0x0000_0000, 0x0000_0000), // TODO coordinator: fill from first run
        (12.0, 0x0000_0000, 0x0000_0000), // TODO coordinator: fill from first run
    ];

    /// `(y, x, atan2(y, x) bits)`.
    const PINNED_ATAN2: [(f32, f32, u32); 8] = [
        (0.0, 1.0, 0x0000_0000),  // TODO coordinator: fill from first run
        (1.0, 0.0, 0x0000_0000),  // TODO coordinator: fill from first run
        (1.0, 1.0, 0x0000_0000),  // TODO coordinator: fill from first run
        (-1.0, 1.0, 0x0000_0000), // TODO coordinator: fill from first run
        (1.0, -1.0, 0x0000_0000), // TODO coordinator: fill from first run
        (0.5, 3.25, 0x0000_0000), // TODO coordinator: fill from first run
        (-3.5, -0.25, 0x0000_0000), // TODO coordinator: fill from first run
        (0.013, 0.7, 0x0000_0000), // TODO coordinator: fill from first run
    ];

    /// Run this once (`cargo test print_the_pinned_table -- --ignored --nocapture`),
    /// paste its output into the two tables above, then the ignored test below
    /// starts pinning them.
    #[test]
    #[ignore = "prints the table the coordinator pins"]
    fn print_the_pinned_table() {
        println!("const PINNED_SIN_COS: [(f32, u32, u32); 8] = [");
        for &(x, _, _) in &PINNED_SIN_COS {
            let (s, c) = sin_cos(x);
            println!(
                "    ({:?}, 0x{:08X}, 0x{:08X}),",
                x,
                s.to_bits(),
                c.to_bits()
            );
        }
        println!("];");
        println!("const PINNED_ATAN2: [(f32, f32, u32); 8] = [");
        for &(y, x, _) in &PINNED_ATAN2 {
            println!(
                "    ({:?}, {:?}, 0x{:08X}),",
                y,
                x,
                atan2(y, x).to_bits()
            );
        }
        println!("];");
    }

    #[test]
    #[ignore = "the coordinator pins the table from the first run"]
    fn the_libm_rounding_is_pinned() {
        for &(x, s_bits, c_bits) in &PINNED_SIN_COS {
            let (s, c) = sin_cos(x);
            assert_eq!(s.to_bits(), s_bits, "sin({x:?}) rounding moved");
            assert_eq!(c.to_bits(), c_bits, "cos({x:?}) rounding moved");
        }
        for &(y, x, bits) in &PINNED_ATAN2 {
            assert_eq!(atan2(y, x).to_bits(), bits, "atan2({y:?}, {x:?}) rounding moved");
        }
    }

    /// The wrappers are `#[inline]` and must not have picked up a different
    /// implementation: on this build, `sin` is the libm one, not the platform
    /// one (they agree to a couple of ulps, so this only fails if the wrapper
    /// stops calling libm at all — kept as a cheap smoke check).
    #[test]
    fn the_wrapper_is_not_a_passthrough_to_the_platform() {
        // Any correct sin at these points is within a ulp of std's, so this is
        // a shape check, not a bit check: it catches a wrapper wired to the
        // wrong function or returning its argument.
        for x in [0.0f32, 0.5, 1.0, 2.0, 3.0] {
            assert!(close(sin(x), x.sin()));
            assert!((sin_cos(x).0 - x.sin()).abs() < 1e-3);
        }
    }
}
