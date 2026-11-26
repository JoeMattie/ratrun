//! One file per Phase 1 assignment. Each system stub documents the exact
//! legacy functions and line ranges it must reproduce, the resources and
//! components it uses, and the ordering constraints it relies on.
//!
//! See `CLAUDE.md` ("Determinism rules") for the stable-ordering rule
//! (`SpawnId`).

pub mod boss;
pub mod combat;
pub mod enemy_move;
pub mod evolutions;
pub mod enemy_wave3;
pub mod enemy_wave4;
pub mod fog;
pub mod fx;
pub mod gates;
pub mod hazards;
pub mod loot;
pub mod miniboss;
pub mod player;
pub mod powerups;
pub mod projectiles;
pub mod shop;
pub mod spawn;
pub mod trailer;
pub mod traps;
pub mod weapons;
// Wave 3's self-driving weapons (Laser Eyes, Super Dash) and Counterweight's
// melee-nudge clock. Split out of `weapons` so the auto-fire file stays about
// the weapons it fires.
pub mod weapons_wave3b;
pub mod weapons_wave3a;
// The Spiked Tail Whip's lash (wave 3, w3-spiketail): its own file so the
// wind-up/crack/recoil beat stays in one place.
pub mod weapon_spiketail;
pub mod floor_laundry;
pub mod floor_printworks;
pub mod floor_kitchen;
pub mod floor_lab;
pub mod floor_sewer;
pub mod floor_flatbed;
// The guest: the Drake, the big duck that pecks (Sewer only). Its own file so
// the spawner, the state machine, the one lane geometry and the art bridge sit
// together; chained in the same slot as `floor_sewer`.
pub mod drake;
// Fodder approach sounds (one pair per floor); not yet in the schedule.
pub mod approach;
// Standing water: the rat's wet-floor footsteps and the ripples (right after
// `player` in the schedule).
pub mod puddles;
pub mod weapon_lava;
pub mod miniboss_shapes;
pub mod miniboss_behaviour;
pub mod floor_outfall;
pub mod outfall;
