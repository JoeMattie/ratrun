//! Platform clocks for render pacing, profiling and fresh run seeds.

#[cfg(target_arch = "wasm32")]
pub use bevy_platform::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
pub fn seed_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn seed_time() -> u64 {
    // Add random low bits to the browser's millisecond wall time.
    ((js_sys::Date::now() as u64) << 20) | ((js_sys::Math::random() * 1_048_576.0) as u64)
}
