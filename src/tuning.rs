//! Run-local balance. Files are resolved once at the CLI boundary.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bevy_ecs::resource::Resource;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::game::ecs::systems::{boss::BossKind, miniboss, player::DASH_SECS};
use crate::game::{director as d, enemy::EnemyKind, gem, loadout, player, shop, weapon::WeaponKind};

macro_rules! section {
    ($name:ident { $($key:ident: $ty:ty = $value:expr; $doc:literal),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(pub $key: $ty),*
        }

        impl Default for $name {
            fn default() -> Self {
                Self { $($key: $value),* }
            }
        }

        impl $name {
            const COMMENTS: &'static [(&'static str, &'static str)] =
                &[$((stringify!($key), $doc)),*];
        }
    };
}

section!(Player {
    max_hp: f32 = 130.0; "Starting maximum health (HP)",
    regen: f32 = 0.4; "Starting regeneration (HP/s)",
    move_speed: f32 = 58.0; "Starting movement speed (world px/s)",
    armor: f32 = 0.0; "Starting flat damage reduction (HP per hit)",
    dash_cooldown: f32 = 2.0; "Starting dash cooldown (s)",
    dash_duration: f32 = DASH_SECS; "Uncharged dash travel duration (s)",
    hit_iframes: f32 = 0.9; "Immunity after a projectile, boss or hazard hit (s)",
    contact_iframes: f32 = 0.65; "Immunity after ordinary enemy contact (s)",
    magnet_radius: f32 = 22.0; "Starting pickup magnet radius (world px)",
});

section!(Xp {
    base: f64 = player::XP_BASE; "Base cost of one progression step (XP)",
    growth: f64 = player::XP_GROWTH; "Cost multiplier per successive progression step",
    steps_per_level: u32 = player::OLD_LEVELS_PER_LEVEL; "Progression steps charged per level after the opening",
    opening_steps: [u32; 3] = player::OPENING_OLD_LEVELS; "Progression steps charged for the first three levels",
    damage_per_level: f32 = player::LEVEL_DAMAGE_PER_LEVEL; "Additive weapon damage bonus per character level above one (fraction)",
    drop_chance: f32 = 1.0; "Chance that a defeated body drops its XP gem (0–1)",
    drop_mult: f32 = 1.0; "XP multiplier for gems dropped by defeated bodies",
    gem_tier_mult: [f32; 5] = [1.0; 5]; "XP multiplier by original drop tier (shard, chip, gem, crystal, core)",
    gem_tier_min: [u32; 5] = gem::GemTier::ALL.map(gem::GemTier::min_value); "XP thresholds for shard, chip, gem, crystal and core",
    merge_every: f32 = gem::MERGE_EVERY; "Gem merge sweep period (s)",
    merge_cell: f32 = gem::MERGE_CELL; "Gem merge grid cell width (world px)",
    merge_from: f32 = gem::MERGE_FROM; "Minimum merge distance (multiple of the widest view half-diagonal)",
    merge_soft_cap: usize = gem::GEM_SOFT_CAP; "Live gems above which far cells merge between periodic sweeps",
});

section!(Director {
    spawn_interval_mult: f32 = 1.0; "Multiplier on the buildup pulse interval (higher means less frequent)",
    opening_secs: f32 = d::OPENING_SECS; "Opening pulse length (curve seconds)",
    opening_interval_mult: f32 = d::OPENING_INTERVAL_MULT; "Pulse interval multiplier during the opening",
    bodies_per_second: [(f32, f32); 7] = d::BODIES_PER_SECOND; "Buildup spawn-rate knots [curve seconds, bodies/s]",
    live_budget: [(f32, usize); 7] = d::LIVE_BUDGET; "Live ordinary body budget steps [curve seconds, bodies]",
    elite_start: f32 = d::ELITE_START; "Earliest timed elite arrival (curve seconds)",
    elite_chance: [(f32, f32); 7] = d::ELITE_CHANCE; "Elite cadence knots [curve seconds, fraction of body rate]",
    elite_interval_mult: f32 = 1.0; "Multiplier on the interval between timed elite arrivals",
    elite_appointment_gap: f32 = d::ELITE_APPOINTMENT_GAP; "Quiet period around each named miniboss arrival (s on either side)",
    curve_speed: f32 = 1.0; "Curve seconds advanced per simulation second",
    build_hp: [(f32, f32); 7] = d::BUILD_HP; "Build HP correction knots [curve seconds, multiplier]",
    build_hp_level: [f32; 4] = d::BUILD_HP_LEVEL; "Build HP correction multipliers by campaign sector",
    level_hp_extra: [f32; 4] = d::LEVEL_HP_EXTRA; "Horde-only HP multipliers reached at the boss by sector",
    level_hp_extra_after: f32 = d::LEVEL_HP_EXTRA_AFTER; "Horde-only HP ramp grace period (s)",
    level_damage_top: [f32; 4] = d::LEVEL_DAMAGE_TOP; "Horde contact damage multipliers reached at the boss by sector",
    level_speed_top: [f32; 4] = d::LEVEL_SPEED_TOP; "Horde speed multipliers reached at the boss by sector",
    level_bonus_hp: [(f32, f32); 8] = d::LEVEL_BONUS_HP; "Character-level HP correction knots [curve seconds, multiplier]",
    horde_speed_top: f32 = d::HORDE_SPEED_TOP; "Top ordinary movement multiplier from the global speed ramp",
    horde_speed_ramp_end: f32 = d::HORDE_SPEED_RAMP_END; "Time to reach the top global horde speed (s)",
    boss_time: f32 = d::BOSS_TIME; "Boss arrival and end of buildup (simulation seconds)",
    cleanup_interval: f32 = d::CLEANUP_INTERVAL; "Seconds between horde cleanup pulses during the boss fight",
    cleanup_pulse: usize = d::CLEANUP_PULSE; "Bodies per cleanup pulse",
    cleanup_live_budget: usize = d::CLEANUP_LIVE_BUDGET; "Live ordinary body budget during the boss fight",
});

section!(Enemy {
    hp: f32 = 1.0; "Base HP before campaign and elite scaling",
    speed: f32 = 1.0; "Base steering speed (world px/s)",
    contact_damage: f32 = 1.0; "Base contact damage (HP before armor)",
    xp: u32 = 1; "XP in the defeated body's gem before global scaling",
    gold_chance: f32 = 0.0; "Base gold drop probability (0–1)",
    gold: u32 = 0; "Gold in a successful drop before global scaling",
});

section!(Enemies {
    hp_mult: f32 = 1.0; "Global ordinary enemy HP multiplier",
    contact_damage_mult: f32 = 1.0; "Global ordinary enemy contact damage multiplier",
    speed_mult: f32 = 1.0; "Global ordinary enemy movement speed multiplier",
    kinds: BTreeMap<EnemyKind, Enemy> = enemy_defaults(); "Per-kind ordinary enemy stats",
});

section!(Minibosses {
    times: [f32; 4] = std::array::from_fn(|i| d::MINIBOSS_TIMES[i]); "Scheduled arrivals by appointment (simulation seconds)",
    hp: [[f32; 4]; 4] = std::array::from_fn(|sector| std::array::from_fn(|slot| miniboss::appointment_hp_for(sector, slot))); "Actual starting HP [sector][appointment]",
    strike_damage: [f32; 4] = std::array::from_fn(miniboss::appointment_signature_damage); "Signature damage by appointment (HP before armor)",
    contact_damage: f32 = miniboss::MINIBOSS_CONTACT_DAMAGE; "Body contact damage (HP before armor)",
    xp: [u32; 4] = std::array::from_fn(miniboss::appointment_xp); "XP dropped by appointment",
});

section!(Boss {
    base_hp: f32 = 1.0; "Base HP before kind and campaign scaling",
    hp_scale: f32 = 1.0; "HP multiplier on sectors two to four",
    first_sector_hp_scale: f32 = 1.0; "HP multiplier on sector-one difficulty (including bare practice)",
    contact_damage_mult: f32 = 1.0; "Multiplier on grounded contact damage in every attack state",
});

section!(Bosses {
    hp_mult: f32 = 1.0; "Global named boss HP multiplier",
    build_level: [f32; 4] = d::BOSS_BUILD_LEVEL; "Boss-only build HP multipliers by campaign sector",
    contact_damage_mult: f32 = 1.0; "Global named boss grounded contact damage multiplier",
    kinds: BTreeMap<BossKind, Boss> = boss_defaults(); "Per-kind named boss stats",
});

section!(Loot {
    gold_chance_mult: f32 = 1.0; "Multiplier on ordinary enemy gold probability",
    gold_value_mult: f32 = 1.0; "Multiplier on enemy, elite, miniboss and boss gold values",
    elite_gold: u32 = shop::ELITE_GOLD; "Guaranteed extra gold for an elite kill before scaling",
    miniboss_gold: u32 = shop::MINIBOSS_GOLD; "Gold awarded with a miniboss chest before scaling",
    boss_gold: u32 = shop::BOSS_GOLD; "Gold awarded directly for a boss kill before scaling",
    pickup_chances: [f32; 3] = [0.012, 0.12, 1.0]; "Rare pickup probabilities for ordinary, large and legacy boss bodies",
    pickup_weights: [u32; 11] = [30, 1, 1, 18, 3, 1, 4, 14, 10, 9, 9]; "Ordered weights for Medkit, Shrink Draught, White Coat, Magnet, Purge, The Collar, Growth Serum, Bottle Cap, Wind-Up Mouse, Pocket Watch, Clean Socks",
    heal_pickup_fraction: f32 = 0.35; "Medkit healing (fraction of max HP)",
    chest_gold: u32 = loadout::CHEST_GOLD; "Chest fallback payout (gold)",
    chest_heal_fraction: f32 = loadout::CHEST_HEAL_FRAC; "Chest fallback healing (fraction of max HP)",
    chest_partner_stacks: u32 = 1; "Partner passive stacks required to evolve a max-rank weapon",
    bandage_heal_fraction: f32 = 0.25; "Level-up Bandage healing (fraction of max HP)",
    purse_gold: u32 = loadout::PURSE_GOLD; "Level-up Purse payout (gold)",
});

section!(Weapon {
    damage_mult: f32 = 1.0; "Multiplier on the weapon and its evolution's damage",
    cooldown_mult: f32 = 1.0; "Multiplier on recovery time between attacks (higher means slower)",
});

section!(Weapons {
    damage_mult: f32 = 1.0; "Multiplier on weapon damage from all weapons",
    kinds: BTreeMap<WeaponKind, Weapon> = WeaponKind::all().into_iter().map(|kind| (kind, Weapon::default())).collect(); "Per-weapon balance overrides",
});

fn enemy_defaults() -> BTreeMap<EnemyKind, Enemy> {
    EnemyKind::all().into_iter().map(|kind| {
        let (gold_chance, gold) = shop::gold_row(kind);
        (kind, Enemy {
            hp: kind.base_hp(),
            speed: kind.speed(),
            contact_damage: kind.damage(),
            xp: kind.xp(),
            gold_chance,
            gold,
        })
    }).collect()
}

fn boss_defaults() -> BTreeMap<BossKind, Boss> {
    BossKind::all().into_iter().map(|kind| (kind, Boss {
        base_hp: kind.base_hp(),
        hp_scale: kind.hp_scale(),
        first_sector_hp_scale: kind.hp_scale_for(0),
        contact_damage_mult: 1.0,
    })).collect()
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tuning {
    pub player: Player,
    pub xp: Xp,
    pub director: Director,
    pub enemies: Enemies,
    pub minibosses: Minibosses,
    pub bosses: Bosses,
    pub loot: Loot,
    pub weapons: Weapons,
}

/// Inserted once per world and shared by the facade and its loadout.
#[derive(Resource, Clone, Debug)]
pub struct RunTuning {
    pub values: Arc<Tuning>,
    pub changed: bool,
}

impl RunTuning {
    pub fn new(values: Tuning) -> Self {
        let changed = values != Tuning::default();
        Self { values: Arc::new(values), changed }
    }
}

impl Default for RunTuning {
    fn default() -> Self {
        Self { values: Arc::new(Tuning::default()), changed: false }
    }
}

impl std::ops::Deref for RunTuning {
    type Target = Tuning;

    fn deref(&self) -> &Tuning {
        &self.values
    }
}

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub run: RunTuning,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Report {
    pub path: Option<PathBuf>,
    pub hash: String,
    pub diff: BTreeMap<String, toml::Value>,
}

impl Selection {
    pub fn from_path(path: Option<PathBuf>) -> Result<Self> {
        let Some(path) = path else { return Ok(Self::default()) };
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading tuning file {}", path.display()))?;
        let values = Tuning::parse(&source)
            .with_context(|| format!("invalid tuning file {}", path.display()))?;
        Ok(Self { run: RunTuning::new(values), path: Some(path) })
    }

    pub fn report(&self) -> Report {
        let canonical = self.run.canonical();
        let current = readable_table(&canonical);
        let defaults = readable_table(&Tuning::default().canonical());
        let mut diff = BTreeMap::new();
        differences(&defaults, &current, "", &mut diff);
        let hash = canonical.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        Report { path: self.path.clone(), hash: format!("{hash:016x}"), diff }
    }
}

impl Tuning {
    pub fn parse(source: &str) -> Result<Self> {
        let overrides = source.parse::<Table>()?;
        let mut values = Table::try_from(Self::default())?;
        merge(&mut values, overrides, "")?;
        let tuning: Self = values.try_into()?;
        tuning.validate()?;
        Ok(tuning)
    }

    fn validate(&self) -> Result<()> {
        for (path, n) in [
            ("player.max_hp", self.player.max_hp),
            ("player.dash_duration", self.player.dash_duration),
            ("player.dash_cooldown", self.player.dash_cooldown),
            ("xp.merge_every", self.xp.merge_every),
            ("xp.merge_cell", self.xp.merge_cell),
            ("director.curve_speed", self.director.curve_speed),
            ("director.spawn_interval_mult", self.director.spawn_interval_mult),
            ("director.opening_interval_mult", self.director.opening_interval_mult),
            ("director.elite_interval_mult", self.director.elite_interval_mult),
            ("director.cleanup_interval", self.director.cleanup_interval),
            ("director.horde_speed_ramp_end", self.director.horde_speed_ramp_end),
            ("director.boss_time", self.director.boss_time),
        ] {
            if !n.is_finite() || n <= 0.0 {
                bail!("{path} must be finite and positive");
            }
        }
        if !self.xp.base.is_finite() || self.xp.base <= 0.0 {
            bail!("xp.base must be finite and positive");
        }
        if !self.xp.growth.is_finite() || self.xp.growth < 1.0 {
            bail!("xp.growth must be finite and at least 1");
        }
        if std::iter::once(&self.xp.steps_per_level).chain(&self.xp.opening_steps)
            .any(|n| !(1..=1000).contains(n))
        {
            bail!("xp.steps_per_level and xp.opening_steps require steps in 1..=1000");
        }
        if self.xp.gem_tier_min.windows(2).any(|w| w[0] > w[1]) {
            bail!("xp.gem_tier_min must be in ascending order");
        }
        let director = &self.director;
        for (path, knots) in [
            ("director.build_hp", &director.build_hp[..]),
            ("director.level_bonus_hp", &director.level_bonus_hp[..]),
            ("director.bodies_per_second", &director.bodies_per_second[..]),
            ("director.elite_chance", &director.elite_chance[..]),
        ] {
            if knots.windows(2).any(|rows| rows[0].0 > rows[1].0) {
                bail!("{path} times must be in ascending order");
            }
        }
        if director.live_budget.windows(2).any(|rows| rows[0].0 > rows[1].0) {
            bail!("director.live_budget times must be in ascending order");
        }
        if self.xp.drop_chance > 1.0 {
            bail!("xp.drop_chance must be in 0..=1");
        }
        if self.enemies.kinds.values().any(|row| row.gold_chance > 1.0) {
            bail!("enemies.kinds gold_chance must be in 0..=1");
        }
        if self.loot.pickup_chances.iter().any(|&chance| chance > 1.0) {
            bail!("loot.pickup_chances must be in 0..=1");
        }
        if self.minibosses.times.windows(2).any(|w| w[0] >= w[1]) {
            bail!("minibosses.times must be strictly increasing");
        }
        if self.loot.pickup_weights.iter().all(|&weight| weight == 0) {
            bail!("loot.pickup_weights must give at least one pickup positive weight");
        }
        Ok(())
    }

    fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialize effective tuning")
    }

    pub fn commented_defaults() -> String {
        let values = readable_table(&Self::default().canonical());
        let mut output = String::from("# Rat Run balance defaults. Omitted fields use the shipped game values.\n");
        for (section, comments) in [
            ("player", Player::COMMENTS), ("xp", Xp::COMMENTS), ("director", Director::COMMENTS),
            ("enemies", Enemies::COMMENTS), ("minibosses", Minibosses::COMMENTS),
            ("bosses", Bosses::COMMENTS), ("loot", Loot::COMMENTS), ("weapons", Weapons::COMMENTS),
        ] {
            let table = values[section].as_table().expect("tuning section");
            write_section(&mut output, section, table, comments);
            if let Some(kinds) = table.get("kinds").and_then(Value::as_table) {
                let kind_comments = match section {
                    "enemies" => Enemy::COMMENTS,
                    "bosses" => Boss::COMMENTS,
                    _ => Weapon::COMMENTS,
                };
                for (kind, values) in kinds {
                    write_section(&mut output, &format!("{section}.kinds.{kind}"),
                        values.as_table().expect("kind table"), kind_comments);
                }
            }
        }
        output
    }
}

// Widen f32 numbers only after formatting them at their original precision.
fn readable_table(canonical: &[u8]) -> Table {
    serde_json::from_slice(canonical).expect("convert numeric tuning tables")
}

fn write_section(output: &mut String, path: &str, values: &Table, comments: &[(&str, &str)]) {
    use std::fmt::Write;

    writeln!(output, "\n[{path}]").expect("write to string");
    for &(key, comment) in comments {
        match &values[key] {
            Value::Table(_) => writeln!(output, "# {comment} follow below."),
            value => writeln!(output, "# {comment}. Default {value}.\n{key} = {value}"),
        }.expect("write to string");
    }
}

fn merge(defaults: &mut Table, overrides: Table, prefix: &str) -> Result<()> {
    for (key, value) in overrides {
        let path = format!("{prefix}{key}");
        let old = defaults.get_mut(&key).with_context(|| format!("unknown key {path}"))?;
        match (old, value) {
            (Value::Table(base), Value::Table(extra)) => merge(base, extra, &format!("{path}."))?,
            (Value::Table(_), _) => bail!("{path} must be a table"),
            (old, mut value) => {
                numbers(&mut value, &path)?;
                *old = value;
            }
        }
    }
    Ok(())
}

fn numbers(value: &mut Value, path: &str) -> Result<()> {
    match value {
        Value::Float(n) if !n.is_finite() || !(0.0..=1.0e12).contains(n) =>
            bail!("{path} must be finite, nonnegative and at most 1e12"),
        Value::Integer(n) if !(0..=1_000_000_000_000).contains(n) =>
            bail!("{path} must be nonnegative and at most 1e12"),
        Value::Float(n) if *n == 0.0 => *n = 0.0,
        Value::Array(values) => {
            for (i, value) in values.iter_mut().enumerate() {
                numbers(value, &format!("{path}[{i}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn differences(defaults: &Table, current: &Table, prefix: &str, diff: &mut BTreeMap<String, Value>) {
    for (key, value) in current {
        let path = format!("{prefix}{key}");
        let default = &defaults[key];
        match (default, value) {
            (Value::Table(base), Value::Table(active)) => differences(base, active, &format!("{path}."), diff),
            (base, active) if base != active => {
                diff.insert(path, active.clone());
            }
            _ => {}
        }
    }
}

/// Strip the shared CLI flags before dispatching to each mode's parser.
pub fn cli(args: &mut Vec<String>) -> Result<Option<Selection>> {
    let mut path = None;
    let mut dump = None;
    let mut i = 0;
    while i < args.len() {
        let target = match args[i].as_str() {
            "--tuning" => &mut path,
            "--tuning-dump" => &mut dump,
            _ => { i += 1; continue }
        };
        let flag = &args[i];
        if target.is_some() {
            bail!("{flag} may only be used once");
        }
        let value = args.get(i + 1).filter(|arg| !arg.starts_with("--"))
            .with_context(|| format!("{flag} requires a TOML file path"))?;
        *target = Some(PathBuf::from(value));
        args.drain(i..i + 2);
    }
    if let Some(dump) = dump {
        if path.is_some() {
            bail!("choose either --tuning or --tuning-dump");
        }
        std::fs::write(&dump, Tuning::commented_defaults())
            .with_context(|| format!("writing tuning defaults to {}", dump.display()))?;
        return Ok(None);
    }
    Selection::from_path(path.or_else(|| std::env::var_os("RATRUN_TUNING").map(PathBuf::from))).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_tables_report_changes_and_reject_typos() {
        let tuning = Tuning::parse("[enemies]\nhp_mult = 1.5\n[enemies.kinds.Pillbug]\nxp = 21").unwrap();
        assert_eq!(tuning.enemies.hp_mult, 1.5);
        assert_eq!(tuning.enemies.kinds[&EnemyKind::Pillbug].xp, 21);
        assert_eq!(tuning.player, Player::default());
        let typo = Tuning::parse("[enemies.kinds.Pillbug]\nxxp = 21").unwrap_err().to_string();
        assert!(typo.contains("enemies.kinds.Pillbug.xxp"));
        let selection = Selection { run: RunTuning { values: Arc::new(tuning), changed: true }, path: None };
        assert_eq!(selection.report().diff.keys().collect::<Vec<_>>(),
            ["enemies.hp_mult", "enemies.kinds.Pillbug.xp"]);
    }

    #[test]
    fn tuning_is_local_to_each_world() {
        use crate::game::{level::Theme, world::World};
        use crate::math::Vec2;

        let run = RunTuning::new(Tuning::parse(
            "[player]\nmax_hp = 200.0\n[xp]\nbase = 12.0\n[enemies]\nhp_mult = 1.5",
        ).unwrap());
        let mut baseline = World::new(Theme::Sewer, 16);
        let mut tuned = World::with_tuning(Theme::Sewer, 16, run);
        assert_eq!(baseline.player.xp_to_next, 6);
        assert_eq!(tuned.player.xp_to_next, 12);
        assert_eq!(tuned.player.max_hp(), 200.0);
        let kind = EnemyKind::Pillbug;
        let standard = baseline.spawn_enemy(kind, Vec2::ZERO, 1.0);
        let hardened = tuned.spawn_enemy(kind, Vec2::ZERO, 1.0);
        assert_eq!(baseline.ecs.get::<crate::game::enemy::Enemy>(standard).unwrap().hp, kind.base_hp());
        assert_eq!(tuned.ecs.get::<crate::game::enemy::Enemy>(hardened).unwrap().hp, kind.base_hp() * 1.5);
    }

    #[test]
    fn dumped_defaults_round_trip() {
        assert_eq!(Tuning::parse(&Tuning::commented_defaults()).unwrap(), Tuning::default());
    }
}
