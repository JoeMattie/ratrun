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
    ($name:ident { $($(#[$attr:meta])* $key:ident: $ty:ty = $value:expr; $doc:literal),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($(#[$attr])* pub $key: $ty),*
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

section!(Campaign {
    carry: bool = false; "Carry a selected build between sectors and use the original campaign balance",
});

section!(Player {
    max_hp: f32 = 158.954; "Starting maximum health (HP)",
    regen: f32 = 0.46474266; "Starting regeneration (HP/s)",
    move_speed: f32 = 58.0; "Starting movement speed (world px/s)",
    armor: f32 = 0.0; "Starting flat damage reduction (HP per hit)",
    dash_cooldown: f32 = 2.0; "Starting dash cooldown (s)",
    dash_duration: f32 = DASH_SECS; "Uncharged dash travel duration (s)",
    hit_iframes: f32 = 1.189484; "Immunity after a projectile, boss or hazard hit (s)",
    contact_iframes: f32 = 1.1170288; "Immunity after ordinary enemy contact (s)",
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
    build_hp_level: [f32; 4] = [0.6262416, 2.6255746, 3.595_260_1, 2.2739563]; "Build HP correction multipliers by campaign sector",
    level_hp_extra: [f32; 4] = d::LEVEL_HP_EXTRA; "Horde-only HP multipliers reached at the boss by sector",
    fresh_build_hp_level: [f32; 3] = [0.64, 0.66, 0.68]; "Build HP correction by sector with campaign carry disabled",
    fresh_level_hp_extra: [f32; 3] = [1.23, 1.26, 1.29]; "Horde-only HP at the boss by sector with campaign carry disabled",
    level_hp_extra_after: f32 = d::LEVEL_HP_EXTRA_AFTER; "Horde-only HP ramp grace period (s)",
    level_damage_top: [f32; 4] = [0.61121884, 1.0319662, 1.2344486, 2.0158477]; "Horde contact damage multipliers reached at the boss by sector",
    level_speed_top: [f32; 4] = d::LEVEL_SPEED_TOP; "Horde speed multipliers reached at the boss by sector",
    level_bonus_hp: [(f32, f32); 8] = d::LEVEL_BONUS_HP; "Character-level HP correction knots [curve seconds, multiplier]",
    horde_speed_top: f32 = d::HORDE_SPEED_TOP; "Top ordinary movement multiplier from the global speed ramp",
    horde_speed_ramp_end: f32 = d::HORDE_SPEED_RAMP_END; "Time to reach the top global horde speed (s)",
    boss_time: f32 = d::BOSS_TIME; "Boss arrival and end of buildup (simulation seconds)",
    cleanup_interval: f32 = d::CLEANUP_INTERVAL; "Seconds between horde cleanup pulses during the boss fight",
    cleanup_pulse: usize = d::CLEANUP_PULSE; "Bodies per cleanup pulse",
    cleanup_live_budget: usize = d::CLEANUP_LIVE_BUDGET; "Live ordinary body budget during the boss fight",
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sectors: BTreeMap<String, DirectorSector> = director_sector_defaults(); "Overrides by campaign sector (1–4, practice uses 1)",
    pressure_beats: bool = true; "Enable the buildup pressure beats and the Sewer opening grace",
    pressure_starts: [f32; 2] = [198.854, 694.324]; "Surge start times in simulation seconds",
    pressure_lengths: [f32; 2] = [104.285, 101.29]; "Surge lengths in seconds",
    pressure_recovery_lengths: [f32; 2] = [268.076, 68.965]; "Recovery lengths in seconds, immediately after each surge",
    pressure_warning_lead: f32 = 5.0; "Seconds between each warning and surge",
    pressure_rate_bonus: f32 = 0.24262717; "Extra ordinary pulse frequency during a surge, balanced against its recovery",
    pressure_budget_bonus: f32 = 0.33028942; "Extra live ordinary budget as a fraction during a surge",
    pressure_contact_bonus: f32 = 1.1266887; "Extra ordinary contact damage as a fraction during a surge",
    pressure_recovery_budget_cut: f32 = 0.19937819; "Fraction of the live ordinary budget removed during recovery",
    pressure_recovery_heal: f32 = 0.67160283; "Fraction of non-heal rare finds converted to Medkits during recovery",
    pressure_sector_scale: [f32; 4] = [0.4145341, 1.1853494, 1.3945287, 1.603708]; "Beat strength by campaign sector. Zero skips beats and warnings",
    fresh_pressure_sector_scale: [f32; 3] = [0.88, 0.95, 0.80]; "Beat strength in sectors two to four with campaign carry disabled",
    pressure_sewer_opening_secs: f32 = 167.557_22; "Length of Sewer opening contact grace in seconds",
    pressure_sewer_opening_cut: f32 = 0.26234644; "Fraction of ordinary contact damage removed during Sewer opening grace",
});

pub(crate) const SECTOR_KEYS: [&str; 4] = ["1", "2", "3", "4"];

section!(DirectorSector {
    #[serde(skip_serializing_if = "Option::is_none")]
    live_budget: Option<[(f32, usize); 7]> = None; "Live ordinary body budget steps [curve seconds, bodies]",
    #[serde(skip_serializing_if = "Option::is_none")]
    bodies_per_second: Option<[(f32, f32); 7]> = None; "Spawn-rate knots used for pulse size and elite cadence [curve seconds, bodies/s]",
    #[serde(skip_serializing_if = "Option::is_none")]
    spawn_interval: Option<[(f32, f32); 7]> = None; "Base pulse interval knots [curve seconds, seconds]",
    #[serde(skip_serializing_if = "Option::is_none")]
    elite_start: Option<f32> = None; "Earliest timed elite arrival (curve seconds)",
    #[serde(skip_serializing_if = "Option::is_none")]
    elite_chance: Option<[(f32, f32); 7]> = None; "Elite cadence knots [curve seconds, fraction of body rate]",
    #[serde(skip_serializing_if = "Option::is_none")]
    elite_interval_mult: Option<f32> = None; "Multiplier on the interval between timed elite arrivals, replacing the shared multiplier",
});

fn director_sector_defaults() -> BTreeMap<String, DirectorSector> {
    let rows = [
        DirectorSector {
            live_budget: Some([
                (0.0, 15), (60.0, 35), (180.0, 59), (360.0, 95),
                (540.0, 124), (720.0, 152), (900.0, 152),
            ]),
            spawn_interval: Some([
                (0.0, 1.7921361), (60.0, 1.717_463_7), (180.0, 1.568_119_2),
                (360.0, 1.3441021), (540.0, 1.1200851), (720.0, 0.8960681),
                (900.0, 0.672_051_1),
            ]),
            elite_interval_mult: Some(1.2033988),
            ..DirectorSector::default()
        },
        DirectorSector {
            live_budget: Some([
                (0.0, 23), (60.0, 54), (180.0, 93), (360.0, 148),
                (540.0, 193), (720.0, 239), (900.0, 239),
            ]),
            spawn_interval: Some([
                (0.0, 2.186611), (60.0, 2.0955021), (180.0, 1.9132845),
                (360.0, 1.6399582), (540.0, 1.366_631_9), (720.0, 1.093_305_5),
                (900.0, 0.8199791),
            ]),
            elite_interval_mult: Some(1.3331294),
            ..DirectorSector::default()
        },
        DirectorSector {
            live_budget: Some([
                (0.0, 24), (60.0, 55), (180.0, 94), (360.0, 150),
                (540.0, 196), (720.0, 242), (900.0, 242),
            ]),
            spawn_interval: Some([
                (0.0, 1.2603316), (60.0, 1.2078178), (180.0, 1.1027902),
                (360.0, 0.9452487), (540.0, 0.78770727), (720.0, 0.6301658),
                (900.0, 0.47262436),
            ]),
            elite_interval_mult: Some(1.336299),
            ..DirectorSector::default()
        },
        DirectorSector {
            live_budget: Some([
                (0.0, 23), (60.0, 53), (180.0, 91), (360.0, 146),
                (540.0, 190), (720.0, 234), (900.0, 234),
            ]),
            spawn_interval: Some([
                (0.0, 1.3493907), (60.0, 1.293_166_2), (180.0, 1.1807169),
                (360.0, 1.012043), (540.0, 0.8433692), (720.0, 0.6746954),
                (900.0, 0.5060215),
            ]),
            elite_interval_mult: Some(1.3003603),
            ..DirectorSector::default()
        },
    ];
    SECTOR_KEYS.into_iter().map(str::to_owned).zip(rows).collect()
}

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
    speed_mult: f32 = 0.85838351; "Global ordinary enemy movement speed multiplier",
    kinds: BTreeMap<EnemyKind, Enemy> = enemy_defaults(); "Per-kind ordinary enemy stats",
});

section!(Minibosses {
    times: [f32; 4] = std::array::from_fn(|i| d::MINIBOSS_TIMES[i]); "Scheduled arrivals by appointment (simulation seconds)",
    hp: [[f32; 4]; 4] = std::array::from_fn(|sector| std::array::from_fn(|slot| miniboss::appointment_hp_for(sector, slot))); "Actual starting HP [sector][appointment]",
    fresh_hp: [[f32; 4]; 3] = [
        [300.0, 1550.0, 2800.0, 4250.0],
        [330.0, 1600.0, 2900.0, 4350.0],
        [360.0, 1650.0, 3000.0, 4450.0],
    ]; "Actual starting HP [sector][appointment] with campaign carry disabled",
    strike_damage: [f32; 4] = std::array::from_fn(miniboss::appointment_signature_damage); "Signature damage by appointment (HP before armor)",
    contact_damage: f32 = miniboss::MINIBOSS_CONTACT_DAMAGE; "Body contact damage (HP before armor)",
    xp: [u32; 4] = std::array::from_fn(miniboss::appointment_xp); "XP dropped by appointment",
});

section!(Boss {
    base_hp: f32 = 1.0; "Base HP before kind and campaign scaling",
    hp_scale: f32 = 1.0; "HP multiplier on sectors two to four",
    first_sector_hp_scale: f32 = 1.0; "HP multiplier on sector-one difficulty (including bare practice)",
    contact_damage_mult: f32 = 1.0; "Multiplier on grounded contact damage in every attack state",
    #[serde(skip_serializing_if = "Option::is_none")]
    attack_damage_mult: Option<f32> = None; "Damage multiplier on timed boss hazards",
    #[serde(skip_serializing_if = "Option::is_none")]
    tell_time_mult: Option<f32> = None; "Multiplier on timed hazard warnings and the Cat's raised-paw tell",
    #[serde(skip_serializing_if = "Option::is_none")]
    swipe_damage: Option<f32> = None; "Cat only: base swipe hazard damage before attack_damage_mult (HP before armor)",
    #[serde(skip_serializing_if = "Option::is_none")]
    swipe_windup_mult: Option<f32> = None; "Cat only: multiplier on the raised-paw tell in both phases, compounded with tell_time_mult",
    #[serde(skip_serializing_if = "Option::is_none")]
    swipe_chain_gap: Option<f32> = None; "Cat only: extra recovery between the first swipe and the chained windup (s)",
});

section!(Bosses {
    hp_mult: f32 = 0.76265487; "Global named boss HP multiplier",
    build_level: [f32; 4] = [0.9580967, 0.65357122, 0.380501, 0.77749956]; "Boss-only build HP multipliers by campaign sector",
    fresh_build_level: [f32; 3] = [1.18, 1.08, 1.08]; "Boss-only build HP multipliers with campaign carry disabled",
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
    BossKind::all().into_iter().map(|kind| {
        let rebalanced_hazards = matches!(
            kind,
            BossKind::Cat | BossKind::HorseshoeCrab | BossKind::Broom | BossKind::MegaRat
                | BossKind::Mower | BossKind::Cobra | BossKind::Magpie | BossKind::Hawk
        );
        (kind, Boss {
            base_hp: kind.base_hp(),
            hp_scale: kind.hp_scale(),
            first_sector_hp_scale: kind.hp_scale_for(0),
            attack_damage_mult: rebalanced_hazards.then_some(1.1779899),
            tell_time_mult: rebalanced_hazards.then_some(1.333_314_4),
            swipe_damage: (kind == BossKind::Cat).then_some(17.509764),
            swipe_windup_mult: (kind == BossKind::Cat).then_some(0.9790898),
            ..Boss::default()
        })
    }).collect()
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tuning {
    pub campaign: Campaign,
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
    #[cfg(test)]
    pub(crate) fn with_base_boss_hazards(kind: BossKind) -> Self {
        // Controller tests pin the art's unscaled hazard timing and damage.
        let mut values = Tuning::default();
        let boss = values.bosses.kinds.get_mut(&kind).expect("boss tuning row");
        boss.attack_damage_mult = None;
        boss.tell_time_mult = None;
        boss.swipe_damage = None;
        boss.swipe_windup_mult = None;
        Self::new(values)
    }

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
    #[cfg(not(target_arch = "wasm32"))]
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
    pub fn miniboss_hp(&self, level: usize, appointment: usize) -> f32 {
        let level = level.min(3);
        let appointment = appointment.min(3);
        if level == 0 || self.campaign.carry {
            self.minibosses.hp[level][appointment]
        } else {
            self.minibosses.fresh_hp[level - 1][appointment]
        }
    }

    pub fn parse(source: &str) -> Result<Self> {
        let overrides = source.parse::<Table>()?;
        let mut values = Table::try_from(Self::default())?;
        let schema = Table::try_from(Self::schema())?;
        merge(&mut values, &schema, overrides, "")?;
        let mut tuning: Self = values.try_into()?;
        tuning.validate()?;
        tuning.omit_inherited_values();
        Ok(tuning)
    }

    /// Example values for optional keys, also used to validate their paths.
    fn schema() -> Self {
        let mut schema = Self::default();
        let director = &mut schema.director;
        for sector in director.sectors.values_mut() {
            sector.bodies_per_second.get_or_insert(director.bodies_per_second);
            sector.elite_start.get_or_insert(director.elite_start);
            sector.elite_chance.get_or_insert(director.elite_chance);
        }
        for boss in schema.bosses.kinds.values_mut() {
            boss.attack_damage_mult.get_or_insert(1.0);
            boss.tell_time_mult.get_or_insert(1.0);
        }
        let cat = schema.bosses.kinds.get_mut(&BossKind::Cat).expect("Cat tuning row");
        cat.swipe_chain_gap = Some(0.0);
        schema
    }

    fn omit_inherited_values(&mut self) {
        let Director {
            bodies_per_second, elite_start, elite_chance, sectors, ..
        } = &mut self.director;
        for row in sectors.values_mut() {
            // Budget, spawn interval and elite interval have shipped sector rows.
            // An explicit shared value there can override the shipped row.
            omit_if_default(&mut row.bodies_per_second, *bodies_per_second);
            omit_if_default(&mut row.elite_start, *elite_start);
            omit_if_default(&mut row.elite_chance, *elite_chance);
        }
        let shipped_bosses = boss_defaults();
        for (kind, boss) in &mut self.bosses.kinds {
            if shipped_bosses[kind].attack_damage_mult.is_none() {
                omit_if_default(&mut boss.attack_damage_mult, 1.0);
                omit_if_default(&mut boss.tell_time_mult, 1.0);
            }
            omit_if_default(&mut boss.swipe_chain_gap, 0.0);
        }
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
        if director.pressure_recovery_budget_cut > 1.0
            || director.pressure_recovery_heal > 1.0
            || director.pressure_sewer_opening_cut >= 1.0
        {
            bail!("director pressure cuts and heal chance must be fractions (opening cut strictly below 1)");
        }
        if director.pressure_beats {
            let lead = director.pressure_warning_lead;
            let mut previous_end = 0.0_f32;
            for i in 0..2 {
                let start = director.pressure_starts[i];
                let surge = director.pressure_lengths[i];
                let recovery = director.pressure_recovery_lengths[i];
                if start - lead < previous_end || surge <= 0.0 || recovery <= 0.0 {
                    bail!("director pressure windows must have positive lengths and must not overlap, including their warnings");
                }
                previous_end = start + surge + recovery;
            }
            if previous_end > director.boss_time {
                bail!("director pressure windows must end by director.boss_time");
            }
        }
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
        for (sector, row) in &director.sectors {
            let path = format!("director.sectors.{sector}");
            if row.live_budget.is_some_and(|steps| steps.windows(2).any(|w| w[0].0 > w[1].0)) {
                bail!("{path}.live_budget times must be in ascending order");
            }
            for (key, knots) in [
                ("bodies_per_second", row.bodies_per_second),
                ("elite_chance", row.elite_chance),
            ] {
                if knots.is_some_and(|knots| knots.windows(2).any(|w| w[0].0 > w[1].0)) {
                    bail!("{path}.{key} times must be in ascending order");
                }
            }
            if let Some(knots) = row.spawn_interval {
                if knots.windows(2).any(|w| w[0].0 >= w[1].0) {
                    bail!("{path}.spawn_interval times must be strictly increasing");
                }
                if knots.iter().any(|&(_, interval)| interval <= 0.0) {
                    bail!("{path}.spawn_interval values must be positive");
                }
            }
            if row.elite_interval_mult == Some(0.0) {
                bail!("{path}.elite_interval_mult must be positive");
            }
        }
        for (kind, boss) in &self.bosses.kinds {
            for (key, value) in [
                ("attack_damage_mult", boss.attack_damage_mult),
                ("tell_time_mult", boss.tell_time_mult),
                ("swipe_damage", boss.swipe_damage),
                ("swipe_windup_mult", boss.swipe_windup_mult),
            ] {
                if value == Some(0.0) {
                    bail!("bosses.kinds.{kind:?}.{key} must be positive");
                }
            }
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
        let defaults = readable_table(&Self::default().canonical());
        let schema = readable_table(&Self::schema().canonical());
        let mut output = String::from("# Rat Run balance defaults. Omitted fields use the shipped game values.\n");
        for (section, comments) in [
            ("campaign", Campaign::COMMENTS), ("player", Player::COMMENTS), ("xp", Xp::COMMENTS), ("director", Director::COMMENTS),
            ("enemies", Enemies::COMMENTS), ("minibosses", Minibosses::COMMENTS),
            ("bosses", Bosses::COMMENTS), ("loot", Loot::COMMENTS), ("weapons", Weapons::COMMENTS),
        ] {
            let table = schema[section].as_table().expect("tuning section");
            let base = defaults[section].as_table().expect("default tuning section");
            write_section(&mut output, section, table, Some(base), comments);
            if let Some(sectors) = table.get("sectors").and_then(Value::as_table) {
                for (sector, values) in sectors {
                    let default = base["sectors"].get(sector.as_str()).and_then(Value::as_table)
                        .expect("default sector table");
                    write_section(&mut output, &format!("{section}.sectors.{sector}"),
                        values.as_table().expect("sector table"), Some(default), DirectorSector::COMMENTS);
                }
            }
            if let Some(kinds) = table.get("kinds").and_then(Value::as_table) {
                let kind_comments = match section {
                    "enemies" => Enemy::COMMENTS,
                    "bosses" => Boss::COMMENTS,
                    _ => Weapon::COMMENTS,
                };
                for (kind, values) in kinds {
                    let default = base["kinds"].get(kind.as_str()).and_then(Value::as_table)
                        .expect("default kind table");
                    write_section(&mut output, &format!("{section}.kinds.{kind}"),
                        values.as_table().expect("kind table"), Some(default), kind_comments);
                }
            }
        }
        output
    }
}

fn omit_if_default<T: PartialEq>(value: &mut Option<T>, default: T) {
    if *value == Some(default) {
        *value = None;
    }
}

// Widen f32 numbers only after formatting them at their original precision.
fn readable_table(canonical: &[u8]) -> Table {
    serde_json::from_slice(canonical).expect("convert numeric tuning tables")
}

fn write_section(
    output: &mut String,
    path: &str,
    values: &Table,
    defaults: Option<&Table>,
    comments: &[(&str, &str)],
) {
    use std::fmt::Write;

    writeln!(output, "\n[{path}]").expect("write to string");
    for &(key, comment) in comments {
        let Some(value) = values.get(key) else { continue };
        match value {
            Value::Table(_) => writeln!(output, "# {comment} follow below."),
            value if defaults.is_some_and(|base| base.contains_key(key)) => {
                writeln!(output, "# {comment}. Default {value}.\n{key} = {value}")
            }
            value => writeln!(output, "# {comment}. Optional override.\n# {key} = {value}"),
        }.expect("write to string");
    }
}

fn merge(defaults: &mut Table, schema: &Table, overrides: Table, prefix: &str) -> Result<()> {
    for (key, value) in overrides {
        let path = format!("{prefix}{key}");
        let expected = schema.get(&key).with_context(|| format!("unknown key {path}"))?;
        match (expected, value) {
            (Value::Table(shape), Value::Table(extra)) => {
                let base = defaults.entry(key).or_insert_with(|| Value::Table(Table::new()));
                merge(base.as_table_mut().expect("default table"), shape, extra, &format!("{path}."))?;
            }
            (Value::Table(_), _) => bail!("{path} must be a table"),
            (_, mut value) => {
                numbers(&mut value, &path)?;
                defaults.insert(key, value);
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
        match (defaults.get(key), value) {
            (Some(Value::Table(base)), Value::Table(active)) => differences(base, active, &format!("{path}."), diff),
            (None, Value::Table(active)) => differences(&Table::new(), active, &format!("{path}."), diff),
            (base, active) if base != Some(active) => {
                diff.insert(path, active.clone());
            }
            _ => {}
        }
    }
}

/// Strip the shared CLI flags before dispatching to each mode's parser.
#[cfg(not(target_arch = "wasm32"))]
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
    fn campaign_carry_selects_the_matching_hp_tables() {
        let fresh = Tuning::default();
        let legacy = Tuning::parse("[campaign]\ncarry = true").unwrap();
        assert!(!fresh.campaign.carry && legacy.campaign.carry);
        let at = fresh.director.boss_time;
        assert_eq!(fresh.horde_hp_mult_for(0, at), legacy.horde_hp_mult_for(0, at));
        assert_eq!(fresh.boss_hp_mult_for(0), legacy.boss_hp_mult_for(0));
        assert_eq!(fresh.pressure_scale_for(0), legacy.pressure_scale_for(0));
        assert_eq!(fresh.miniboss_hp(0, 0), legacy.miniboss_hp(0, 0));
        for level in 1..4 {
            assert!(fresh.horde_hp_mult_for(level, at) < legacy.horde_hp_mult_for(level, at));
            assert!(fresh.boss_hp_mult_for(level) < legacy.boss_hp_mult_for(level));
            assert!(fresh.pressure_scale_for(level) < legacy.pressure_scale_for(level));
            for appointment in 0..4 {
                assert!(fresh.miniboss_hp(level, appointment) < legacy.miniboss_hp(level, appointment));
            }
        }
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
        assert_eq!(baseline.player.xp_to_next, 5);
        assert_eq!(tuned.player.xp_to_next, 12);
        assert_eq!(tuned.player.max_hp(), 200.0);
        let kind = EnemyKind::Pillbug;
        let standard = baseline.spawn_enemy(kind, Vec2::ZERO, 1.0);
        let hardened = tuned.spawn_enemy(kind, Vec2::ZERO, 1.0);
        assert_eq!(baseline.ecs.get::<crate::game::enemy::Enemy>(standard).unwrap().hp, kind.base_hp());
        let speed = baseline.ecs.get::<crate::game::enemy::Enemy>(standard).unwrap().speed();
        assert_eq!(speed, kind.speed() * baseline.tuning().enemies.speed_mult);
        assert!(!baseline.tuning().changed);
        assert_eq!(tuned.ecs.get::<crate::game::enemy::Enemy>(hardened).unwrap().hp, kind.base_hp() * 1.5);
    }

    #[test]
    fn dumped_defaults_round_trip() {
        let dump = Tuning::commented_defaults();
        assert!(dump.contains("[director.sectors.4]"));
        assert!(dump.contains("\nspawn_interval = "));
        assert!(dump.contains("\nswipe_damage = "));
        for source in ["", &dump] {
            let values = Tuning::parse(source).unwrap();
            assert_eq!(values, Tuning::default());
            assert!(!RunTuning::new(values).changed);
        }
    }

    #[test]
    fn inherited_overrides_keep_the_shared_tuning_hash() {
        let shared = "[director]\nelite_start = 240";
        let explicit = format!("{shared}\n[director.sectors.1]\nelite_start = 240\n\
            [bosses.kinds.Lantern]\nattack_damage_mult = 1\ntell_time_mult = 1\n\
            [bosses.kinds.Cat]\nswipe_chain_gap = 0");
        let report = |source| Selection {
            run: RunTuning::new(Tuning::parse(source).unwrap()), path: None,
        }.report();
        assert_eq!(report(shared).hash, report(&explicit).hash);
        assert_eq!(report(shared).diff, report(&explicit).diff);
    }

    #[test]
    fn explicit_neutral_values_override_shipped_sector_and_boss_settings() {
        let values = Tuning::parse(
            "[director.sectors.1]\nelite_interval_mult = 1\n\
             [bosses.kinds.Cat]\nattack_damage_mult = 1\ntell_time_mult = 1\n\
             swipe_damage = 22\nswipe_windup_mult = 1",
        ).unwrap();
        let selection = Selection { run: RunTuning::new(values), path: None };
        assert!(selection.run.changed);
        assert_eq!(selection.report().diff.keys().map(String::as_str).collect::<Vec<_>>(), [
            "bosses.kinds.Cat.attack_damage_mult",
            "bosses.kinds.Cat.swipe_damage",
            "bosses.kinds.Cat.swipe_windup_mult",
            "bosses.kinds.Cat.tell_time_mult",
            "director.sectors.1.elite_interval_mult",
        ]);
    }

    #[test]
    fn optional_overrides_report_leaf_paths_and_reject_unknown_rows() {
        let tuning = Tuning::parse(
            "[director.sectors.2]\nelite_interval_mult = 2.0\n[bosses.kinds.Cat]\nswipe_chain_gap = 0.25",
        ).unwrap();
        let report = Selection { run: RunTuning::new(tuning), path: None }.report();
        assert_eq!(report.diff.keys().map(String::as_str).collect::<Vec<_>>(),
            ["bosses.kinds.Cat.swipe_chain_gap", "director.sectors.2.elite_interval_mult"]);
        for (source, path) in [
            ("[director.sectors.5]\nelite_start = 0", "director.sectors.5"),
            ("[director.sectors.1]\nelite_intervl_mult = 1", "director.sectors.1.elite_intervl_mult"),
            ("[bosses.kinds.Lantern]\nswipe_damage = 12", "bosses.kinds.Lantern.swipe_damage"),
            ("[bosses.kinds.Cat]\nswipe_windup_mult = 0", "bosses.kinds.Cat.swipe_windup_mult"),
        ] {
            assert!(Tuning::parse(source).unwrap_err().to_string().contains(path));
        }
    }
}
