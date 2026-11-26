//! Enemies: kinds, per-kind stats, and steering behavior toward the player.
//!
//! `EnemyKind` is append-only and `EnemyKind::all()` lists every kind: the 56
//! ordinary bodies the floor rosters own (`game/roster.rs`), one **guest**
//! (the Drake, which no roster slot carries) and the legacy `Boss` body: 58.
//! Stats, name, colour and XP key on the kind itself; behaviour keys on
//! [`EnemyKind::archetype`], the existing body whose steering, attack and nav
//! size class a kind reuses until it has a rule of its own.

use crate::math::Vec2;
use crate::render::palette::Rgb;
use rand::{Rng, RngExt};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnemyKind {
    Skitterer,
    Bat,
    Cat,
    Spitter,
    Brute,
    Boss,
    /// Wave 3: an armoured oval that curls to present its shell (`WAVE3_DESIGN.md` §9).
    Pillbug,
    /// Wave 3: a tuning fork that jumps out and back, landing twice.
    BellFlea,
    /// Wave 3: a moth whose death lights a fuse, then bursts.
    WickMoth,
    /// Wave 3: a beetle carrying a sugar cube it raises to heal the horde.
    Porter,
    /// Wave 3: a key-shaped drone that lays crossbars across its own path.
    Turnkey,
    /// Wave 4: a long multi-segment body that cuts the room in half and lunges.
    BigSnake,
    /// Wave 4: a very fast wall-hugging scuttler that darts and flees when hit.
    HouseCentipede,
    /// Wave 4: a flyer that hovers on a rising buzz, dives in a locked line and
    /// must recover on the ground.
    MurderHornet,
    /// Wave 4: a seething swarm with one HP pool, a per-hit damage cap, splits
    /// and regrowth.
    TermiteHorde,
    /// Wave 4: a tiny human squad member (spear, archer, rope crew, leader).
    Lilliputian,

    // ---- Wave-four registrations -----------------------------------------
    //
    // The 33 ordinary kinds `WAVE4_ROSTERS.md`'s six floor tables add, in
    // floor and table order (the enum and `all()` are append-only). Each one
    // is a real, named, correctly-statted body from the day it registers;
    // until its floor's own batch gives it a rule it behaves exactly as
    // [`EnemyKind::archetype`] says, and it draws its archetype's body in its
    // own colour. Per-kind rules and bespoke art are the next batch's work.
    ///
    /// Sewer · C · V(Cat): steers at half turning rate.
    PipeEel,
    /// Sewer · R · R(Spitter): a shot through the puckered lip.
    GrateArcher,
    /// Sewer · II · N: a warned tongue extension (archetype Pillbug for now).
    SumpLeech,
    /// Sewer · H · R(Brute): the wide slow blocker.
    SluiceToad,
    /// Kitchen · A · V(Skitterer): holds its heading between corrections.
    CrumbAnt,
    /// Kitchen · B · R(Bat): a weaving flyer with a red eye.
    FruitFly,
    /// Kitchen · I · N: winds up close, then rings a departing footprint
    /// (archetype Pillbug for now).
    EggTimer,
    /// Kitchen · R · V(Spitter): the shot splits into two rays.
    PepperMill,
    /// Kitchen · H · R(Brute): keeps advancing under fire.
    PieWeight,
    /// Kitchen · III · V(Pillbug): a curl behind a pot-lid claw.
    SinkCrab,
    /// Lab · A · R(Skitterer): a small direct cleaner.
    SwabFoot,
    /// Lab · B · V(Bat): the weave flips handedness.
    SpinnerLid,
    /// Lab · C · R(Cat): a direct large-class route.
    GlassRunner,
    /// Lab · I · N: plants a tip and draws a dotted ray (archetype Bell Flea
    /// for now).
    WalkingSyringe,
    /// Lab · H · R(Brute): a wide slow blocker.
    AutoclaveCart,
    /// Laundry · A · V(Skitterer): the rank stalls to pull its strand free.
    WoolPill,
    /// Laundry · B · R(Bat): a moving shawl over a blunt head.
    ClothesMoth,
    /// Laundry · I · N: two jaws shown, then slid together (archetype Bell
    /// Flea for now).
    PegSnapper,
    /// Laundry · R · V(Spitter): the shot accelerates after its first half
    /// second.
    StarchBulb,
    /// Laundry · H · R(Brute): a slow solid plug.
    LintGolem,
    /// Printworks · A · R(Skitterer): a small direct streamer.
    Silverfish,
    /// Printworks · B · V(Bat): weaves twice as wide for its first second.
    ErratumSlip,
    /// Printworks · C · V(Cat): keeps its last heading after losing sight.
    TypeSlug,
    /// Printworks · I · N: two needle feet marked, then planted (archetype
    /// Bell Flea for now).
    StapleCrawler,
    /// Printworks · R · R(Spitter): a shot from the rolled tip.
    PaperWasp,
    /// Printworks · H · R(Brute): wide continuous contact.
    PaperReel,
    /// Printworks · III · V(Wick Moth): the burst centre slides along the last
    /// velocity.
    InkBlot,
    /// Flatbed · A · V(Skitterer): drifts toward the tailgate while chasing.
    GravelTick,
    /// Flatbed · B · R(Bat): wobbles into the rat's line around cargo.
    JuneBug,
    /// Flatbed · C · R(Cat): a straight large-class chaser.
    TarRat,
    /// Flatbed · I · N: a warned roll that rebounds off the first solid face
    /// (archetype Murder Hornet for now).
    LooseCan,
    /// Flatbed · R · V(Spitter): the shot leads the rat.
    TollGull,
    /// Flatbed · III · V(Turnkey): the crossbar pulls the rat toward its hook.
    LooseBungee,

    // ---- The guest ---------------------------------------------------------
    /// A **guest** body: the Drake, a big duck that pecks (Sewer only, outside
    /// the eight roster slots). It is not a registration and not a slot: the
    /// director's own guest rule keeps at most one alive (two from 8:00), and
    /// its own system (`ecs/systems/drake.rs`) owns its movement states. It
    /// names the Brute as its archetype only so the *nav* size class, the glue
    /// weight, `holds()` and the steering rules agree with a body of its mass.
    Drake,

    // Outfall, in roster slot order.
    SandHopper,
    GlassShrimp,
    Conger,
    Skater,
    PistolShrimp,
    Lampfish,
    Grouper,
    FrySchool,
}

impl EnemyKind {
    pub fn base_hp(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 6.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 18.0,
            EnemyKind::Spitter => 12.0,
            EnemyKind::Brute => 60.0,
            EnemyKind::Boss => 1400.0,
            // Wave 3 (`WAVE3_DESIGN.md` §9: base HP / speed / contact / XP).
            EnemyKind::Pillbug => 18.0,
            EnemyKind::BellFlea => 9.0,
            EnemyKind::WickMoth => 12.0,
            EnemyKind::Porter => 26.0,
            EnemyKind::Turnkey => 22.0,
            // Wave 4.
            EnemyKind::BigSnake => 80.0,
            EnemyKind::HouseCentipede => 7.0,
            EnemyKind::MurderHornet => 14.0,
            EnemyKind::TermiteHorde => 64.0,
            EnemyKind::Lilliputian => 3.0,
            // ---- Wave-four registrations ----------------------------------
            // `WAVE4_ROSTERS.md`, one row per kind: HP / speed / contact / XP.
            // These numbers are the kind's own; behaviour is its archetype's
            // until the floor's own batch lands.
            //
            // Sewer.
            EnemyKind::PipeEel => 16.0,
            EnemyKind::GrateArcher => 12.0,
            EnemyKind::SumpLeech => 11.0,
            EnemyKind::SluiceToad => 58.0,
            // Kitchen.
            EnemyKind::CrumbAnt => 5.0,
            EnemyKind::FruitFly => 4.0,
            EnemyKind::EggTimer => 13.0,
            EnemyKind::PepperMill => 13.0,
            EnemyKind::PieWeight => 60.0,
            EnemyKind::SinkCrab => 22.0,
            // Lab.
            EnemyKind::SwabFoot => 6.0,
            EnemyKind::SpinnerLid => 4.0,
            EnemyKind::GlassRunner => 17.0,
            EnemyKind::WalkingSyringe => 13.0,
            EnemyKind::AutoclaveCart => 60.0,
            // Laundry.
            EnemyKind::WoolPill => 5.0,
            EnemyKind::ClothesMoth => 4.0,
            EnemyKind::PegSnapper => 14.0,
            EnemyKind::StarchBulb => 12.0,
            EnemyKind::LintGolem => 58.0,
            // Printworks.
            EnemyKind::Silverfish => 5.0,
            EnemyKind::ErratumSlip => 4.0,
            EnemyKind::TypeSlug => 18.0,
            EnemyKind::StapleCrawler => 14.0,
            EnemyKind::PaperWasp => 12.0,
            EnemyKind::PaperReel => 62.0,
            EnemyKind::InkBlot => 12.0,
            // Flatbed.
            EnemyKind::GravelTick => 6.0,
            EnemyKind::JuneBug => 4.0,
            EnemyKind::TarRat => 17.0,
            EnemyKind::LooseCan => 13.0,
            EnemyKind::TollGull => 12.0,
            EnemyKind::LooseBungee => 20.0,
            // The guest: the Sewer heavy's own pool (the Sluice Toad's 58.0)
            // times 1.6, so a pecking duck is a real fight and not a fodder
            // body that happens to be wide.
            EnemyKind::Drake => 92.8,
            EnemyKind::SandHopper => 6.0,
            EnemyKind::GlassShrimp => 4.0,
            EnemyKind::Conger => 16.0,
            EnemyKind::Skater => 14.0,
            EnemyKind::PistolShrimp => 12.0,
            EnemyKind::Lampfish => 11.0,
            EnemyKind::Grouper => 60.0,
            EnemyKind::FrySchool => 26.0,
        }
    }
    pub fn speed(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 30.0,
            EnemyKind::Bat => 42.0,
            EnemyKind::Cat => 38.0,
            EnemyKind::Spitter => 22.0,
            EnemyKind::Brute => 16.0,
            EnemyKind::Boss => 20.0,
            EnemyKind::Pillbug => 23.0,
            EnemyKind::BellFlea => 32.0,
            EnemyKind::WickMoth => 28.0,
            EnemyKind::Porter => 20.0,
            EnemyKind::Turnkey => 24.0,
            // Wave 4. The centipede is the fastest body in the game; the thing
            // that keeps it fair is that it always stops for its flutter.
            EnemyKind::BigSnake => 26.0,
            EnemyKind::HouseCentipede => 54.0,
            EnemyKind::MurderHornet => 32.0,
            EnemyKind::TermiteHorde => 26.0,
            EnemyKind::Lilliputian => 22.0,
            // Registrations (`WAVE4_ROSTERS.md` px/s).
            EnemyKind::PipeEel => 39.0,
            EnemyKind::GrateArcher => 22.0,
            EnemyKind::SumpLeech => 25.0,
            EnemyKind::SluiceToad => 16.0,
            EnemyKind::CrumbAnt => 31.0,
            EnemyKind::FruitFly => 41.0,
            EnemyKind::EggTimer => 21.0,
            EnemyKind::PepperMill => 20.0,
            EnemyKind::PieWeight => 16.0,
            EnemyKind::SinkCrab => 23.0,
            EnemyKind::SwabFoot => 29.0,
            EnemyKind::SpinnerLid => 40.0,
            EnemyKind::GlassRunner => 39.0,
            EnemyKind::WalkingSyringe => 25.0,
            EnemyKind::AutoclaveCart => 15.0,
            EnemyKind::WoolPill => 34.0,
            EnemyKind::ClothesMoth => 40.0,
            EnemyKind::PegSnapper => 24.0,
            EnemyKind::StarchBulb => 21.0,
            EnemyKind::LintGolem => 16.0,
            EnemyKind::Silverfish => 32.0,
            EnemyKind::ErratumSlip => 38.0,
            EnemyKind::TypeSlug => 39.0,
            EnemyKind::StapleCrawler => 23.0,
            EnemyKind::PaperWasp => 23.0,
            EnemyKind::PaperReel => 15.0,
            EnemyKind::InkBlot => 25.0,
            EnemyKind::GravelTick => 30.0,
            EnemyKind::JuneBug => 41.0,
            EnemyKind::TarRat => 38.0,
            EnemyKind::LooseCan => 22.0,
            EnemyKind::TollGull => 22.0,
            EnemyKind::LooseBungee => 25.0,
            // Dry-floor speed. The paddling 40 px/s on a puddle is the floor's
            // own rule (`drake::water_gait`), not this number.
            EnemyKind::Drake => 30.0,
            EnemyKind::SandHopper => 30.0,
            EnemyKind::GlassShrimp => 42.0,
            EnemyKind::Conger => 38.0,
            EnemyKind::Skater => 12.0,
            EnemyKind::PistolShrimp => 22.0,
            EnemyKind::Lampfish => 24.0,
            EnemyKind::Grouper => 16.0,
            EnemyKind::FrySchool => 22.0,
        }
    }
    /// Collision radius, which also picks the navigation size class
    /// ([`crate::game::collision::SizeClass::of_enemy`]): a body at or under
    /// `RAT_HOLE_MAX_RADIUS` (3.5 px) squeezes through a rat-hole, a larger one
    /// must take the long way round.
    ///
    /// Wave 3: the Bell Flea (2.0) and the Wick Moth (2.5) are small and follow
    /// the rat through holes; the Pillbug (5.0), Porter (4.5) and Turnkey (4.0)
    /// are large and are held to corridors and doorways.
    pub fn radius(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 2.5,
            EnemyKind::Bat => 2.0,
            EnemyKind::Cat => 4.0,
            EnemyKind::Spitter => 3.0,
            EnemyKind::Brute => 6.5,
            EnemyKind::Boss => 14.0,
            EnemyKind::Pillbug => 5.0,
            EnemyKind::BellFlea => 2.0,
            EnemyKind::WickMoth => 2.5,
            EnemyKind::Porter => 4.5,
            EnemyKind::Turnkey => 4.0,
            // Wave 4. The snake is the only Large one (it cannot use a
            // rat-hole); the centipede, hornet and horde are rat-sized.
            EnemyKind::BigSnake => 6.0,
            EnemyKind::HouseCentipede => 2.4,
            EnemyKind::MurderHornet => 2.6,
            EnemyKind::TermiteHorde => 3.2,
            EnemyKind::Lilliputian => 2.0,
            // The guest: a big body (the rat is 3), one size class above the
            // Brute's 6.5 and the widest ordinary body in the game.
            EnemyKind::Drake => 8.0,
            EnemyKind::SandHopper => 2.5,
            EnemyKind::GlassShrimp => 2.0,
            EnemyKind::Conger => 4.0,
            EnemyKind::Skater => 3.0,
            EnemyKind::PistolShrimp => 3.0,
            EnemyKind::Lampfish => 3.0,
            EnemyKind::Grouper => 6.5,
            EnemyKind::FrySchool => 12.0,
            // A registration borrows its base body's collision radius, and so
            // its navigation size class. Kinds with their own archetype
            // must define a radius above.
            _ => {
                debug_assert!(self.archetype() != self, "a kind must name its archetype");
                self.archetype().radius()
            }
        }
    }
    pub fn damage(self) -> f32 {
        match self {
            EnemyKind::Skitterer => 5.0,
            EnemyKind::Bat => 4.0,
            EnemyKind::Cat => 9.0,
            EnemyKind::Spitter => 6.0,
            EnemyKind::Brute => 16.0,
            EnemyKind::Boss => 30.0,
            EnemyKind::Pillbug => 5.0,
            EnemyKind::BellFlea => 4.0,
            EnemyKind::WickMoth => 4.0,
            EnemyKind::Porter => 6.0,
            EnemyKind::Turnkey => 6.0,
            // Wave 4. The horde's contact is deliberately light: it is a
            // damage check, not a brawler.
            EnemyKind::BigSnake => 8.0,
            EnemyKind::HouseCentipede => 5.0,
            EnemyKind::MurderHornet => 6.0,
            EnemyKind::TermiteHorde => 3.0,
            EnemyKind::Lilliputian => 3.0,
            // Registrations (`WAVE4_ROSTERS.md` contact damage).
            EnemyKind::PipeEel => 8.0,
            EnemyKind::GrateArcher => 6.0,
            EnemyKind::SumpLeech => 5.0,
            EnemyKind::SluiceToad => 15.0,
            EnemyKind::CrumbAnt => 5.0,
            EnemyKind::FruitFly => 4.0,
            EnemyKind::EggTimer => 4.0,
            EnemyKind::PepperMill => 6.0,
            EnemyKind::PieWeight => 15.0,
            EnemyKind::SinkCrab => 6.0,
            EnemyKind::SwabFoot => 5.0,
            EnemyKind::SpinnerLid => 4.0,
            EnemyKind::GlassRunner => 8.0,
            EnemyKind::WalkingSyringe => 5.0,
            EnemyKind::AutoclaveCart => 16.0,
            EnemyKind::WoolPill => 5.0,
            EnemyKind::ClothesMoth => 4.0,
            EnemyKind::PegSnapper => 5.0,
            EnemyKind::StarchBulb => 6.0,
            EnemyKind::LintGolem => 16.0,
            EnemyKind::Silverfish => 5.0,
            EnemyKind::ErratumSlip => 4.0,
            EnemyKind::TypeSlug => 9.0,
            EnemyKind::StapleCrawler => 5.0,
            EnemyKind::PaperWasp => 6.0,
            EnemyKind::PaperReel => 15.0,
            EnemyKind::InkBlot => 4.0,
            EnemyKind::GravelTick => 5.0,
            EnemyKind::JuneBug => 4.0,
            EnemyKind::TarRat => 8.0,
            EnemyKind::LooseCan => 5.0,
            EnemyKind::TollGull => 6.0,
            EnemyKind::LooseBungee => 6.0,
            // Body contact only: the Sewer's ground fodder's number. It is the
            // peck that hurts (`drake::PECK_DAMAGE`), not the shoulder.
            EnemyKind::Drake => 5.0,
            EnemyKind::SandHopper => 5.0,
            EnemyKind::GlassShrimp => 4.0,
            EnemyKind::Conger => 8.0,
            EnemyKind::Skater => 5.0,
            EnemyKind::PistolShrimp => 6.0,
            EnemyKind::Lampfish => 5.0,
            EnemyKind::Grouper => 15.0,
            EnemyKind::FrySchool => 5.0,
        }
    }
    pub fn xp(self) -> u32 {
        match self {
            EnemyKind::Skitterer => 1,
            EnemyKind::Bat => 1,
            EnemyKind::Cat => 3,
            EnemyKind::Spitter => 4,
            EnemyKind::Brute => 10,
            EnemyKind::Boss => 200,
            EnemyKind::Pillbug => 3,
            EnemyKind::BellFlea => 2,
            EnemyKind::WickMoth => 3,
            EnemyKind::Porter => 5,
            EnemyKind::Turnkey => 4,
            // Wave 4.
            EnemyKind::BigSnake => 12,
            EnemyKind::HouseCentipede => 2,
            EnemyKind::MurderHornet => 4,
            EnemyKind::TermiteHorde => 3,
            EnemyKind::Lilliputian => 1,
            // Registrations (`WAVE4_ROSTERS.md` gem XP).
            EnemyKind::PipeEel => 3,
            EnemyKind::GrateArcher => 4,
            EnemyKind::SumpLeech => 3,
            EnemyKind::SluiceToad => 10,
            EnemyKind::CrumbAnt => 1,
            EnemyKind::FruitFly => 1,
            EnemyKind::EggTimer => 3,
            EnemyKind::PepperMill => 4,
            EnemyKind::PieWeight => 10,
            EnemyKind::SinkCrab => 4,
            EnemyKind::SwabFoot => 1,
            EnemyKind::SpinnerLid => 1,
            EnemyKind::GlassRunner => 3,
            EnemyKind::WalkingSyringe => 3,
            EnemyKind::AutoclaveCart => 10,
            EnemyKind::WoolPill => 1,
            EnemyKind::ClothesMoth => 1,
            EnemyKind::PegSnapper => 3,
            EnemyKind::StarchBulb => 4,
            EnemyKind::LintGolem => 10,
            EnemyKind::Silverfish => 1,
            EnemyKind::ErratumSlip => 1,
            EnemyKind::TypeSlug => 3,
            EnemyKind::StapleCrawler => 3,
            EnemyKind::PaperWasp => 4,
            EnemyKind::PaperReel => 10,
            EnemyKind::InkBlot => 3,
            EnemyKind::GravelTick => 1,
            EnemyKind::JuneBug => 1,
            EnemyKind::TarRat => 3,
            EnemyKind::LooseCan => 3,
            EnemyKind::TollGull => 4,
            EnemyKind::LooseBungee => 4,
            // The guest: three times the Sewer heavy (the Sluice Toad's 10).
            EnemyKind::Drake => 30,
            EnemyKind::SandHopper => 1,
            EnemyKind::GlassShrimp => 1,
            EnemyKind::Conger => 3,
            EnemyKind::Skater => 3,
            EnemyKind::PistolShrimp => 4,
            EnemyKind::Lampfish => 3,
            EnemyKind::Grouper => 10,
            EnemyKind::FrySchool => 3,
        }
    }
    pub fn color(self) -> Rgb {
        match self {
            EnemyKind::Skitterer => (150, 120, 90),
            EnemyKind::Bat => (120, 90, 160),
            EnemyKind::Cat => (90, 90, 100),
            EnemyKind::Spitter => (120, 200, 90),
            EnemyKind::Brute => (170, 70, 60),
            EnemyKind::Boss => (220, 60, 120),
            // The wave-3 silhouettes read by colour as well as shape: silver
            // shell, copper fork, warm wick, pale sugar, brass key.
            EnemyKind::Pillbug => (186, 192, 206),
            EnemyKind::BellFlea => (214, 138, 78),
            EnemyKind::WickMoth => (232, 168, 104),
            EnemyKind::Porter => (140, 176, 128),
            EnemyKind::Turnkey => (206, 186, 102),
            // Wave 4: olive scales, pale chitin, a hot hazard stripe and a
            // termite cream.
            EnemyKind::BigSnake => (118, 146, 78),
            EnemyKind::HouseCentipede => (198, 176, 138),
            EnemyKind::MurderHornet => (214, 148, 40),
            EnemyKind::TermiteHorde => (206, 190, 150),
            // Facility staff, shrunk: the pale lab-coat grey-blue of the walls.
            EnemyKind::Lilliputian => (198, 206, 216),
            // ---- Wave-four registrations ----------------------------------
            // Picked from each row's silhouette and its floor's palette so the
            // eight bodies of a floor read apart at a glance. The colour is
            // the kind's own even while it draws its archetype's body.
            //
            // Sewer: bone, barnacle, black sump, wet olive.
            EnemyKind::PipeEel => (222, 214, 190),
            EnemyKind::GrateArcher => (140, 168, 150),
            EnemyKind::SumpLeech => (58, 54, 62),
            EnemyKind::SluiceToad => (104, 138, 96),
            // Kitchen: crust, peach fly, enamel, wood, tin, rust.
            EnemyKind::CrumbAnt => (92, 66, 44),
            EnemyKind::FruitFly => (196, 96, 72),
            EnemyKind::EggTimer => (222, 214, 196),
            EnemyKind::PepperMill => (168, 120, 72),
            EnemyKind::PieWeight => (176, 180, 188),
            EnemyKind::SinkCrab => (184, 86, 64),
            // Lab: cotton, cyan rim, glass, steel, enamel grey.
            EnemyKind::SwabFoot => (236, 238, 242),
            EnemyKind::SpinnerLid => (120, 208, 224),
            EnemyKind::GlassRunner => (176, 214, 208),
            EnemyKind::WalkingSyringe => (156, 168, 186),
            EnemyKind::AutoclaveCart => (150, 156, 170),
            // Laundry: ivory wool, dusty shawl, peg wood, starch, lint.
            EnemyKind::WoolPill => (226, 218, 198),
            EnemyKind::ClothesMoth => (170, 150, 120),
            EnemyKind::PegSnapper => (190, 146, 92),
            EnemyKind::StarchBulb => (198, 206, 214),
            EnemyKind::LintGolem => (140, 136, 130),
            // Printworks: silver, paper, lead, staple steel, newsprint, spool,
            // dried ink.
            EnemyKind::Silverfish => (196, 200, 206),
            EnemyKind::ErratumSlip => (232, 228, 208),
            EnemyKind::TypeSlug => (108, 110, 118),
            EnemyKind::StapleCrawler => (150, 166, 190),
            EnemyKind::PaperWasp => (206, 196, 164),
            EnemyKind::PaperReel => (140, 112, 80),
            EnemyKind::InkBlot => (64, 54, 80),
            // Flatbed: gravel, beetle brown, tar, aluminium, gull white, red
            // rubber.
            EnemyKind::GravelTick => (150, 146, 138),
            EnemyKind::JuneBug => (120, 84, 52),
            EnemyKind::TarRat => (56, 52, 48),
            EnemyKind::LooseCan => (188, 192, 196),
            EnemyKind::TollGull => (232, 232, 224),
            EnemyKind::LooseBungee => (186, 78, 74),
            // The guest: a white body under an orange bill (the bill itself is
            // `art_duck`'s own colour; this is what the rim, the corpse and the
            // Library card read).
            EnemyKind::Drake => (236, 232, 220),
            EnemyKind::SandHopper => (184, 147, 106),
            EnemyKind::GlassShrimp => (151, 211, 213),
            EnemyKind::Conger => (124, 163, 149),
            EnemyKind::Skater => (109, 211, 182),
            EnemyKind::PistolShrimp => (226, 137, 126),
            EnemyKind::Lampfish => (123, 187, 208),
            EnemyKind::Grouper => (146, 125, 164),
            EnemyKind::FrySchool => (175, 199, 150),
        }
    }

    /// The **existing** kind whose steering, attack, navigation size class,
    /// collision radius, wave-3/4 behaviour state and behaviour systems this
    /// kind reuses.
    ///
    /// Every kind that existed before the wave-four batch is its own archetype.
    /// Each of the batch's 33 registrations names one of them — `R(X)` and
    /// `V(X)` rows name `X`, and the six `N` rows name the nearest existing
    /// behaviour for their slot until the floor's own batch gives them their
    /// own. That is the whole inheritance seam: every **behavioural** decision
    /// in the simulation keys on `archetype()`, while stats, name, colour,
    /// gold, XP, Library identity and art tint stay keyed on the real kind.
    /// The next batch replaces an `N` row's base by changing one arm here.
    pub fn archetype(self) -> EnemyKind {
        match self {
            // ---- Sewer ----------------------------------------------------
            // Pipe Eel · V(Cat): the design inherits the Cat's straight
            // large-class pursuit.
            EnemyKind::PipeEel => EnemyKind::Cat,
            // Grate Archer · R(Spitter): the Spitter's aimed pellet.
            EnemyKind::GrateArcher => EnemyKind::Spitter,
            // Sump Leech · N: a warned extension the rat steps out of. The
            // Pillbug is the nearest existing "commit in place, then expose"
            // body on the same floor.
            EnemyKind::SumpLeech => EnemyKind::Pillbug,
            // Sluice Toad · R(Brute): the Brute's slow wide block.
            EnemyKind::SluiceToad => EnemyKind::Brute,
            // ---- Kitchen --------------------------------------------------
            // Crumb Ant · V(Skitterer): the Skitterer's direct column.
            EnemyKind::CrumbAnt => EnemyKind::Skitterer,
            // Fruit Fly · R(Bat): the Bat's weave.
            EnemyKind::FruitFly => EnemyKind::Bat,
            // Egg Timer · N: a stationary timed footprint. Pillbug (stop,
            // commit, expose) for now.
            EnemyKind::EggTimer => EnemyKind::Pillbug,
            // Pepper Mill · V(Spitter): the Spitter's stand-off and shot.
            EnemyKind::PepperMill => EnemyKind::Spitter,
            // Pie Weight · R(Brute): the Brute.
            EnemyKind::PieWeight => EnemyKind::Brute,
            // Sink Crab · V(Pillbug): the Pillbug's curl.
            EnemyKind::SinkCrab => EnemyKind::Pillbug,
            // ---- Lab ------------------------------------------------------
            // Swab Foot · R(Skitterer): the Skitterer.
            EnemyKind::SwabFoot => EnemyKind::Skitterer,
            // Spinner Lid · V(Bat): the Bat.
            EnemyKind::SpinnerLid => EnemyKind::Bat,
            // Glass Runner · R(Cat): the Cat.
            EnemyKind::GlassRunner => EnemyKind::Cat,
            // Walking Syringe · N: a planted, warned thrust. The Bell Flea's
            // "mark two points, then commit" read is the nearest.
            EnemyKind::WalkingSyringe => EnemyKind::BellFlea,
            // Autoclave Cart · R(Brute): the Brute.
            EnemyKind::AutoclaveCart => EnemyKind::Brute,
            // ---- Laundry --------------------------------------------------
            // Wool Pill · V(Skitterer): the Skitterer.
            EnemyKind::WoolPill => EnemyKind::Skitterer,
            // Clothes Moth · R(Bat): the Bat.
            EnemyKind::ClothesMoth => EnemyKind::Bat,
            // Peg Snapper · N: two jaws shown, then closed. Bell Flea (two
            // marked points, then a commit) for now.
            EnemyKind::PegSnapper => EnemyKind::BellFlea,
            // Starch Bulb · V(Spitter): the Spitter.
            EnemyKind::StarchBulb => EnemyKind::Spitter,
            // Lint Golem · R(Brute): the Brute.
            EnemyKind::LintGolem => EnemyKind::Brute,
            // ---- Printworks ----------------------------------------------
            // Silverfish · R(Skitterer): the Skitterer.
            EnemyKind::Silverfish => EnemyKind::Skitterer,
            // Erratum Slip · V(Bat): the Bat.
            EnemyKind::ErratumSlip => EnemyKind::Bat,
            // Type Slug · V(Cat): the Cat.
            EnemyKind::TypeSlug => EnemyKind::Cat,
            // Staple Crawler · N: two needle feet marked, then planted. Bell
            // Flea for now (the same two-cross read).
            EnemyKind::StapleCrawler => EnemyKind::BellFlea,
            // Paper Wasp · R(Spitter): the Spitter.
            EnemyKind::PaperWasp => EnemyKind::Spitter,
            // Paper Reel · R(Brute): the Brute.
            EnemyKind::PaperReel => EnemyKind::Brute,
            // Ink Blot · V(Wick Moth): the Moth's fuse and burst.
            EnemyKind::InkBlot => EnemyKind::WickMoth,
            // ---- Flatbed --------------------------------------------------
            // Gravel Tick · V(Skitterer): the Skitterer.
            EnemyKind::GravelTick => EnemyKind::Skitterer,
            // June Bug · R(Bat): the Bat.
            EnemyKind::JuneBug => EnemyKind::Bat,
            // Tar Rat · R(Cat): the Cat.
            EnemyKind::TarRat => EnemyKind::Cat,
            // Loose Can · N: a warned roll that rebounds. The Murder Hornet's
            // locked, warned lane and grounded recovery is the nearest.
            EnemyKind::LooseCan => EnemyKind::MurderHornet,
            // Toll Gull · V(Spitter): the Spitter.
            EnemyKind::TollGull => EnemyKind::Spitter,
            // Loose Bungee · V(Turnkey): the Turnkey's crossbar.
            EnemyKind::LooseBungee => EnemyKind::Turnkey,
            // ---- The guest --------------------------------------------------
            // The Drake names the Brute for the decisions that are *not* a rule
            // of its own: the navigation size class (`SizeClass::of_enemy`),
            // the glue weight and hold (`traps::enemy_units`/`enemy_hold`), the
            // steering shape (`Enemy::steer`), the bot's archetype scans and
            // `W4::holds`. Its movement states, tell, peck volley and recovery
            // live in its own system, so it is **not** a registration (see
            // `is_registration`).
            EnemyKind::Drake => EnemyKind::Brute,
            // ---- Every earlier kind is its own archetype -------------------
            EnemyKind::Skitterer
            | EnemyKind::Bat
            | EnemyKind::Cat
            | EnemyKind::Spitter
            | EnemyKind::Brute
            | EnemyKind::Boss
            | EnemyKind::Pillbug
            | EnemyKind::BellFlea
            | EnemyKind::WickMoth
            | EnemyKind::Porter
            | EnemyKind::Turnkey
            | EnemyKind::BigSnake
            | EnemyKind::HouseCentipede
            | EnemyKind::MurderHornet
            | EnemyKind::TermiteHorde
            | EnemyKind::Lilliputian => self,
            EnemyKind::SandHopper => EnemyKind::Skitterer,
            EnemyKind::GlassShrimp => EnemyKind::Bat,
            EnemyKind::Conger => EnemyKind::Cat,
            EnemyKind::Skater => self,
            EnemyKind::PistolShrimp => EnemyKind::Spitter,
            EnemyKind::Lampfish => self,
            EnemyKind::Grouper => EnemyKind::Brute,
            EnemyKind::FrySchool => self,
        }
    }

    /// True for a wave-four registration: a kind that reuses another body's
    /// behaviour intact until its floor's own batch gives it a rule.
    ///
    /// The **guest** (the Drake) is not one: it names the Brute as its
    /// archetype for the nav and mass decisions alone, and has owned its own
    /// tell → peck → recover state machine from the day it shipped.
    #[inline]
    pub fn is_registration(self) -> bool {
        // The historical registrations are contiguous in the append-only enum.
        (EnemyKind::PipeEel as usize..=EnemyKind::LooseBungee as usize).contains(&(self as usize))
    }
    /// Spitters fire enemy bullets; others are melee.
    ///
    /// A wave-3 kind is never `ranged`: its damage arrives as a floor hazard the
    /// behaviour system resolves (`enemy_wave3`), not as a bullet.
    pub fn ranged(self) -> bool {
        matches!(self.archetype(), EnemyKind::Spitter | EnemyKind::Boss)
    }

    /// Runs one of the five wave-3 state machines.
    #[inline]
    pub fn is_wave3(self) -> bool {
        matches!(
            self.archetype(),
            EnemyKind::Pillbug
                | EnemyKind::BellFlea
                | EnemyKind::WickMoth
                | EnemyKind::Porter
                | EnemyKind::Turnkey
        )
    }

    /// Runs a wave-4 state machine.
    #[inline]
    pub fn is_wave4(self) -> bool {
        matches!(
            self.archetype(),
            EnemyKind::BigSnake
                | EnemyKind::HouseCentipede
                | EnemyKind::MurderHornet
                | EnemyKind::TermiteHorde
                | EnemyKind::Lilliputian
        )
    }

    /// A body that cannot be shoved around by an ordinary melee hit.
    #[inline]
    pub fn heavy(self) -> bool {
        matches!(
            self.archetype(),
            EnemyKind::Brute
                | EnemyKind::Boss
                | EnemyKind::Pillbug
                | EnemyKind::BigSnake
                | EnemyKind::TermiteHorde
        )
    }

    /// The Outfall swimmers that cross tidal drops without falling.
    pub fn outfall_swims(self) -> bool {
        matches!(self, EnemyKind::GlassShrimp | EnemyKind::Lampfish)
    }

    pub(crate) fn has_outfall_state(self) -> bool {
        matches!(self, EnemyKind::Skater | EnemyKind::Lampfish | EnemyKind::FrySchool)
    }

    /// Every kind, in enum order. The console's `spawn` command and its Tab
    /// completion read this table rather than a retyped list.
    ///
    /// 58 kinds: the 16 earlier bodies (including the legacy Boss), the 33
    /// wave-four registrations, the Drake guest, and the eight Outfall bodies.
    pub fn all() -> [EnemyKind; 58] {
        [
            EnemyKind::Skitterer,
            EnemyKind::Bat,
            EnemyKind::Cat,
            EnemyKind::Spitter,
            EnemyKind::Brute,
            EnemyKind::Boss,
            EnemyKind::Pillbug,
            EnemyKind::BellFlea,
            EnemyKind::WickMoth,
            EnemyKind::Porter,
            EnemyKind::Turnkey,
            EnemyKind::BigSnake,
            EnemyKind::HouseCentipede,
            EnemyKind::MurderHornet,
            EnemyKind::TermiteHorde,
            EnemyKind::Lilliputian,
            // The 33 wave-four registrations, in floor and table order.
            EnemyKind::PipeEel,
            EnemyKind::GrateArcher,
            EnemyKind::SumpLeech,
            EnemyKind::SluiceToad,
            EnemyKind::CrumbAnt,
            EnemyKind::FruitFly,
            EnemyKind::EggTimer,
            EnemyKind::PepperMill,
            EnemyKind::PieWeight,
            EnemyKind::SinkCrab,
            EnemyKind::SwabFoot,
            EnemyKind::SpinnerLid,
            EnemyKind::GlassRunner,
            EnemyKind::WalkingSyringe,
            EnemyKind::AutoclaveCart,
            EnemyKind::WoolPill,
            EnemyKind::ClothesMoth,
            EnemyKind::PegSnapper,
            EnemyKind::StarchBulb,
            EnemyKind::LintGolem,
            EnemyKind::Silverfish,
            EnemyKind::ErratumSlip,
            EnemyKind::TypeSlug,
            EnemyKind::StapleCrawler,
            EnemyKind::PaperWasp,
            EnemyKind::PaperReel,
            EnemyKind::InkBlot,
            EnemyKind::GravelTick,
            EnemyKind::JuneBug,
            EnemyKind::TarRat,
            EnemyKind::LooseCan,
            EnemyKind::TollGull,
            EnemyKind::LooseBungee,
            // The guest, appended after the roster's 33 registrations.
            EnemyKind::Drake,
            EnemyKind::SandHopper,
            EnemyKind::GlassShrimp,
            EnemyKind::Conger,
            EnemyKind::Skater,
            EnemyKind::PistolShrimp,
            EnemyKind::Lampfish,
            EnemyKind::Grouper,
            EnemyKind::FrySchool,
        ]
    }

    /// Lowercase display name used by the console and diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            EnemyKind::Skitterer => "skitterer",
            EnemyKind::Bat => "bat",
            EnemyKind::Cat => "cat",
            EnemyKind::Spitter => "spitter",
            EnemyKind::Brute => "brute",
            EnemyKind::Boss => "boss",
            EnemyKind::Pillbug => "pillbug",
            EnemyKind::BellFlea => "bellflea",
            EnemyKind::WickMoth => "wickmoth",
            EnemyKind::Porter => "porter",
            EnemyKind::Turnkey => "turnkey",
            EnemyKind::BigSnake => "bigsnake",
            EnemyKind::HouseCentipede => "housecentipede",
            EnemyKind::MurderHornet => "murderhornet",
            EnemyKind::TermiteHorde => "termitehorde",
            EnemyKind::Lilliputian => "lilliputian",
            // The wave-four registrations (no spaces: the console's `spawn`
            // command slugs these names).
            EnemyKind::PipeEel => "pipeeel",
            EnemyKind::GrateArcher => "gratearcher",
            EnemyKind::SumpLeech => "sumpleech",
            EnemyKind::SluiceToad => "sluicetoad",
            EnemyKind::CrumbAnt => "crumbant",
            EnemyKind::FruitFly => "fruitfly",
            EnemyKind::EggTimer => "eggtimer",
            EnemyKind::PepperMill => "peppermill",
            EnemyKind::PieWeight => "pieweight",
            EnemyKind::SinkCrab => "sinkcrab",
            EnemyKind::SwabFoot => "swabfoot",
            EnemyKind::SpinnerLid => "spinnerlid",
            EnemyKind::GlassRunner => "glassrunner",
            EnemyKind::WalkingSyringe => "walkingsyringe",
            EnemyKind::AutoclaveCart => "autoclavecart",
            EnemyKind::WoolPill => "woolpill",
            EnemyKind::ClothesMoth => "clothesmoth",
            EnemyKind::PegSnapper => "pegsnapper",
            EnemyKind::StarchBulb => "starchbulb",
            EnemyKind::LintGolem => "lintgolem",
            EnemyKind::Silverfish => "silverfish",
            EnemyKind::ErratumSlip => "erratumslip",
            EnemyKind::TypeSlug => "typeslug",
            EnemyKind::StapleCrawler => "staplecrawler",
            EnemyKind::PaperWasp => "paperwasp",
            EnemyKind::PaperReel => "paperreel",
            EnemyKind::InkBlot => "inkblot",
            EnemyKind::GravelTick => "graveltick",
            EnemyKind::JuneBug => "junebug",
            EnemyKind::TarRat => "tarrat",
            EnemyKind::LooseCan => "loosecan",
            EnemyKind::TollGull => "tollgull",
            EnemyKind::LooseBungee => "loosebungee",
            // The guest (one word: the console's `spawn` command slugs names).
            EnemyKind::Drake => "drake",
            EnemyKind::SandHopper => "sandhopper",
            EnemyKind::GlassShrimp => "glassshrimp",
            EnemyKind::Conger => "conger",
            EnemyKind::Skater => "skater",
            EnemyKind::PistolShrimp => "pistolshrimp",
            EnemyKind::Lampfish => "lampfish",
            EnemyKind::Grouper => "grouper",
            EnemyKind::FrySchool => "fryschool",
        }
    }
}

// ---- Wave-3 behaviour state ----------------------------------------------

/// Per-body scratch for the five wave-3 behaviours (`systems::enemy_wave3`).
///
/// The timers, ranges and damages themselves live in that module; this struct
/// is only the storage, so the state round-trips with the body (a test can
/// inspect or force it between ticks) and the fast path stays allocation-free.
/// Every field is zero/`ZERO` for the base six kinds, so their tick is
/// byte-for-byte unchanged and [`W3::holds`] is never true for them.
#[derive(Clone, Copy, Debug)]
pub struct W3 {
    /// 1 once the behaviour system has seeded this body's timers (a body can be
    /// spawned by the facade or a test as well as by the director, so the
    /// seeding happens on the first tick it is simulated).
    pub armed: u8,
    /// Seconds since the current warned tell started (0 = this tick).
    pub warn_age: f32,
    /// Pillbug: seconds left curled (0 = walking), and the cooldown after one.
    pub curl_t: f32,
    pub curl_cd: f32,
    /// Bell Flea: 0 walking, 1 winding up, 2 landed (about to spring back).
    pub flea_phase: u8,
    /// Seconds left in the current flea phase.
    pub flea_t: f32,
    /// The two locked landing points: where the hop goes and where it started.
    pub hop_to: Vec2,
    pub home: Vec2,
    /// Seconds until the next hop.
    pub flea_cd: f32,
    /// Wick Moth: > 0 the fuse is burning on the corpse (the behaviour system
    /// spawns the `Wick` entity on the tick it sees it), -1 = spent.
    pub wick_t: f32,
    /// Porter: seconds left raising the cube (0 = not raising).
    pub raise_t: f32,
    /// Seconds until the next raise.
    pub heal_cd: f32,
    /// `SpawnId` of the ally being raised for (`u64::MAX` = none).
    pub heal_target: u64,
    /// Turnkey: seconds until the next crossbar.
    pub bar_cd: f32,
    /// Seconds left of the current crossbar (tell first, then live).
    pub bar_t: f32,
    /// Where the crossbar was laid and which way it runs (world space).
    pub bar_pos: Vec2,
    pub bar_dir: Vec2,
    /// The crossbar has already taken its one bite out of the rat.
    pub bar_spent: bool,
    /// The Sewer's two opening Pillbugs hold their first curl until approached.
    pub opening_curl: bool,
}

impl Default for W3 {
    fn default() -> Self {
        W3 {
            armed: 0,
            warn_age: 0.0,
            curl_t: 0.0,
            curl_cd: 0.0,
            flea_phase: 0,
            flea_t: 0.0,
            hop_to: Vec2::ZERO,
            home: Vec2::ZERO,
            flea_cd: 0.0,
            wick_t: 0.0,
            raise_t: 0.0,
            heal_cd: 0.0,
            heal_target: u64::MAX,
            bar_cd: 0.0,
            bar_t: 0.0,
            bar_pos: Vec2::ZERO,
            bar_dir: Vec2::ZERO,
            bar_spent: false,
            opening_curl: false,
        }
    }
}

impl W3 {
    /// The turnkey's 0.8 s dotted tell (before the bar goes live).
    #[inline]
    pub fn bar_telling(&self) -> bool {
        self.bar_t > crate::game::ecs::systems::enemy_wave3::CROSSBAR_LIVE_SECS
    }

    /// The body is held in place this tick (its behaviour owns its position).
    #[inline]
    pub fn holds(&self) -> bool {
        self.curl_t > 0.0 || self.flea_phase != 0 || self.raise_t > 0.0
    }
}

// ---- Wave-4 behaviour state ----------------------------------------------

/// Body points the big snake draws and collides with, head first.
pub const SNAKE_MAX_SEGMENTS: usize = 7;

/// Per-body scratch for the four wave-4 behaviours (`systems::enemy_wave4`).
///
/// The same contract as [`W3`]: only storage. Every field is zero/`ZERO` for
/// every other kind, so a body's tick is unchanged and [`W4::holds`] is never
/// true for the ten older kinds. `phase`, `t`, `cd` and `dir` are shared
/// between the three state machines because only one of them ever runs on a
/// given body.
#[derive(Clone, Copy, Debug)]
pub struct W4 {
    /// 1 once `enemy_wave4` has seeded this body's timers.
    pub armed: u8,
    /// Seconds since the current warned tell started (0 = this tick).
    pub warn_age: f32,
    /// Kind-specific phase (see the constants in `enemy_wave4`).
    pub phase: u8,
    /// Seconds left in the current phase.
    pub t: f32,
    /// Seconds until the next warned attack.
    pub cd: f32,
    /// The locked attack direction (lunge, dart, dive).
    pub dir: Vec2,
    /// The snake's head-first body points (world space, `trail[0]` = head).
    pub trail: [Vec2; SNAKE_MAX_SEGMENTS],
    /// How far the current lunge has travelled (px).
    pub traveled: f32,
    /// One-bit latch for the current phase (the hornet's dive jab).
    pub spent: u8,
    /// How many of [`W4::trail`] are live (0 until the first simulated tick).
    pub trail_len: u8,
    /// Centipede: seconds of fleeing left after a hit.
    pub flee_t: f32,
    /// Horde: size tier (2 large, 1 medium, 0 small).
    pub tier: u8,
    /// Horde: seconds since the last damage (regrowth clock).
    pub regrow_t: f32,
    /// Horde: set by `Enemy::apply_damage` when a hard hit should split it.
    pub split_pending: u8,
    /// Horde: 1 once this body has already split (a body splits at most once).
    pub splits: u8,
    /// Horde: seconds until the next chewing crumb.
    pub chew_t: f32,
    /// Lilliputian: role (0 leader, 1 spear, 2 archer, 3 rope).
    pub role: u8,
    /// Lilliputian: place in the squad, so the spear line forms up in order.
    pub slot: u8,
    /// Lilliputian: the squad's id (its leader's `SpawnId`), so a squad can
    /// count itself for the panic rule and a panicked straggler can join
    /// another squad.
    pub squad: u32,
    /// Lilliputian: the largest this body's squad has ever been, so a squad
    /// that *started* small (a test, or a half squad) does not instantly break.
    pub squad_peak: u16,
}

impl Default for W4 {
    fn default() -> Self {
        W4 {
            armed: 0,
            warn_age: 0.0,
            phase: 0,
            t: 0.0,
            cd: 0.0,
            dir: Vec2::ZERO,
            trail: [Vec2::ZERO; SNAKE_MAX_SEGMENTS],
            traveled: 0.0,
            spent: 0,
            trail_len: 0,
            flee_t: 0.0,
            tier: 0,
            regrow_t: 0.0,
            split_pending: 0,
            splits: 0,
            chew_t: 0.0,
            role: 0,
            slot: 0,
            squad: 0,
            squad_peak: 0,
        }
    }
}

impl W4 {
    /// The body is held in place this tick (its behaviour owns its position).
    ///
    /// The snake and the hornet always move themselves (a slither or a hover
    /// is not a straight chase); the centipede only while it flutters or
    /// darts. The horde is an ordinary flow-field chaser.
    #[inline]
    pub fn holds(&self, kind: EnemyKind) -> bool {
        match kind.archetype() {
            // Every wave-4 body with a state machine owns its own position for
            // the whole tick: the snake slithers S-curves, the hornet orbits,
            // the centipede scuttles along walls and the lilliputian walks in
            // formation. `enemy_wave4` reads the glue/ink components itself and
            // folds them into the speed (see `speed_mult` in that module), so
            // holding here does not make a body immune to a trap.
            EnemyKind::BigSnake
            | EnemyKind::HouseCentipede
            | EnemyKind::MurderHornet
            | EnemyKind::Lilliputian => true,
            _ => false,
        }
    }
}

/// How a campaign level's horde differs from the base kinds.
///
/// The base kinds ([`EnemyKind::damage`], [`EnemyKind::speed`]) are the Sewer's
/// and are owned by the enemy-stats tuning pass; a later level *multiplies*
/// them here instead of retuning them, so the two cannot fight each other.
/// [`crate::game::director::enemy_mods_for`] builds these per spawn and ramps
/// them over the level's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyMods {
    /// Multiplies [`EnemyKind::damage`] (what contact/bullets take off the rat).
    pub damage: f32,
    /// Multiplies [`EnemyKind::speed`] (the Lab's swarm is quicker than the
    /// Sewer's).
    pub speed: f32,
}

impl EnemyMods {
    /// The base kinds, unchanged: campaign level 1.
    pub const NONE: EnemyMods = EnemyMods {
        damage: 1.0,
        speed: 1.0,
    };
}

impl Default for EnemyMods {
    fn default() -> Self {
        EnemyMods::NONE
    }
}

/// Attack clocks and locked geometry for the Outfall specials.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum OutfallAttack {
    #[default]
    Idle,
    Tell { start: Vec2, dir: Vec2, remaining: f32 },
    Splash { start: Vec2, dir: Vec2, spent: bool },
    Lunge {
        start: Vec2,
        dir: Vec2,
        remaining: f32,
        previous: Vec2,
        hit: bool,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OutfallState {
    pub attack: OutfallAttack,
    pub rearm: f32,
    pub bunched: bool,
}

#[derive(Clone)]
pub struct Enemy {
    /// Monotonic runtime identifier assigned on spawn (0 if constructed outside `World`).
    pub id: u32,
    pub kind: EnemyKind,
    /// True for periodic elite Brutes spawned by the director.
    pub elite: bool,
    /// A named miniboss body (`systems::miniboss`): it also carries the
    /// `Miniboss` component, but the flag lets the many `&Enemy` readers (the
    /// decoy lure, the Porter's casebook, the flatbed's pits) recognise it
    /// without another query. False for every director-spawned body.
    pub named: bool,
    pub pos: Vec2,
    pub vel: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub radius: f32,
    pub damage: f32,
    pub flash: f32,
    pub fire_cd: f32,
    pub wander: f32,
    pub contact_cd: f32,
    /// Campaign-level speed multiplier ([`EnemyMods::speed`]); 1.0 for the base
    /// kinds. Stored on the body because [`Enemy::steer`] reads it every tick.
    pub speed_scale: f32,
    /// Gait/wing animation phase, advanced by movement.
    pub anim: f32,
    /// Smoothed facing direction for sprite orientation.
    pub facing: Vec2,
    /// Seconds of hard stun left (0 = free). A stunned body does not steer, its
    /// velocity bleeds off and it cannot fire (the Sonic Squeak ring).
    pub stun: f32,
    /// Movement multiplier while `slow_t > 0` (1.0 = unaffected). Elites and the
    /// boss are slowed instead of stunned.
    pub slow_mult: f32,
    /// Seconds of slow left.
    pub slow_t: f32,
    /// Wave-3 behaviour state (all zero for the six base kinds).
    pub w3: W3,
    /// Wave-4 behaviour state (all zero for the ten older kinds).
    pub w4: W4,
    /// A named body underground, temporarily immune to hits and contact.
    pub buried: bool,
    /// A body's movement or attack currently fixes its position.
    pub position_locked: bool,
    /// Appointment fodder carries no kill credit, drops or horde budget.
    pub summoned: bool,
    pub(crate) outfall: OutfallState,
}

impl Enemy {
    pub fn spawn(kind: EnemyKind, pos: Vec2, hp_mult: f32, rng: &mut impl Rng) -> Self {
        Self::spawn_with_phases(
            kind, pos, hp_mult,
            rng.random_range(1.0..2.5),
            rng.random_range(0.0..std::f32::consts::TAU),
            rng.random_range(0.0..std::f32::consts::TAU),
        )
    }

    /// An authored body with fixed initial gait and firing phases.
    pub(crate) fn spawn_fixed(kind: EnemyKind, pos: Vec2, hp_mult: f32) -> Self {
        Self::spawn_with_phases(kind, pos, hp_mult, 1.0, 0.0, 0.0)
    }

    pub(crate) fn spawn_with_phases(
        kind: EnemyKind, pos: Vec2, hp_mult: f32, fire_cd: f32, wander: f32, anim: f32,
    ) -> Self {
        let hp = kind.base_hp() * hp_mult;
        Enemy {
            id: 0,
            kind,
            elite: false,
            named: false,
            pos,
            vel: Vec2::ZERO,
            hp,
            max_hp: hp,
            radius: kind.radius(),
            damage: kind.damage(),
            flash: 0.0,
            fire_cd,
            wander,
            contact_cd: 0.0,
            speed_scale: 1.0,
            anim,
            facing: Vec2::new(1.0, 0.0),
            // No extra rng draws: a freshly spawned body is neither stunned nor
            // slowed, so adding these fields cannot move the rng stream.
            stun: 0.0,
            slow_mult: 1.0,
            slow_t: 0.0,
            // Likewise zero-initialised: the wave-3 timers are seeded by the
            // behaviour system on the body's first simulated tick, never from
            // the rng.
            w3: W3::default(),
            // Same rule for wave 4 (the snake's trail is seeded from its spawn
            // point on the first simulated tick).
            w4: W4::default(),
            buried: false,
            position_locked: false,
            summoned: false,
            outfall: OutfallState::default(),
        }
    }

    /// Decay the crowd-control timers and stop a stunned body dead.
    ///
    /// Returns `true` while the body is stunned (the caller must skip
    /// steering). At the defaults (`stun == 0`, `slow_t == 0`) this only
    /// performs the two timer comparisons, so an ordinary enemy's tick is
    /// unchanged.
    fn tick_control(&mut self, dt: f32) -> bool {
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            self.vel = self.vel * (1.0 - (12.0 * dt).min(1.0));
            self.pos += self.vel * dt;
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return true;
        }
        if self.slow_t > 0.0 {
            self.slow_t = (self.slow_t - dt).max(0.0);
            if self.slow_t <= 0.0 {
                self.slow_mult = 1.0;
            }
        }
        false
    }

    /// Stun the body for `secs` (keeps the longer of the two).
    #[inline]
    pub fn stun_for(&mut self, secs: f32) {
        if secs > 0.0 {
            self.stun = self.stun.max(secs);
        }
    }

    /// Slow the body to `mult` of its speed for `secs` (keeps the stronger of
    /// the two effects; the slow lifts when its timer runs out).
    #[inline]
    pub fn slow_for(&mut self, mult: f32, secs: f32) {
        if secs > 0.0 {
            let mult = mult.clamp(0.05, 1.0);
            if self.slow_t <= 0.0 || mult < self.slow_mult {
                self.slow_mult = mult;
            }
            self.slow_t = self.slow_t.max(secs);
        }
    }

    /// Clear every crowd-control effect (used by tests and by a boss's death
    /// break, so nothing is left frozen after the horde flees).
    #[inline]
    pub fn clear_control(&mut self) {
        self.stun = 0.0;
        self.slow_mult = 1.0;
        self.slow_t = 0.0;
    }

    /// [`Enemy::spawn`] with a campaign level's [`EnemyMods`] applied.
    ///
    /// Draws exactly the same rng values as [`Enemy::spawn`] (it delegates to
    /// it), so a later level changes numbers, never the rng stream.
    pub fn spawn_with_mods(
        kind: EnemyKind,
        pos: Vec2,
        hp_mult: f32,
        mods: EnemyMods,
        rng: &mut impl Rng,
    ) -> Self {
        let mut e = Enemy::spawn(kind, pos, hp_mult, rng);
        e.damage = kind.damage() * mods.damage;
        e.speed_scale = mods.speed;
        e
    }

    /// Steer toward the player, routing around walls via the flow field.
    /// `flow_dir` is the pathfinding direction at this enemy's cell (ZERO
    /// inside the goal cell / off-grid, in which case we head straight at the
    /// player).
    pub fn steer(&mut self, target: Vec2, flow_dir: Vec2, dt: f32, speed_mult: f32) {
        // A wave-3 body in the middle of its own tell or hop is held in place:
        // its behaviour owns the position this tick (a curled Pillbug, a Bell
        // Flea between its two crosses, a Porter raising the cube). Wave 4's
        // snake, hornet and darting centipede are the same. The timers, the
        // flash and the contact grace still advance, so the body keeps taking
        // damage and its own state machine keeps running. Dead cheap for the
        // older kinds: both `holds()` are false.
        if self.w3.holds() || self.w4.holds(self.kind) {
            self.vel = self.vel * (1.0 - (12.0 * dt).min(1.0));
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return;
        }

        // Crowd control (the Sonic Squeak's ring): a stunned body does not steer
        // and its momentum bleeds off fast, so it stops dead instead of sliding
        // past the rat. The slow is a straight multiplier on the per-kind speed
        // below. Both are no-ops at their defaults (0 / 1.0), so the existing
        // simulation is byte-for-byte unchanged.
        if self.tick_control(dt) {
            return;
        }

        if self.kind.has_outfall_state() && self.position_locked {
            self.vel = Vec2::ZERO;
            if self.flash > 0.0 {
                self.flash -= dt;
            }
            if self.contact_cd > 0.0 {
                self.contact_cd -= dt;
            }
            return;
        }

        let to = target - self.pos;
        let dist = to.len();
        let direct = to.normalized();
        // Follow the route whenever the field has an opinion. The field is
        // only ZERO inside the player's own cell (and off-grid), i.e. exactly
        // where the straight line cannot cross a wall, so the un-gridded last
        // stretch costs nothing in look: the velocity smoothing below hides
        // the grid. Taking the direct line within 26 px instead used to drag
        // enemies into props, where wall resolution parked them for good.
        let chase = if flow_dir.len_sq() > 0.001 {
            flow_dir
        } else {
            direct
        };
        let speed = self.kind.speed() * self.speed_scale * speed_mult * self.slow_mult;
        // Steering and gait are the **archetype's**: a registration flies, kites
        // or walks exactly as the base body it names, until its own batch.
        let archetype = self.kind.archetype();
        let desired = match archetype {
            EnemyKind::Bat | EnemyKind::Lampfish => {
                self.wander += dt * 4.0;
                (chase + Vec2::from_angle(self.wander) * 0.6).normalized() * speed
            }
            EnemyKind::Spitter => {
                // Keep a stand-off range: approach via the route, retreat directly.
                if dist > 110.0 {
                    chase * speed
                } else if dist < 80.0 {
                    direct * (-speed)
                } else {
                    chase.perp() * speed * 0.5
                }
            }
            _ => chase * speed,
        };
        // Smooth toward desired velocity.
        self.vel += (desired - self.vel) * (6.0 * dt).min(1.0);
        self.pos += self.vel * dt;

        // Animation + facing.
        let sp = self.vel.len();
        // Flyers buzz constantly; crawlers scuttle in proportion to speed.
        let cadence = if matches!(archetype, EnemyKind::Bat | EnemyKind::Lampfish) {
            22.0
        } else {
            4.0 + sp * 0.5
        };
        self.anim += dt * cadence;
        if sp > 1.0 {
            let turn_rate = if self.kind == EnemyKind::Conger { 4.0 } else { 8.0 };
            self.facing += (self.vel.normalized() - self.facing) * (turn_rate * dt).min(1.0);
        }

        if self.flash > 0.0 {
            self.flash -= dt;
        }
        if self.contact_cd > 0.0 {
            self.contact_cd -= dt;
        }
    }

    pub fn hurt(&mut self, dmg: f32) {
        self.apply_damage(dmg, None);
    }

    /// Land `dmg` after the kind's own incoming-damage rule and return what was
    /// actually taken (so `hurt_from`'s caller can draw and record the truth).
    ///
    /// The Termite Horde has a per-hit cap
    /// ([`crate::game::ecs::systems::enemy_wave4::HORDE_HIT_CAP`]): one hit can
    /// remove at most a capped fraction of its pool, so a single-target dart
    /// only chips the swarm while a many-hit area volley (`rain`, a nova, a
    /// thrash sweep, a pool's continuous ticks) still tears through it. The
    /// same pass spots a hard enough hit and raises the split flag the wave-4
    /// system turns into a second, smaller horde.
    ///
    /// Bunched Fry School takes double damage from any direction.
    fn apply_damage(&mut self, dmg: f32, from: Option<Vec2>) -> f32 {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        if self.buried {
            return 0.0;
        }
        let mult = from.map_or_else(
            || self.outfall_damage_mult(),
            |point| self.damage_mult_from(point),
        );
        let mut dmg = (dmg * mult).max(0.0);
        if self.kind.archetype() == EnemyKind::TermiteHorde {
            if self.hp > 0.0
                && self.w4.tier >= w4::HORDE_SPLIT_TIER
                && dmg >= self.max_hp * w4::HORDE_SPLIT_FRACTION
            {
                self.w4.split_pending = 1;
            }
            dmg = dmg.min(self.max_hp * w4::HORDE_HIT_CAP);
        }
        self.hp -= dmg;
        self.flash = 0.12;
        self.note_damage();
        dmg
    }

    /// `hurt` for a hit that arrives **from** a point in world space, with the
    /// kind's directional armour applied ([`Enemy::damage_mult_from`]). Returns
    /// the damage actually taken, which is what a caller should draw and record.
    pub fn hurt_from(&mut self, dmg: f32, from: Vec2) -> f32 {
        self.apply_damage(dmg, Some(from))
    }

    /// The Pillbug's shell: while it is curled, a hit arriving from its frontal
    /// 120° arc is quartered and anything else (the exposed rear) doubles.
    ///
    /// Every damage source in the game routes through here — bullets, pools, the
    /// melee arc, the actives, traps, boss hazards and the nuke — so the shell
    /// cannot be bypassed by picking a different weapon. The arc is measured off
    /// [`Enemy::facing`], which the curl locks when the tell starts.
    #[inline]
    pub fn damage_mult_from(&self, from: Vec2) -> f32 {
        use crate::game::ecs::systems::enemy_wave4 as w4;
        // The guest's exposed second: the 1.3 s recovery after a peck volley
        // takes x1.5 from every source, the same mechanism the Pillbug's rear
        // and the hornet's grounded punish use. Keyed on the **real** kind
        // (its archetype is the Brute, which has no armour of its own) and read
        // off the `w4` mirror `drake_system` keeps, so a source holding only an
        // `&Enemy` still gets the right answer.
        if self.kind == EnemyKind::Drake {
            return if self.w4.phase == crate::game::ecs::systems::drake::PHASE_RECOVER {
                crate::game::ecs::systems::drake::RECOVER_DAMAGE_MULT
            } else {
                1.0
            };
        }
        match self.kind.archetype() {
            EnemyKind::Pillbug => {
                if self.w3.curl_t <= 0.0 {
                    return 1.0;
                }
                let to_hit = from - self.pos;
                if to_hit.len_sq() <= 1e-6 {
                    return 1.0;
                }
                let f = self.facing;
                if f.len_sq() <= 1e-6 {
                    return 1.0;
                }
                // cos(half of 120°) = 0.5, so `dot >= 0.5` is the shell's cone.
                if to_hit.normalized().dot(f.normalized()) >= 0.5 {
                    crate::game::ecs::systems::enemy_wave3::PILLBUG_FRONT_MULT
                } else {
                    crate::game::ecs::systems::enemy_wave3::PILLBUG_REAR_MULT
                }
            }
            // The snake's head is the soft part: every hit on it lands harder
            // than the same hit on the scale-armoured body.
            EnemyKind::BigSnake => {
                if from.dist_sq(self.pos) <= w4::SNAKE_HEAD_SWEET_SPOT * w4::SNAKE_HEAD_SWEET_SPOT {
                    w4::SNAKE_HEAD_MULT
                } else {
                    1.0
                }
            }
            // The spear line: a wall of tiny shields and levelled points in
            // front, nothing at all behind.
            EnemyKind::Lilliputian => {
                let to_hit = from - self.pos;
                if to_hit.len_sq() <= 1e-6 || self.facing.len_sq() <= 1e-6 {
                    return 1.0;
                }
                if to_hit.normalized().dot(self.facing.normalized()) >= 0.4 {
                    w4::LILLI_FRONT_MULT
                } else {
                    w4::LILLI_REAR_MULT
                }
            }
            // The hornet on the ground after a missed dive is the punish
            // window, and it says so with a number.
            EnemyKind::MurderHornet
                if self.w4.phase == w4::HORNET_PHASE_RECOVER => {
                    w4::HORNET_RECOVER_MULT
                }
            _ => self.outfall_damage_mult(),
        }
    }

    /// Fry School is vulnerable from every direction while bunched. Both
    /// direct hits and pools read this multiplier through their damage paths.
    fn outfall_damage_mult(&self) -> f32 {
        if self.kind == EnemyKind::FrySchool && self.outfall.bunched {
            2.0
        } else {
            1.0
        }
    }

    /// Book a damaging hit's side effects without touching the flash tint.
    ///
    /// Used by the damage-over-time path (acid pools keep their own softer
    /// flash) and by the nuke, so a Porter still flinches and a Wick Moth still
    /// arms its fuse however it died. Draws no rng.
    pub fn note_damage(&mut self) {
        // Every side effect below belongs to the **archetype's** rule, so a
        // registration reacts to a hit exactly as its base body does.
        let archetype = self.kind.archetype();
        // The Porter drops its raise and has to try again (a flinch window, so
        // one stray shot does not disable it for its whole 5 s cycle).
        if archetype == EnemyKind::Porter && self.w3.raise_t > 0.0 {
            self.w3.raise_t = 0.0;
            self.w3.heal_target = u64::MAX;
            self.w3.heal_cd = self.w3.heal_cd.max(
                crate::game::ecs::systems::enemy_wave3::PORTER_INTERRUPT_COOLDOWN,
            );
        }
        // A Wick Moth that dies lights its own fuse, whatever killed it.
        if archetype == EnemyKind::WickMoth && self.hp <= 0.0 && self.w3.wick_t == 0.0 {
            self.w3.wick_t = crate::game::ecs::systems::enemy_wave3::WICK_FUSE_SECS;
        }
        // A centipede bolts for the nearest wall the moment anything touches
        // it: that is the whole "hard to pin down" clause, and it is booked
        // here so every damage source (a pool tick, a boss hazard, a trap) makes
        // it run, not just bullets.
        if archetype == EnemyKind::HouseCentipede && self.hp > 0.0 {
            self.w4.flee_t = self.w4.flee_t.max(
                crate::game::ecs::systems::enemy_wave4::CENTIPEDE_FLEE_SECS,
            );
        }
        // Any hit on the horde restarts its regrowth clock.
        if archetype == EnemyKind::TermiteHorde {
            self.w4.regrow_t = 0.0;
        }
        // A hit on a rope carrier drops its rope: killing the crew is one of
        // the two ways out of a knot (the other is the dash).
        if archetype == EnemyKind::Lilliputian
            && self.w4.role == crate::game::ecs::systems::enemy_wave4::LILLI_ROLE_ROPE
            && (self.w4.phase == crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_TELL
                || self.w4.phase == crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_ACTION)
        {
            self.w4.phase = crate::game::ecs::systems::enemy_wave4::LILLI_PHASE_WALK;
            self.w4.t = 0.0;
        }
    }

    #[inline]
    pub fn targetable(&self) -> bool {
        self.alive() && !self.buried
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::collision;
    use crate::game::level::{Level, Rectf, Theme};
    use crate::game::nav::{test_tiles, FlowField, NavBudget};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn direct_chase_needs_an_empty_flow() {
        let mut rng = StdRng::seed_from_u64(7);
        let target = Vec2::new(20.0, 0.0); // well inside the old 26 px shortcut
        let route = Vec2::new(0.0, 1.0); // the field says "go around"
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, route, 0.2, 1.0);
        assert!(
            e.vel.y > 5.0 && e.vel.x.abs() < 1.0,
            "must keep following the route on the last stretch: {:?}",
            e.vel
        );

        // With no flow (inside the goal cell) it heads straight in.
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::ZERO, 1.0, &mut rng);
        e.steer(target, Vec2::ZERO, 0.2, 1.0);
        assert!(
            e.vel.x > 5.0 && e.vel.y.abs() < 1.0,
            "an empty flow means the straight line is clear: {:?}",
            e.vel
        );
    }

    #[test]
    fn enemy_behind_a_prop_is_not_dragged_into_it() {
        // Player just right of a barrier, enemy just left of it: the straight
        // line crosses the prop, the flow must bend around it.
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let tiles = test_tiles(20, 20, &[wall]);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(122.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
        let mut rng = StdRng::seed_from_u64(3);
        let mut e = Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, 73.0), 1.0, &mut rng);
        assert!(
            e.pos.dist(player) <= 30.0,
            "the case under test is a short hop"
        );
        let flow = f.dir_at(collision::SizeClass::Small, e.pos);
        assert!(flow.len() > 0.5, "the flow should route around the prop");
        let before = e.pos.dist(player);
        // The wall is five cells tall, so the detour around it is longer than
        // half a second of a skitterer's life: give it five.
        for _ in 0..300 {
            let flow = f.dir_at(collision::SizeClass::Small, e.pos);
            e.steer(player, flow, 1.0 / 60.0, 1.0);
            e.pos = collision::resolve_circle_rect(e.pos, e.radius, &wall);
        }
        // It must not have been pushed into the prop (which would be the
        // straight-line tell) and must not have lost ground.
        assert!(
            !wall.contains(e.pos),
            "enemy ended inside the prop at {:?}",
            e.pos
        );
        assert!(e.pos.dist(player) <= before, "enemy stalled at {:?}", e.pos);
    }

    #[test]
    fn skitterers_parked_in_a_prop_halo_reach_the_player() {
        // The reported stall: 20 skitterers tucked behind a 16x66 barrier.
        let wall = Rectf::new(96.0, 32.0, 16.0, 80.0);
        let walls = [wall];
        let tiles = test_tiles(20, 20, &walls);
        let mut f = FlowField::new(&tiles);
        let player = Vec2::new(200.0, 73.0);
        f.update(&tiles, player, NavBudget::FULL);
        let mut rng = StdRng::seed_from_u64(11);
        let mut enemies: Vec<Enemy> = (0..20)
            .map(|i| {
                let y = 46.0 + i as f32 * 3.0;
                Enemy::spawn(EnemyKind::Skitterer, Vec2::new(94.0, y), 1.0, &mut rng)
            })
            .collect();
        let dt = 1.0 / 60.0;
        for _ in 0..20 * 60 {
            for e in enemies.iter_mut() {
                let flow = f.dir_at(collision::SizeClass::Small, e.pos);
                e.steer(player, flow, dt, 1.0);
                e.pos = e.pos.clamp_box(Vec2::splat(2.0), Vec2::new(318.0, 318.0));
                for w in &walls {
                    e.pos = collision::resolve_circle_rect(e.pos, e.radius, w);
                }
            }
        }
        let stuck: Vec<Vec2> = enemies
            .iter()
            .filter(|e| e.pos.dist(player) > 24.0)
            .map(|e| e.pos)
            .collect();
        assert!(stuck.is_empty(), "still stuck after 20 s: {stuck:?}");
    }

    #[test]
    fn enemies_reach_the_player_on_every_theme() {
        // Same promise on the real maps: nobody is left in a blocked cell.
        //
        // The map is 3200 × 2560 px now, so a skitterer (30 px/s) cannot cross
        // the whole arena in 60 s of simulated time: the enemies are scattered
        // inside a ring around the rat — the director spawns them a few dozen
        // pixels out, and the flow field's fine window covers 384 px — which
        // still exercises walls, corners and doorways. (Beyond the fine window
        // the coarse block field is best-effort: a point whose block-graph exit
        // needs a detour *inside* its own 8 × 8-cell block can stall.)
        for theme in Theme::all() {
            let level = Level::new(theme);
            let mut f = FlowField::new(&level.tiles);
            let player = level.arena * 0.5;
            f.update(&level.tiles, player, NavBudget::FULL);
            let mut rng = StdRng::seed_from_u64(5);
            let mut enemies: Vec<Enemy> = (0..12)
                .map(|_| {
                    let angle = rng.random_range(0.0..std::f32::consts::TAU);
                    let radius = rng.random_range(60.0..350.0);
                    let p = player + Vec2::from_angle(angle) * radius;
                    Enemy::spawn(EnemyKind::Skitterer, p, 1.0, &mut rng)
                })
                .collect();
            let dt = 1.0 / 60.0;
            for _ in 0..60 * 60 {
                for e in enemies.iter_mut() {
                    let flow = f.dir_at(collision::SizeClass::Small, e.pos);
                    e.steer(player, flow, dt, 1.0);
                    e.pos = e
                        .pos
                        .clamp_box(Vec2::splat(2.0), level.arena - Vec2::splat(2.0));
                    e.pos = level.tiles.resolve_circle(
                        e.pos,
                        e.radius,
                        collision::SizeClass::of_enemy(e.kind),
                        None,
                    );
                }
            }
            let stuck: Vec<Vec2> = enemies
                .iter()
                .filter(|e| e.pos.dist(player) > 40.0)
                .map(|e| e.pos)
                .collect();
            assert!(stuck.is_empty(), "{theme:?}: stuck enemies at {stuck:?}");
        }
    }

    /// The console table follows the enum order, without duplicates.
    #[test]
    fn all_lists_every_kind_once() {
        let all = EnemyKind::all();
        assert_eq!(
            all.len(),
            58,
            "16 earlier kinds + 33 registrations + the guest + eight Outfall bodies"
        );
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "{a:?} is listed twice");
            }
        }
        assert_eq!(
            all.iter().filter(|k| k.is_registration()).count(),
            33,
            "the batch registers 33 ordinary kinds (the guest is not one)"
        );
        // The guest keeps its original ordinal and is nobody's roster slot.
        assert_eq!(all[49], EnemyKind::Drake);
        assert!(!EnemyKind::Drake.is_registration());
        assert_eq!(EnemyKind::Drake.archetype(), EnemyKind::Brute);
    }

    #[test]
    fn outfall_ordinary_stats_and_archetypes() {
        use EnemyKind::*;

        // Slot order doubles as the append-only save index.
        let rows = [
            (SandHopper, 6.0, 30.0, 5.0, 1, 2.5, Skitterer),
            (GlassShrimp, 4.0, 42.0, 4.0, 1, 2.0, Bat),
            (Conger, 16.0, 38.0, 8.0, 3, 4.0, Cat),
            (Skater, 14.0, 12.0, 5.0, 3, 3.0, Skater),
            (PistolShrimp, 12.0, 22.0, 6.0, 4, 3.0, Spitter),
            (Lampfish, 11.0, 24.0, 5.0, 3, 3.0, Lampfish),
            (Grouper, 60.0, 16.0, 15.0, 10, 6.5, Brute),
            (FrySchool, 26.0, 22.0, 5.0, 3, 12.0, FrySchool),
        ];
        let all = EnemyKind::all();
        for (i, (kind, hp, speed, damage, xp, radius, archetype)) in rows.into_iter().enumerate() {
            assert_eq!(all[50 + i], kind);
            assert_eq!(kind as usize, 50 + i);
            assert_eq!(crate::game::roster::floor(Theme::Outfall).slots[i].kind, kind);
            assert_eq!(
                (kind.base_hp(), kind.speed(), kind.damage(), kind.xp(), kind.radius()),
                (hp, speed, damage, xp, radius),
            );
            assert_eq!(kind.archetype(), archetype);
            assert_eq!(kind.ranged(), kind == PistolShrimp);
            assert_eq!(kind.heavy(), kind == Grouper);
            assert!(!kind.is_wave3() && !kind.is_wave4());
        }
    }

    #[test]
    fn conger_turns_its_facing_at_half_the_cats_rate() {
        let mut cat = Enemy::spawn_with_phases(EnemyKind::Cat, Vec2::ZERO, 1.0, 2.2, 0.0, 0.0);
        let mut conger = Enemy::spawn_with_phases(EnemyKind::Conger, Vec2::ZERO, 1.0, 2.2, 0.0, 0.0);
        let north = Vec2::new(0.0, -20.0);
        cat.steer(north, Vec2::ZERO, 0.05, 1.0);
        conger.steer(north, Vec2::ZERO, 0.05, 1.0);
        assert_eq!(cat.vel, conger.vel);
        assert_eq!(cat.facing.y, conger.facing.y * 2.0);
    }

    /// Every registration inherits from a kind that already existed, the
    /// inheritance is idempotent, and it really is the base body's collision
    /// radius (and so its navigation size class).
    #[test]
    fn every_registration_names_an_existing_archetype() {
        let all = EnemyKind::all();
        let existing = &all[..16];
        for &k in all.iter().filter(|k| k.is_registration()) {
            let base = k.archetype();
            assert!(
                existing.contains(&base),
                "{k:?} inherits {base:?}, which is not one of the sixteen existing kinds"
            );
            assert_eq!(base.archetype(), base, "{base:?} is not its own archetype");
            assert_eq!(
                k.archetype().archetype(),
                base,
                "archetype() is not idempotent for {k:?}"
            );
            assert_eq!(
                k.radius(),
                base.radius(),
                "{k:?} must inherit its base body's collision radius"
            );
            assert_eq!(
                k.heavy(),
                base.heavy(),
                "{k:?} must be exactly as immovable as {base:?}"
            );
            assert_eq!(k.ranged(), base.ranged(), "{k:?} must shoot like {base:?}");
        }
        for &k in existing {
            assert_eq!(k.archetype(), k, "an existing kind is its own archetype");
            assert!(!k.is_registration(), "{k:?} is not a registration");
        }
    }

    /// Three sampled kinds per floor carry their `WAVE4_ROSTERS.md` row exactly
    /// (HP / speed / contact / XP): the design's numbers are the kind's own,
    /// not its archetype's.
    #[test]
    fn sampled_registrations_match_the_design_table() {
        use EnemyKind::*;
        // (kind, hp, px/s, contact, xp), three rows per floor.
        let rows: [(EnemyKind, f32, f32, f32, u32); 18] = [
            // Sewer.
            (PipeEel, 16.0, 39.0, 8.0, 3),
            (GrateArcher, 12.0, 22.0, 6.0, 4),
            (SluiceToad, 58.0, 16.0, 15.0, 10),
            // Kitchen.
            (CrumbAnt, 5.0, 31.0, 5.0, 1),
            (PepperMill, 13.0, 20.0, 6.0, 4),
            (PieWeight, 60.0, 16.0, 15.0, 10),
            // Lab.
            (SwabFoot, 6.0, 29.0, 5.0, 1),
            (WalkingSyringe, 13.0, 25.0, 5.0, 3),
            (AutoclaveCart, 60.0, 15.0, 16.0, 10),
            // Laundry.
            (WoolPill, 5.0, 34.0, 5.0, 1),
            (StarchBulb, 12.0, 21.0, 6.0, 4),
            (LintGolem, 58.0, 16.0, 16.0, 10),
            // Printworks.
            (Silverfish, 5.0, 32.0, 5.0, 1),
            (TypeSlug, 18.0, 39.0, 9.0, 3),
            (PaperReel, 62.0, 15.0, 15.0, 10),
            // Flatbed.
            (GravelTick, 6.0, 30.0, 5.0, 1),
            (TollGull, 12.0, 22.0, 6.0, 4),
            (LooseBungee, 20.0, 25.0, 6.0, 4),
        ];
        for (kind, hp, speed, contact, xp) in rows {
            assert_eq!(kind.base_hp(), hp, "{kind:?} HP");
            assert_eq!(kind.speed(), speed, "{kind:?} speed");
            assert_eq!(kind.damage(), contact, "{kind:?} contact");
            assert_eq!(kind.xp(), xp, "{kind:?} XP");
        }
        // The gold column, chance × pieces: the `WAVE4_ROSTERS.md` chances cut
        // to about a third by the batch-six income pass and by a further
        // ×0.72 in batch seven, read from the shop's own row.
        use crate::game::shop;
        assert_eq!(shop::gold_row(EnemyKind::PipeEel), (0.043, 2));
        assert_eq!(shop::gold_row(EnemyKind::CrumbAnt), (0.022, 1));
        assert_eq!(shop::gold_row(EnemyKind::PaperReel), (0.108, 3));
        assert_eq!(shop::gold_row(EnemyKind::LooseBungee), (0.079, 3));
    }

    /// One floor's registrations, in roster order (`Boss` is nobody's slot).
    fn floor_registrations(theme: Theme) -> Vec<EnemyKind> {
        crate::game::roster::floor(theme)
            .slots
            .iter()
            .map(|s| s.kind)
            .filter(|k| k.is_registration())
            .collect()
    }

    /// One floor's new kinds spawn, steer, take damage and die through a few
    /// hundred ticks on their own floor without panicking.
    ///
    /// The ticks run the real schedule (`World::update_with`), so this also
    /// exercises the archetype inheritance the registrations lean on: a Sump
    /// Leech runs the Pillbug's curl, a Loose Can the hornet's dive, and so on.
    /// The rat neither moves nor attacks, so every body that dies is killed by
    /// the floor itself (a pit, a steam plume) — which is the point of running
    /// it rather than simulating a bare `Enemy`.
    fn exercise_floor(theme: Theme) {
        use crate::game::world::World;
        use rand::rngs::StdRng;
        use rand::SeedableRng;

        let kinds = floor_registrations(theme);
        assert!(!kinds.is_empty(), "{theme:?} adds no registrations");

        let mut w = World::new(theme, 13);
        let p = w.player.pos;
        let mut rng = StdRng::seed_from_u64(17);
        for (i, &kind) in kinds.iter().enumerate() {
            let a = i as f32 / kinds.len() as f32 * std::f32::consts::TAU;
            // A ring 48 px out from the rat: on screen, wall-resolved by the
            // mover, and close enough that every body is chasing within a tick.
            let at = p + Vec2::from_angle(a) * 48.0;
            w.spawn_enemy(kind, at, 1.0);
        }
        assert!(
            w.enemies().len() >= kinds.len(),
            "{theme:?}: only {} of {} bodies spawned",
            w.enemies().len(),
            kinds.len()
        );

        for _ in 0..300 {
            w.update_with(1.0 / 60.0, Vec2::ZERO, false, false);
        }

        // The run is coherent: nothing went non-finite and nothing healed past
        // its pool. (The rat neither moves nor attacks, so this is the floor's
        // own hazards and bodies doing all of the moving and hitting.)
        assert!(
            w.player.hp.is_finite() && w.player.pos.x.is_finite(),
            "{theme:?}: the rat's state went non-finite"
        );
        for e in w.enemies() {
            assert!(
                e.pos.x.is_finite() && e.pos.y.is_finite(),
                "{:?} on {theme:?} has a non-finite position",
                e.kind
            );
            assert!(
                e.hp <= e.max_hp + 1e-3,
                "{:?} on {theme:?} healed past its pool",
                e.kind
            );
            assert!(
                e.pos.x >= 0.0
                    && e.pos.y >= 0.0
                    && e.pos.x <= w.level.arena.x
                    && e.pos.y <= w.level.arena.y,
                "{:?} on {theme:?} left the arena",
                e.kind
            );
        }

        // And every one of them takes a hit and dies through the ordinary
        // damage path (the same `hurt` every weapon, pool and trap calls).
        for &kind in kinds.iter() {
            let mut e = Enemy::spawn(kind, Vec2::ZERO, 1.0, &mut rng);
            let before = e.hp;
            e.hurt(before + 1.0);
            assert!(e.hp <= 0.0, "{kind:?} survived a lethal hit");
            assert!(!e.alive(), "{kind:?} still reports alive");
        }
    }

    #[test]
    fn sewer_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Sewer);
    }

    #[test]
    fn kitchen_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Kitchen);
    }

    #[test]
    fn lab_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Lab);
    }

    #[test]
    fn laundry_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Laundry);
    }

    #[test]
    fn printworks_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Printworks);
    }

    #[test]
    fn flatbed_registrations_run_on_their_own_floor() {
        exercise_floor(Theme::Trailer);
    }
}
