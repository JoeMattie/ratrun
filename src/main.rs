//! Rat Run — a terminal bullet-hell horde survivor built with ratatui.

// A few small geometry/helper methods are kept as deliberate building-block
// API even when not every one is wired up yet.
#![allow(dead_code)]

mod app;
#[cfg(not(target_arch = "wasm32"))]
mod autoplay;
mod audio;
#[cfg(not(target_arch = "wasm32"))]
mod config;
mod controls;
mod gamepad;
mod console;
mod input;
mod lore;
mod meta;
// Platform-independent maths for the simulation (libm instead of the
// platform maths library); see the module docs and `clippy.toml`.
mod detmath;
mod math;
mod music;
mod present;
mod prof;
#[cfg(not(target_arch = "wasm32"))]
mod profbench;
mod render;
mod scores;
#[cfg(not(target_arch = "wasm32"))]
mod spectate;
mod term;
mod titleart;

mod game;

mod clock;
mod input_event;

#[cfg(not(target_arch = "wasm32"))]
include!("native.rs");

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web;

#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
compile_error!("Enable the web feature for a wasm32 build.");

#[cfg(target_arch = "wasm32")]
fn main() {}
