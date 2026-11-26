//! Half-block pixel icons for weapons, passives, loot, and evolved frames.
//!
//! Every [`Icon`] carries two hand-authored half-block pixel grids:
//! - **Large** (`8 × 8` square pixels = `8 × 4` terminal cells) for level-up
//!   cards and showcase panels.
//! - **Small** (`4 × 4` square pixels = `4 × 2` terminal cells) for the HUD
//!   weapon/passive rack, compact level-up cards (`57 × 20`), the pause
//!   dossier, and the campaign carry-over manifest.
//!
//! Both forms are authored explicitly (never auto-shrunk) with top-left
//! directional lighting matching `art_rat.rs` and `render/cursor.rs`, using
//! each item's established in-game colour family and tuned so 256-colour and
//! 16-colour fallbacks stay vivid and readable.
//!
//! # Zero-Allocation Contract
//!
//! Every [`Icon`] lives in static read-only memory (`const`). [`draw`] and its
//! siblings write directly into ratatui's [`Buffer`], composite transparent
//! pixels over existing cells (preserving half-block pixels and UI plate
//! backgrounds), clip cleanly at every edge, emit only safe block glyphs
//! (`▀` / `▄`), and never allocate.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::game::loadout::PassiveKind;
use crate::game::weapon::WeaponKind;
use crate::render::palette::{self, Rgb};
use crate::term::ColorDepth;

/// Large icon dimensions in half-block pixels (`8 × 8` square pixels).
pub const LARGE_PX_W: usize = 8;
pub const LARGE_PX_H: usize = 8;
/// Large icon footprint in terminal cells (`8 cols × 4 rows`).
pub const LARGE_CELL_W: u16 = LARGE_PX_W as u16;
pub const LARGE_CELL_H: u16 = (LARGE_PX_H / 2) as u16;

/// Small icon dimensions in half-block pixels (`4 × 4` square pixels).
pub const SMALL_PX_W: usize = 4;
pub const SMALL_PX_H: usize = 4;
/// Small icon footprint in terminal cells (`4 cols × 2 rows`).
pub const SMALL_CELL_W: u16 = SMALL_PX_W as u16;
pub const SMALL_CELL_H: u16 = (SMALL_PX_H / 2) as u16;

/// Framed evolved showcase dimensions (`10 × 10` pixels = `10 × 5` cells).
pub const EVOLVED_FRAME_CELL_W: u16 = 10;
pub const EVOLVED_FRAME_CELL_H: u16 = 5;

/// Fixed half-block pixel icon in both large (`8 × 8` px) and small (`4 × 4` px)
/// hand-authored forms. `None` entries are transparent and preserve whatever is
/// beneath them in the terminal buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    pub large: [[Option<Rgb>; LARGE_PX_W]; LARGE_PX_H],
    pub small: [[Option<Rgb>; SMALL_PX_W]; SMALL_PX_H],
    /// Representative UI accent colour for borders, labels, and badges.
    pub accent: Rgb,
}

// ---- Compile-time sprite compiler -----------------------------------------

const fn lookup_key(ch: u8, pal: &[(u8, Rgb)]) -> Option<Rgb> {
    if ch == b'.' || ch == b' ' {
        return None;
    }
    let mut i = 0;
    while i < pal.len() {
        if pal[i].0 == ch {
            return Some(pal[i].1);
        }
        i += 1;
    }
    panic!("unknown palette key in icon sprite")
}

const fn grid8(rows: [&[u8; 8]; 8], pal: &[(u8, Rgb)]) -> [[Option<Rgb>; 8]; 8] {
    let mut out = [[None; 8]; 8];
    let mut y = 0;
    while y < 8 {
        let mut x = 0;
        while x < 8 {
            out[y][x] = lookup_key(rows[y][x], pal);
            x += 1;
        }
        y += 1;
    }
    out
}

const fn grid4(rows: [&[u8; 4]; 4], pal: &[(u8, Rgb)]) -> [[Option<Rgb>; 4]; 4] {
    let mut out = [[None; 4]; 4];
    let mut y = 0;
    while y < 4 {
        let mut x = 0;
        while x < 4 {
            out[y][x] = lookup_key(rows[y][x], pal);
            x += 1;
        }
        y += 1;
    }
    out
}

// ============================================================================
// Existing Weapons (8)
// ============================================================================

/// `WeaponKind::Gnaw` — twin ivory rat incisors beneath a pink gum arch.
pub const GNAW: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'G', (244, 82, 114)),  // gum pink (LightRed/Red in 16c)
        (b'D', (168, 42, 68)),   // gum shadow (Red in 16c)
        (b'W', (255, 254, 238)), // enamel highlight
        (b'I', (255, 236, 152)), // ivory tooth (WeaponKind::Gnaw)
        (b'S', (206, 174, 88)),  // tooth shadow
    ];
    Icon {
        large: grid8(
            [
                b".GGGGGG.",
                b"GWWGGWWD",
                b".WII.WII",
                b".WII.WIS",
                b".WIS.WIS",
                b"..IS..IS",
                b"..W...W.",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"GGGD", b"WIWI", b"WSWS", b".W.W"], PAL),
        accent: (255, 240, 180),
    }
};

/// `WeaponKind::CheeseSpray` — Swiss cheese wedge with holes & flying crumbs.
pub const CHEESE_SPRAY: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'S', (255, 252, 206)), // flying crumb sparkle
        (b'T', (255, 244, 114)), // sunlit cheese tip (LightYellow in 16c)
        (b'H', (255, 224, 90)),  // lit slope
        (b'B', (255, 204, 72)),  // body (WeaponKind::CheeseSpray)
        (b'R', (202, 132, 22)),  // golden-ochre rind (Yellow in 16c)
        (b'O', (124, 68, 14)),   // Swiss hole (Yellow in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"S....TRR",
                b"....THHR",
                b"..STHBOR",
                b"...THBBR",
                b".TTHOBBR",
                b"TBBBBBOR",
                b"BBBBBBBR",
                b".RRRRRRR",
            ],
            PAL,
        ),
        small: grid4([b"..TR", b".HOR", b"TBBR", b".RRR"], PAL),
        accent: (255, 210, 90),
    }
};

/// `WeaponKind::SqueakNova` — radial 8-point violet starburst shockwave.
pub const SQUEAK_NOVA: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (252, 242, 255)), // white-lavender core
        (b'L', (214, 108, 255)), // electric lavender (LightMagenta in 16c)
        (b'V', (174, 62, 248)),  // vivid violet (LightMagenta/Magenta in 16c)
        (b'D', (118, 34, 188)),  // deep purple tip (Magenta in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"...LV...",
                b".D.LW.D.",
                b"..VLWV..",
                b"LLWWWWLL",
                b"VVWWWWVV",
                b"..VLWV..",
                b".D.VW.D.",
                b"...VD...",
            ],
            PAL,
        ),
        small: grid4([b"L..V", b".WW.", b".WW.", b"V..D"], PAL),
        accent: (200, 160, 255),
    }
};

/// `WeaponKind::SporeOrbit` — three glowing mint spores circling a teal orbit.
pub const SPORE_ORBIT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (206, 255, 212)), // bioluminescent highlight
        (b'M', (76, 246, 118)),  // mint spore (LightGreen in 16c)
        (b'G', (36, 178, 86)),   // emerald spore shadow (Green in 16c)
        (b'T', (26, 108, 96)),   // orbital trail ring (Cyan/Green in 16c)
        (b'C', (178, 214, 206)), // center anchor
    ];
    Icon {
        large: grid8(
            [
                b"..WMG...",
                b".TMMGT..",
                b"T.....T.",
                b"T..C..WM",
                b"T.....MG",
                b".T...T..",
                b".WMTG...",
                b"..MG....",
            ],
            PAL,
        ),
        small: grid4([b".WM.", b"T..M", b"M..T", b".GT."], PAL),
        accent: (140, 255, 180),
    }
};

/// `WeaponKind::TailWhip` — curled hot-pink rat tail snapping in a wide lash.
pub const TAIL_WHIP: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 236, 246)), // whip-tip flash
        (b'P', (255, 92, 186)),  // hot tail pink (LightMagenta in 16c)
        (b'R', (218, 52, 146)),  // rose midtone (Magenta/LightMagenta in 16c)
        (b'D', (146, 26, 92)),   // crimson-rose shadow (Magenta in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"..WWPP..",
                b".WPPPRR.",
                b".PR..DR.",
                b"..D..PR.",
                b"....PPR.",
                b".R.PPR..",
                b".RPPD...",
                b"..DD....",
            ],
            PAL,
        ),
        small: grid4([b"WPP.", b"..PR", b".PP.", b"RD.."], PAL),
        accent: (255, 150, 200),
    }
};

/// `WeaponKind::SpikedTail` — a curled steel rat tail bristling with bone-white
/// spikes along its outer spine (the lash's "spikes out, then the crack").
pub const SPIKED_TAIL: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 255, 255)), // spike tip, white-hot
        (b'S', (206, 214, 226)), // steel body (WeaponKind::SpikedTail)
        (b'G', (150, 160, 178)), // steel shade
        (b'D', (84, 92, 112)),   // dark iron underside
    ];
    Icon {
        large: grid8(
            [
                b"..W..W..",
                b".WSWSWS.",
                b"WSSGGSSW",
                b".G...DS.",
                b".....GSW",
                b"..W.SSG.",
                b"..DSSG..",
                b"..DD....",
            ],
            PAL,
        ),
        small: grid4([b".W.W", b"WSSG", b"..SW", b".DG."], PAL),
        accent: (206, 214, 226),
    }
};

/// `WeaponKind::AcidSpit` — dripping toxic lime glob over a bubbling pool.
pub const ACID_SPIT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'H', (162, 255, 84)), // chartreuse specular (LightGreen in 16c)
        (b'A', (92, 242, 48)),  // acid lime (LightGreen in 16c)
        (b'G', (46, 174, 28)),  // mid acid green (Green in 16c)
        (b'D', (24, 102, 18)),  // dark caustic pool rim (Green in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"...H....",
                b"..HAA...",
                b"..HAAA..",
                b"..AAAG..",
                b"...AG...",
                b".H....H.",
                b".AAGGAA.",
                b"DGGGGGGD",
            ],
            PAL,
        ),
        small: grid4([b".HA.", b".AG.", b"H..A", b"GGGG"], PAL),
        accent: (160, 255, 90),
    }
};

/// `WeaponKind::TailThrash` (ACTIVE) — fiery amber-orange cyclone double-sweep.
pub const TAIL_THRASH: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 246, 192)), // white-gold core
        (b'O', (255, 174, 72)),  // bright amber (WeaponKind::TailThrash)
        (b'F', (236, 106, 28)),  // flame orange
        (b'B', (164, 54, 16)),   // burnt orange arc
    ];
    Icon {
        large: grid8(
            [
                b"..WWOO..",
                b".WOOFF..",
                b"WO..BF.B",
                b"OF.WW.FO",
                b"BF.WW.FO",
                b"B.FB..OW",
                b"..FFOOW.",
                b"..OOWW..",
            ],
            PAL,
        ),
        small: grid4([b".WOO", b"F.WF", b"FW.F", b"OOW."], PAL),
        accent: (255, 178, 92),
    }
};

/// `WeaponKind::SonicSqueak` (ACTIVE) — concentric expanding cyan sonic ring.
pub const SONIC_SQUEAK: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (236, 255, 255)), // white-cyan center pulse
        (b'C', (104, 238, 255)), // electric cyan (WeaponKind::SonicSqueak)
        (b'T', (36, 158, 188)),  // deep aqua ring
    ];
    Icon {
        large: grid8(
            [
                b"..CCCC..",
                b".C....C.",
                b"C..TT..C",
                b"C.TWWC.C",
                b"C.CWWC.C",
                b"C..TT..C",
                b".C....C.",
                b"..TTTT..",
            ],
            PAL,
        ),
        small: grid4([b".CC.", b"CWWC", b"CWWC", b".TT."], PAL),
        accent: (140, 240, 255),
    }
};

// ============================================================================
// Existing Passives (7)
// ============================================================================

/// `PassiveKind::MaxHp` ("Fat Reserves") — plump crimson-rose heart.
pub const MAX_HP: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 224, 232)), // heart specular glint
        (b'H', (246, 76, 98)),   // bright heart red
        (b'R', (196, 36, 58)),   // crimson body
        (b'D', (124, 18, 36)),   // deep maroon shadow
    ];
    Icon {
        large: grid8(
            [
                b"........",
                b".HH..HH.",
                b"HWWHHHHR",
                b"HWHHHHHR",
                b".HHHHHR.",
                b"..HHHR..",
                b"...RD...",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"H..H", b"WHHR", b"HHRR", b".RD."], PAL),
        accent: (222, 120, 132),
    }
};

/// `PassiveKind::MoveSpeed` ("Quick Paws") — sky-blue twin sprint chevrons.
pub const MOVE_SPEED: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (224, 252, 255)), // ice-white leading edge
        (b'S', (56, 192, 255)),  // sky cyan-blue (LightCyan in 16c)
        (b'B', (34, 114, 242)),  // cerulean blue (LightBlue/Blue in 16c)
        (b'D', (24, 72, 176)),   // trailing motion streak (Blue in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"....WS..",
                b"..WS.WS.",
                b"DD.WS.WS",
                b"....WWSS",
                b"DD.BS.BS",
                b"..BS.BS.",
                b"....BD..",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b".W..", b"D.WS", b"D.BS", b".B.."], PAL),
        accent: (128, 206, 250),
    }
};

/// `PassiveKind::Damage` ("Sharp Teeth") — diagonal gleaming blade & crimson hilt.
pub const DAMAGE: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 238)), // razor highlight
        (b'S', (232, 212, 168)), // steel-gold blade
        (b'M', (154, 138, 118)), // shaded blade edge
        (b'R', (228, 62, 52)),   // crimson crossguard
        (b'G', (244, 186, 68)),  // gold pommel
    ];
    Icon {
        large: grid8(
            [
                b"......WW",
                b".....WWS",
                b"....WWSM",
                b".R.WWSM.",
                b"..RRSM..",
                b"..GRR...",
                b".G..R...",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"..WW", b".RSM", b".GR.", b"G..."], PAL),
        accent: (248, 214, 152),
    }
};

/// `PassiveKind::FireRate` ("Frenzy") — electric amber lightning bolt.
pub const FIRE_RATE: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 204)), // white-hot bolt core
        (b'Y', (255, 206, 48)),  // electric amber
        (b'O', (236, 130, 26)),  // deep orange edge
        (b'D', (152, 68, 14)),   // bolt shadow
    ];
    Icon {
        large: grid8(
            [
                b"....WWY.",
                b"...WYYO.",
                b"..WYYO..",
                b".WYYYYYO",
                b"..DDWYYO",
                b"...WYYO.",
                b"..WYYD..",
                b"..YO....",
            ],
            PAL,
        ),
        small: grid4([b"..WY", b"WYYO", b"..YO", b".OD."], PAL),
        accent: (242, 184, 102),
    }
};

/// `PassiveKind::Magnet` ("Whiskers") — horseshoe magnet pulling a cyan spark.
pub const MAGNET: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (244, 246, 255)), // bright steel pole tips
        (b'S', (168, 176, 194)), // silver pole base
        (b'P', (198, 82, 255)),  // magnetic violet (LightMagenta in 16c)
        (b'V', (130, 44, 208)),  // deep indigo body (Magenta in 16c)
        (b'C', (96, 238, 255)),  // pulled XP spark (LightCyan in 16c)
    ];
    Icon {
        large: grid8(
            [
                b".WW..WW.",
                b".SS.CSS.",
                b".PP..PV.",
                b".PP..PV.",
                b".PP..PV.",
                b".PPVVVV.",
                b"..VVVV..",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"W..W", b"P..V", b"P..V", b".VV."], PAL),
        accent: (176, 140, 240),
    }
};

/// `PassiveKind::Regen` ("Resilience") — glowing emerald vitality cross.
pub const REGEN: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (208, 255, 214)), // mint-white bevel highlight
        (b'G', (62, 236, 98)),   // vibrant emerald-mint (LightGreen in 16c)
        (b'E', (28, 166, 68)),   // rich emerald shade (Green in 16c)
        (b'D', (16, 98, 42)),    // dark forest edge (Green in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"..WWGE..",
                b"..WGGE..",
                b"WWGGGGGE",
                b"WGGWWGGE",
                b"GGGGGGEE",
                b"EGGGGEEE",
                b"..EGEE..",
                b"..DDEE..",
            ],
            PAL,
        ),
        small: grid4([b".WG.", b"WGGD", b"GGEE", b".GE."], PAL),
        accent: (126, 224, 152),
    }
};

/// `PassiveKind::Armor` ("Thick Hide") — heraldic beveled steel shield.
pub const ARMOR: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (236, 248, 255)), // specular steel top-left
        (b'L', (162, 202, 218)), // polished steel
        (b'S', (96, 134, 152)),  // slate steel
        (b'D', (48, 72, 86)),    // iron rim
    ];
    Icon {
        large: grid8(
            [
                b".WWWWLL.",
                b"WWLLLSSD",
                b"WLLWWSSD",
                b"WLWLLSSD",
                b".LLSSSD.",
                b".LSSSDD.",
                b"..SSDD..",
                b"...DD...",
            ],
            PAL,
        ),
        small: grid4([b"WWLL", b"WLSD", b".SSD", b".DD."], PAL),
        accent: (150, 190, 200),
    }
};

// ============================================================================
// Wave-3 Upcoming Weapons, Passives, Loot & Evolved Frame Constants
// ============================================================================

/// Upcoming weapon: **Acid Rain** — toxic storm cloud pouring neon-lime drops.
pub const ACID_RAIN: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'C', (126, 152, 134)), // lit storm cloud rim
        (b'S', (72, 92, 82)),    // heavy cloud body
        (b'K', (42, 56, 50)),    // cloud underbelly
        (b'H', (160, 255, 86)),  // bright acid glint (LightGreen in 16c)
        (b'A', (86, 240, 48)),   // toxic lime raindrop (LightGreen in 16c)
        (b'G', (42, 170, 28)),   // dark acid streak (Green in 16c)
    ];
    Icon {
        large: grid8(
            [
                b"..CCCC..",
                b".CSSSSC.",
                b"CSSSSSSK",
                b".KKKKKK.",
                b".H..H..H",
                b"A..A..A.",
                b"G.H..H.G",
                b"..A..A..",
            ],
            PAL,
        ),
        small: grid4([b".CC.", b"SSKK", b"H.H.", b".A.G"], PAL),
        accent: (154, 248, 82),
    }
};

/// `WeaponKind::Hantavirus` — symmetric crimson-violet spiked pathogen orb.
pub const HANTAVIRUS: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'Y', (252, 228, 72)),  // biohazard yellow spike tip
        (b'O', (186, 146, 26)),  // spike stalk
        (b'W', (255, 198, 222)), // glowing nucleus
        (b'P', (222, 48, 116)),  // viral capsid crimson-magenta
        (b'D', (134, 22, 68)),   // deep capsid shade
    ];
    Icon {
        large: grid8(
            [
                b"...YY...",
                b"Y..OO..Y",
                b".OPWWPO.",
                b"YOWPPWOY",
                b"YOPPPDOY",
                b".OPDDPO.",
                b"Y..DD..Y",
                b"...YY...",
            ],
            PAL,
        ),
        small: grid4([b".Y.Y", b"YWP.", b".PDY", b"Y.Y."], PAL),
        accent: (224, 68, 128),
    }
};

/// Upcoming weapon: **Laser Eyes** — twin crimson-hot eyes firing horizontal beams.
pub const LASER_EYES: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'K', (78, 74, 96)),    // rat brow silhouette
        (b'D', (44, 40, 56)),    // dark silhouette shade
        (b'W', (255, 248, 242)), // white-hot laser core
        (b'R', (255, 42, 60)),   // intense crimson beam
        (b'G', (184, 20, 42)),   // outer laser glow
    ];
    Icon {
        large: grid8(
            [
                b".KK.....",
                b"KDDGGGGG",
                b"DWWRWWWW",
                b"DDDRRRRR",
                b"KDD.....",
                b"DWWRWWWW",
                b"DDDRRRRR",
                b".DDGGGGG",
            ],
            PAL,
        ),
        small: grid4([b"KWWR", b"DGGG", b"KWWR", b"DGGG"], PAL),
        accent: (255, 68, 84),
    }
};

/// Upcoming weapon: **Super Dash** — blazing cyan-and-gold comet arrowhead.
pub const SUPER_DASH: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 220)), // white-hot comet core
        (b'C', (96, 238, 255)),  // cyan shock-cone nose
        (b'T', (38, 152, 212)),  // electric blue shock edge
        (b'O', (255, 166, 48)),  // blazing gold-orange wake
        (b'F', (224, 78, 24)),   // fiery tail spark
    ];
    Icon {
        large: grid8(
            [
                b"....TT..",
                b"..OO.CC.",
                b"FFOOWWWC",
                b".OOWWWWW",
                b".OOWWWWW",
                b"FFOOWWWC",
                b"..OO.CC.",
                b"....TT..",
            ],
            PAL,
        ),
        small: grid4([b"..TC", b"OOWC", b"FOWC", b"..TC"], PAL),
        accent: (110, 232, 255),
    }
};

/// `PassiveKind::SpringHeel` ("Spring Heel" / dash cooldown) — coiled brass
/// heel-spring launching a bright cyan rebound spark.
pub const DASH_COOLDOWN: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'G', (248, 206, 76)),  // lit brass coil
        (b'B', (174, 118, 24)),  // shaded back coil
        (b'W', (232, 255, 255)), // white-cyan launch glint
        (b'C', (92, 232, 255)),  // cyan dash spark
        (b'D', (78, 96, 112)),   // steel heel base plate
    ];
    Icon {
        large: grid8(
            [
                b"..C.WC..",
                b"..GGGG..",
                b".B...GG.",
                b".GGGG...",
                b"...BBGG.",
                b".GGGG...",
                b".B...BB.",
                b".DDDDDD.",
            ],
            PAL,
        ),
        small: grid4([b".CW.", b"GGG.", b".BGG", b"DDD."], PAL),
        accent: (116, 224, 248),
    }
};

/// `PassiveKind::Counterweight` — heavy copper washer tied with knotted twine.
pub const COUNTERWEIGHT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'K', (246, 234, 206)), // knotted hemp twine highlight
        (b'T', (176, 152, 118)), // twine shade
        (b'W', (255, 224, 152)), // sunlit copper bevel (LightYellow/Yellow in 16c)
        (b'C', (234, 136, 52)),  // hammered copper body
        (b'B', (158, 72, 24)),   // bronze mid-shadow
        (b'D', (98, 42, 14)),    // deep bronze underside
    ];
    Icon {
        large: grid8(
            [
                b"....KKT.",
                b"..WWCK..",
                b".WWCCKB.",
                b".WC..CB.",
                b".WC..CB.",
                b".WCCBBB.",
                b"..BDDD..",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b".WKK", b"WC.B", b"WC.B", b".BDD"], PAL),
        accent: (236, 154, 78),
    }
};

/// `PassiveKind::LongFuse` — coiled braided fuse burning with a bright star spark.
pub const LONG_FUSE: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 254, 214)), // white-hot spark core
        (b'Y', (255, 212, 62)),  // golden spark rays (LightYellow in 16c)
        (b'O', (244, 112, 34)),  // ember orange flare
        (b'R', (198, 48, 28)),   // burning coal neck (Red in 16c)
        (b'H', (206, 174, 118)), // lit braided hemp cord
        (b'D', (122, 92, 52)),   // shaded cord coil
    ];
    Icon {
        large: grid8(
            [
                b".....Y..",
                b"....YWY.",
                b".....OY.",
                b"..HHHR..",
                b".HD..DH.",
                b".HDHHHD.",
                b"..DDDD..",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"..YW", b".HRO", b"HD.D", b".DD."], PAL),
        accent: (255, 168, 72),
    }
};

/// `PassiveKind::SecondScent` — twin rising aqua scent wisps above stacked golden crumbs.
pub const SECOND_SCENT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (234, 255, 252)), // bright scent mist tip
        (b'S', (92, 236, 224)),  // vibrant aqua scent ribbon (LightCyan in 16c)
        (b'T', (38, 152, 160)),  // trailing teal wisp (Cyan in 16c)
        (b'G', (255, 222, 88)),  // upper golden crumb (LightYellow in 16c)
        (b'C', (226, 156, 36)),  // lower second crumb (Yellow in 16c)
        (b'B', (146, 88, 22)),   // crumb underside shade
    ];
    Icon {
        large: grid8(
            [
                b"..W...W.",
                b".TS..TS.",
                b"..ST..ST",
                b".TS..TS.",
                b"..GG....",
                b".GGCB...",
                b"..CCCBB.",
                b"...BBB..",
            ],
            PAL,
        ),
        small: grid4([b"W..W", b".S.S", b".GC.", b"CBB."], PAL),
        accent: (104, 236, 226),
    }
};

/// `PassiveKind::LensCap` — dark-rimmed cap with grip tab over gleaming green bottle-glass lens.
pub const LENS_CAP: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (242, 255, 255)), // crisp glass specular glare
        (b'S', (196, 218, 224)), // cut-glass silver bevel rim
        (b'L', (122, 242, 212)), // sunlit seafoam bottle glass (LightCyan/LightGreen in 16c)
        (b'G', (42, 168, 146)),  // mid bottle-green glass (Cyan/Green in 16c)
        (b'D', (22, 92, 92)),    // thick bottle-base rim shadow
        (b'K', (52, 64, 76)),    // dark rubber cap housing & grip tab
    ];
    Icon {
        large: grid8(
            [
                b"..KKKK..",
                b".KSSSSK.",
                b"KSWWLGKK",
                b"KSWLLGKK",
                b"KSLGGLDK",
                b".KGGWDK.",
                b"..KKKKK.",
                b"......K.",
            ],
            PAL,
        ),
        small: grid4([b".KK.", b"KWLD", b"KLGK", b".KKK"], PAL),
        accent: (122, 232, 214),
    }
};

/// Currency icon: **Gold Coin** — minted gleaming gold disc with bronze rim.
pub const GOLD_COIN: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 186)), // specular glint
        (b'Y', (255, 210, 48)),  // sunlit gold
        (b'G', (226, 158, 22)),  // rich gold body
        (b'B', (154, 92, 14)),   // bronze shadow rim
    ];
    Icon {
        large: grid8(
            [
                b"..WYYG..",
                b".WWYGGG.",
                b"WYGWYGGG",
                b"YYGWYGBB",
                b"YGGWYGBB",
                b"GGGWYGBB",
                b".GGGBBB.",
                b"..BBBB..",
            ],
            PAL,
        ),
        small: grid4([b".WY.", b"WYGB", b"YGBB", b".BB."], PAL),
        accent: (255, 214, 72),
    }
};

/// Reward icon: **Chest** — banded mahogany treasure chest with glowing gold clasp.
pub const CHEST: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 200)), // glowing brass clasp gleam
        (b'Y', (255, 214, 62)),  // bright gold trim band
        (b'G', (198, 134, 22)),  // aged brass
        (b'M', (178, 92, 38)),   // lit mahogany wood
        (b'D', (116, 52, 22)),   // dark walnut wood
    ];
    Icon {
        large: grid8(
            [
                b".YMMMMY.",
                b"YMMYYMMY",
                b"YMMYYMMY",
                b"YYYYYYYY",
                b"GDDWWDDG",
                b"GDDGGDDG",
                b"GDDDDDDG",
                b".GGGGGG.",
            ],
            PAL,
        ),
        small: grid4([b"YMMY", b"YWWY", b"GDDG", b"GGGG"], PAL),
        accent: (248, 202, 74),
    }
};

/// Overlay treatment: **Evolved Frame** — gilded royal corner filigree and
/// top-right star glint with a transparent center so any weapon icon can wear it.
pub const EVOLVED_FRAME: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 254, 226)), // radiant crown sparkle
        (b'Y', (255, 216, 62)),  // gilded gold
        (b'G', (214, 142, 20)),  // royal amber
    ];
    Icon {
        large: grid8(
            [
                b"WY....YW",
                b"Y......W",
                b"........",
                b"........",
                b"........",
                b"........",
                b"G......G",
                b"YG....GY",
            ],
            PAL,
        ),
        small: grid4([b"W..W", b"....", b"....", b"Y..G"], PAL),
        accent: (255, 224, 88),
    }
};

/// Fallback icon for any future enum variant whose dedicated art has not landed yet.
pub const PLACEHOLDER: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (236, 234, 232)), // crisp question mark
        (b'L', (134, 150, 158)), // lit slate bevel
        (b'S', (72, 86, 94)),    // slate tile body
        (b'D', (42, 52, 58)),    // dark rim
    ];
    Icon {
        large: grid8(
            [
                b".LLLLLL.",
                b"LSSWWSSD",
                b"LSWSSWSD",
                b"LSSSSWSD",
                b"LSSWWSSD",
                b"LSSSSSSD",
                b"LSSWWSSD",
                b".DDDDDD.",
            ],
            PAL,
        ),
        small: grid4([b"LLLS", b"LWW.", b"S.SD", b"SDDD"], PAL),
        accent: (156, 167, 169),
    }
};

// ============================================================================
// Evolutions (13)
// ============================================================================
//
// One silhouette per evolution, in the base weapon's colour family. An evolved
// weapon used to wear its base icon inside a frame; at HUD size that frame was
// four corner pixels, so the rack could not say which weapons had evolved.

/// Milk Teeth (Gnaw) — two baby teeth in an arched gum and a third dangling on its thread.
const MILK_TEETH: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'P', (242, 108, 136)), // gum
        (b'W', (255, 254, 240)), // enamel glint
        (b'I', (236, 222, 180)), // ivory
        (b'S', (184, 164, 124)), // tooth shade
        (b'T', (104, 224, 246)), // the taut thread
    ];
    Icon {
        large: grid8(
            [
                b".PPPPPP.",
                b"PWIPPWIP",
                b".WI..WI.",
                b".IS..IS.",
                b".......T",
                b"..WI..T.",
                b"..WIST..",
                b"...IS...",
            ],
            PAL,
        ),
        small: grid4([b".PP.", b"W.W.", b"I.IT", b".WT."], PAL),
        accent: (244, 232, 200),
    }
};

/// Fondue Fork (Cheese Spray) — twin gleaming steel tines piercing a melting cheese cube.
const FONDUE_FORK: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (252, 254, 255)), // steel tine glint
        (b'S', (206, 216, 230)), // steel
        (b'K', (118, 130, 150)), // steel shade
        (b'H', (255, 246, 174)), // hot cheese highlight
        (b'Y', (255, 206, 58)),  // melted cheese
        (b'O', (196, 124, 24)),  // cheese rind
        (b'M', (134, 68, 36)),   // wooden handle
    ];
    Icon {
        large: grid8(
            [
                b"..W..W..",
                b"..S..S..",
                b".HSYYSO.",
                b".HYYYSY.",
                b"..YYYYO.",
                b"..YSSO..",
                b"...SK...",
                b"...MM...",
            ],
            PAL,
        ),
        small: grid4([b".S.S", b"HYYO", b".SS.", b"..M."], PAL),
        accent: (255, 206, 58),
    }
};

/// Echo Chamber (Squeak Nova) — a pulsing squeak resonating between two thick chamber walls.
const ECHO_CHAMBER: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'L', (228, 196, 255)), // lit wall
        (b'V', (176, 98, 248)),  // wall
        (b'D', (96, 42, 168)),   // shaded wall
        (b'W', (255, 255, 255)), // the bouncing squeak
    ];
    Icon {
        large: grid8(
            [
                b".L....D.",
                b"LL....DD",
                b"LV.WW.VD",
                b"LVW..WVD",
                b"LVW..WVD",
                b"LV.WW.VD",
                b"VD....DD",
                b".D....D.",
            ],
            PAL,
        ),
        small: grid4([b"L..V", b"LWWD", b"LWWD", b"V..D"], PAL),
        accent: (196, 140, 255),
    }
};

/// Fairy Ring (Spore Orbit) — four glowing mint mushrooms round an empty circle.
const FAIRY_RING: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (232, 255, 244)), // cap glint
        (b'M', (96, 248, 164)),  // cap
        (b'G', (36, 154, 96)),   // cap shade
        (b'S', (248, 240, 214)), // warm ivory stem
    ];
    Icon {
        large: grid8(
            [
                b"...WM...",
                b"..WMMG..",
                b"...SS...",
                b"WM....WM",
                b"MMG..MMG",
                b".S....S.",
                b"...WM...",
                b"..WMMG..",
            ],
            PAL,
        ),
        small: grid4([b".WM.", b"M.SM", b"S..S", b".MG."], PAL),
        accent: (108, 248, 172),
    }
};

/// Ratchet Tail (Tail Whip) — a toothed steel ratchet wheel with a hot-pink tail snapping off it.
const RATCHET_TAIL: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (246, 250, 255)), // tooth glint
        (b'S', (174, 188, 208)), // steel
        (b'D', (52, 60, 80)),    // hub
        (b'P', (255, 108, 178)), // hot-pink tail
        (b'R', (196, 46, 118)),  // tail shade
    ];
    Icon {
        large: grid8(
            [
                b"..W.S...",
                b".WSSSW..",
                b"WSDDDSW.",
                b".SD.DS..",
                b"WSDDDSSP",
                b".SSSSS.P",
                b"..S.S.PR",
                b"....PPR.",
            ],
            PAL,
        ),
        small: grid4([b".WS.", b"WDSP", b".SSP", b"..PR"], PAL),
        accent: (200, 208, 222),
    }
};

/// Drain Cleaner (Acid Spit) — a capped caustic bottle wearing a yellow hazard diamond.
const DRAIN_CLEANER: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'K', (64, 104, 218)),  // cap
        (b'W', (236, 255, 214)), // glass glint
        (b'A', (196, 255, 92)),  // acid
        (b'G', (88, 198, 46)),   // acid shade
        (b'D', (32, 92, 28)),    // bottle shade
        (b'Y', (255, 228, 76)),  // hazard warning label
    ];
    Icon {
        large: grid8(
            [
                b"...KK...",
                b"...KK...",
                b"..WGGD..",
                b".WGGGGD.",
                b".WAGYGD.",
                b".WAYYAD.",
                b".WGGGGD.",
                b"..DDDD..",
            ],
            PAL,
        ),
        small: grid4([b".KK.", b"WGGD", b"WAYD", b".DD."], PAL),
        accent: (150, 236, 70),
    }
};

/// Flywheel (Tail Thrash) — a heavy spoked iron wheel with a blazing hot rim.
const FLYWHEEL: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 248, 216)), // white-hot hub
        (b'O', (255, 174, 70)),  // blazing rim
        (b'F', (208, 92, 26)),   // rim shade
        (b'S', (80, 56, 50)),    // dark iron spokes
    ];
    Icon {
        large: grid8(
            [
                b"..OOOO..",
                b".OS..SO.",
                b"O.S..S.F",
                b"OSSWWSSF",
                b"OSSWWSSF",
                b"O.S..S.F",
                b".OS..SF.",
                b"..FFFF..",
            ],
            PAL,
        ),
        small: grid4([b".OO.", b"OSWF", b"OWSF", b".FF."], PAL),
        accent: (255, 170, 72),
    }
};

/// Roll Call (Sonic Squeak) — a clipboard with three names on it.
const ROLL_CALL: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'K', (84, 96, 112)), // clip
        (b'P', (244, 240, 222)), // paper
        (b'C', (64, 176, 214)), // names
        (b'T', (150, 146, 128)), // paper shade
    ];
    Icon {
        large: grid8(
            [
                b"..KKKK..",
                b".PPKKPP.",
                b".PCCCCP.",
                b".PPPPPP.",
                b".PCCCPP.",
                b".PPPPPP.",
                b".PCCCCT.",
                b".TTTTTT.",
            ],
            PAL,
        ),
        small: grid4([b".KK.", b"PCCP", b"PCPP", b"TTTT"], PAL),
        accent: (120, 204, 232),
    }
};

/// Downspout (Acid Rain) — a steel pipe elbow letting go of bright lime acid drops.
const DOWNSPOUT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'L', (206, 216, 226)), // lit pipe
        (b'S', (120, 134, 150)), // pipe
        (b'D', (58, 68, 84)),    // pipe shade
        (b'A', (170, 255, 72)),  // acid
    ];
    Icon {
        large: grid8(
            [
                b"LLLLLL..",
                b"SSSSSSL.",
                b"DDDDSSL.",
                b"...DSSL.",
                b"...DSSL.",
                b"...DDDD.",
                b"....AA..",
                b"...A..A.",
            ],
            PAL,
        ),
        small: grid4([b".LL.", b"LDSL", b"..SL", b".AA."], PAL),
        accent: (170, 255, 72),
    }
};

/// Dead Letter (Hantavirus) — a wide sealed envelope; the wax seal is the wrong colour.
const DEAD_LETTER: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (246, 240, 226)), // paper
        (b'D', (176, 164, 142)), // fold
        (b'S', (120, 108, 92)),  // paper shade
        (b'P', (232, 78, 204)),  // the seal
        (b'Y', (255, 226, 96)),  // seal glint
    ];
    Icon {
        large: grid8(
            [
                b"........",
                b"WWWWWWWW",
                b"WDWWWWDW",
                b"WWDWWDWW",
                b"WWWYPWWS",
                b"WWWPPWWS",
                b"SSSSSSSS",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"....", b"WDDW", b"WYPW", b"SSSS"], PAL),
        accent: (232, 78, 204),
    }
};

/// Carbon Copy (Laser Eyes) — a carbon sheet and the copy under it, one red line on both.
const CARBON_COPY: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'K', (44, 50, 72)), // carbon
        (b'B', (86, 96, 128)), // carbon sheen
        (b'W', (250, 250, 244)), // copy
        (b'L', (170, 176, 186)), // ruled lines
        (b'R', (255, 70, 60)), // the beam
    ];
    Icon {
        large: grid8(
            [
                b".BBBBB..",
                b".BKKKK..",
                b".BRRRK..",
                b".BKWWWWW",
                b".KKWLLLW",
                b"...WRRRW",
                b"...WLLLW",
                b"...WWWWW",
            ],
            PAL,
        ),
        small: grid4([b"BKK.", b"BRWW", b".KRW", b".WWW"], PAL),
        accent: (255, 110, 96),
    }
};

/// Ripcord (Super Dash) — a red pull ring on a white cord.
const RIPCORD: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'C', (236, 244, 255)), // cord
        (b'W', (255, 190, 180)), // ring glint
        (b'R', (238, 62, 62)), // ring
        (b'D', (150, 28, 40)), // ring shade
    ];
    Icon {
        large: grid8(
            [
                b"......CC",
                b".....CC.",
                b"....CC..",
                b"..WRCR..",
                b".WR..RD.",
                b".R....D.",
                b".RR..DD.",
                b"..RDDD..",
            ],
            PAL,
        ),
        small: grid4([b"..CC", b".WC.", b"R..D", b".RD."], PAL),
        accent: (238, 62, 62),
    }
};

/// Barbed Wire (Spiked Tail) — a taut twisted rust-iron strand knotted with two
/// razor-white steel X-barbs.
const BARBED_WIRE: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 255, 255)), // razor barb tip
        (b'S', (198, 214, 232)), // steel barb wrap
        (b'R', (216, 78, 52)),   // lit rust-red wire strand
        (b'D', (82, 34, 28)),    // dark iron-rust wire shadow
    ];
    Icon {
        large: grid8(
            [
                b".W.W....",
                b"..S..W.W",
                b"RRSR..S.",
                b"DDSDRRSR",
                b"..S.DDSD",
                b".W.W..S.",
                b".....W.W",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b".W..", b"RSSW", b"WDDR", b"..W."], PAL),
        accent: (216, 78, 52),
    }
};

// ============================================================================
// Gene splices (10)
// ============================================================================
//
// One silhouette per splice. They used to borrow the passive icon of the stat
// they sell, which made Glass Incisors read as Sharp Teeth on the shelf and
// left two pairs (Double Lid / Copper Nerves, Blackboard Belly / Long Fingers)
// pixel-identical.

/// Glass Incisors — two long translucent glass teeth hanging from a pink gum.
const GLASS_INCISORS: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'P', (236, 128, 150)), // gum
        (b'W', (250, 254, 255)), // glass glint
        (b'C', (132, 214, 236)), // glass body
        (b'D', (52, 108, 136)),  // glass shadow
    ];
    Icon {
        large: grid8(
            [
                b".PPPPPP.",
                b"PWCDPWCD",
                b".WCD.WCD",
                b".CWD.CWD",
                b".WCD.WCD",
                b".WC..WC.",
                b"..W...W.",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"PPPP", b"WCWC", b"CDCD", b".W.W"], PAL),
        accent: (132, 214, 236),
    }
};

/// Boiler Heart — a furnace-orange heart with glowing coals behind an iron grate and steam.
const BOILER_HEART: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (236, 240, 244)), // steam
        (b'Y', (255, 228, 120)), // hot furnace coals & glint
        (b'O', (244, 138, 48)),  // boiler body
        (b'R', (172, 70, 28)),   // shaded side
        (b'D', (44, 22, 20)),    // iron grate bar
    ];
    Icon {
        large: grid8(
            [
                b"..W..W..",
                b".OO..OO.",
                b"OYOOOOOR",
                b"ODDYDDOR",
                b"ODYDYDDR",
                b".OODDOR.",
                b"..OORR..",
                b"...RR...",
            ],
            PAL,
        ),
        small: grid4([b".W.W", b"OYOR", b"ODYR", b".RR."], PAL),
        accent: (244, 138, 48),
    }
};

/// Hollow Bones — a double-knobbed bone on the diagonal with a dark hollow down its shaft.
const HOLLOW_BONES: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (250, 246, 232)), // bone highlight
        (b'B', (206, 196, 172)), // bone shade
        (b'D', (52, 44, 58)),    // the hollow
    ];
    Icon {
        large: grid8(
            [
                b".W.W....",
                b"WWWW....",
                b".WWDW...",
                b"..WDDW..",
                b"...WDDW.",
                b"...BWDB.",
                b"....BBBB",
                b"....B.B.",
            ],
            PAL,
        ),
        small: grid4([b"WW..", b"WDB.", b".WDB", b"..BB"], PAL),
        accent: (226, 218, 196),
    }
};

/// Double Lid — an almond eye peering out from beneath two stacked drooping eyelids.
const DOUBLE_LID: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'V', (136, 86, 206)),  // outer lid
        (b'L', (204, 174, 246)), // inner translucent lid
        (b'W', (252, 252, 255)), // sclera
        (b'A', (255, 204, 76)),  // amber iris
        (b'K', (22, 16, 36)),    // pupil
    ];
    Icon {
        large: grid8(
            [
                b"..VVVV..",
                b".VVVVVV.",
                b"VLLLLLLV",
                b"LWAKKAWL",
                b"WWAKKAWW",
                b".WWWWWW.",
                b"..VVVV..",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"VVVV", b"LLLL", b"WKKW", b".WW."], PAL),
        accent: (176, 136, 232),
    }
};

/// Hoard Gland — a bulging pink cheek pouch hanging from its duct with a gold coin inside.
const HOARD_GLAND: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 232, 236)), // pouch sheen
        (b'P', (240, 150, 170)), // pouch
        (b'M', (168, 82, 110)),  // pouch shade
        (b'Y', (255, 240, 150)), // coin glint
        (b'G', (236, 178, 44)),  // coin
        (b'D', (132, 82, 16)),   // coin rim shade
    ];
    Icon {
        large: grid8(
            [
                b".PP.....",
                b"..PPM...",
                b".PWWPPM.",
                b"PWPYYGPM",
                b"PPGYGGPM",
                b"PPPDDGMM",
                b".PPPMMM.",
                b"..MMMM..",
            ],
            PAL,
        ),
        small: grid4([b"P...", b"PYGM", b"PGDM", b".MM."], PAL),
        accent: (240, 150, 170),
    }
};

/// Copper Nerves — a live copper neuron: dendrites, glowing soma, and a firing axon spark.
const COPPER_NERVES: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 252, 224)), // white-hot synapse spark
        (b'Y', (255, 214, 96)),  // live soma node
        (b'C', (226, 132, 72)),  // copper wire
        (b'B', (148, 76, 40)),   // tarnished copper
    ];
    Icon {
        large: grid8(
            [
                b"C...C...",
                b".C.C....",
                b"..CYC...",
                b"CCYWCCBW",
                b"..CYC...",
                b".C...B..",
                b"C.....B.",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b"C.C.", b".YCW", b"CYCB", b"C..B"], PAL),
        accent: (226, 132, 72),
    }
};

/// Night Shift — a crescent moon and one star.
const NIGHT_SHIFT: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 255, 240)), // star and moon rim
        (b'Y', (246, 226, 132)), // moon
        (b'O', (176, 150, 70)),  // moon shade
    ];
    Icon {
        large: grid8(
            [
                b"..WYY...",
                b".WYY....",
                b"WYY...W.",
                b"WYY..WWW",
                b"YYY...W.",
                b"YYYO....",
                b".YYYOOO.",
                b"..OOOO..",
            ],
            PAL,
        ),
        small: grid4([b".WY.", b"WY.W", b"YY..", b".YOO"], PAL),
        accent: (246, 226, 132),
    }
};

/// Blackboard Belly — a slate in a wooden frame with two chalked lines.
const BLACKBOARD_BELLY: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'B', (170, 118, 66)),  // wooden frame
        (b'D', (104, 68, 36)),   // frame shade
        (b'S', (36, 62, 54)),    // slate
        (b'W', (240, 244, 236)), // chalk
    ];
    Icon {
        large: grid8(
            [
                b"BBBBBBBB",
                b"BSSSSSSD",
                b"BSWWWSSD",
                b"BSSSSSSD",
                b"BSWWWWSD",
                b"BSSSSSSD",
                b"BDDDDDDD",
                b".D....D.",
            ],
            PAL,
        ),
        small: grid4([b"BBBB", b"BWWD", b"BSSD", b"DDDD"], PAL),
        accent: (124, 176, 150),
    }
};

/// Ballast Sac — a tied sandbag, narrow at the neck and heavy at the base.
const BALLAST_SAC: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'L', (232, 212, 164)), // lit burlap
        (b'T', (192, 164, 112)), // burlap
        (b'D', (112, 90, 56)),   // shaded burlap
        (b'K', (70, 44, 30)),    // the tie
    ];
    Icon {
        large: grid8(
            [
                b"..L.T...",
                b"..LTT...",
                b"..KKK...",
                b".LTTTT..",
                b"LLTTTTD.",
                b"LTTTTTDD",
                b"TTTTTDDD",
                b".DDDDDD.",
            ],
            PAL,
        ),
        small: grid4([b".LT.", b".KK.", b"LTTD", b"TDDD"], PAL),
        accent: (192, 164, 112),
    }
};

/// Long Fingers — a splayed rat paw with four staggered spindly fingers reaching up.
const LONG_FINGERS: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 244, 240)), // pale claw tip
        (b'P', (244, 176, 172)), // skin
        (b'M', (176, 104, 110)), // shaded palm
    ];
    Icon {
        large: grid8(
            [
                b"...W....",
                b".W.P.W..",
                b".P.P.P..",
                b".P.P.P.W",
                b".P.P.P.P",
                b".PPPPPP.",
                b"..PPPPM.",
                b"...MMM..",
            ],
            PAL,
        ),
        small: grid4([b".W..", b"WPW.", b"PPPW", b".MM."], PAL),
        accent: (244, 176, 172),
    }
};

const fn wave5_icon(large: [&[u8; 8]; 8], small: [&[u8; 4]; 4], accent: Rgb) -> Icon {
    let palette = [(b'X', accent)];
    Icon {
        large: grid8(large, &palette),
        small: grid4(small, &palette),
        accent,
    }
}

const GLASS_BEAD: Icon = wave5_icon(
    [
        b"........",
        b".....XX.",
        b"....XXXX",
        b"....XXXX",
        b".....XX.",
        b"...X....",
        b".X......",
        b"........",
    ],
    [b"..X.", b"..XX", b".X..", b"X..."],
    (138, 244, 255),
);

const LAST_BOUNCE: Icon = wave5_icon(
    [
        b"...X....",
        b"...X..X.",
        b"X..X.X..",
        b".XXXXX..",
        b"..XXXXXX",
        b".XXXXX..",
        b"X..X.X..",
        b"...X..X.",
    ],
    [b".X.X", b"XXXX", b".XX.", b"X..X"],
    (173, 234, 255),
);

const PLUMB_BOB: Icon = wave5_icon(
    [
        b"...X....",
        b"...X....",
        b"...X....",
        b"...X....",
        b"..XXX...",
        b".XXXXX..",
        b"..XXX...",
        b"...X....",
    ],
    [b".X..", b".X..", b"XXX.", b".X.."],
    (255, 212, 110),
);

const SHED_BOB: Icon = wave5_icon(
    [
        b"........",
        b"X.......",
        b".X......",
        b"..XXXX..",
        b".....XX.",
        b"....XXXX",
        b".....XX.",
        b"......X.",
    ],
    [b"X...", b".XX.", b"..XX", b"...X"],
    (255, 228, 137),
);

// ============================================================================
// Lookup API
// ============================================================================

/// Look up the icon for a weapon.
///
/// Every weapon is matched explicitly (no wildcard): a new `WeaponKind`
/// must pick its icon here to compile.
pub fn weapon(kind: WeaponKind) -> &'static Icon {
    match kind {
        WeaponKind::Gnaw => &GNAW,
        WeaponKind::CheeseSpray => &CHEESE_SPRAY,
        WeaponKind::SqueakNova => &SQUEAK_NOVA,
        WeaponKind::SporeOrbit => &SPORE_ORBIT,
        WeaponKind::TailWhip => &TAIL_WHIP,
        WeaponKind::AcidSpit => &ACID_SPIT,
        WeaponKind::TailThrash => &TAIL_THRASH,
        WeaponKind::SonicSqueak => &SONIC_SQUEAK,
        WeaponKind::LaserEyes => &LASER_EYES,
        WeaponKind::SuperDash => &SUPER_DASH,
        WeaponKind::AcidRain => &ACID_RAIN,
        WeaponKind::Hantavirus => &HANTAVIRUS,
        WeaponKind::SpikedTail => &SPIKED_TAIL,
        WeaponKind::LavaBreath => &LAVA_BREATH,
        WeaponKind::GlassBead => &GLASS_BEAD,
        WeaponKind::PlumbBob => &PLUMB_BOB,
    }
}

/// Look up the icon for a weapon's **evolution** (`WeaponKind::evolution`), or
/// `None` for a weapon that has none (none today). No wildcard: a new weapon
/// must say here whether it has an evolved icon.
pub fn evolution(kind: WeaponKind) -> Option<&'static Icon> {
    Some(match kind {
        WeaponKind::Gnaw => &MILK_TEETH,
        WeaponKind::CheeseSpray => &FONDUE_FORK,
        WeaponKind::SqueakNova => &ECHO_CHAMBER,
        WeaponKind::SporeOrbit => &FAIRY_RING,
        WeaponKind::TailWhip => &RATCHET_TAIL,
        WeaponKind::AcidSpit => &DRAIN_CLEANER,
        WeaponKind::TailThrash => &FLYWHEEL,
        WeaponKind::SonicSqueak => &ROLL_CALL,
        WeaponKind::AcidRain => &DOWNSPOUT,
        WeaponKind::Hantavirus => &DEAD_LETTER,
        WeaponKind::LaserEyes => &CARBON_COPY,
        WeaponKind::SuperDash => &RIPCORD,
        WeaponKind::SpikedTail => &BARBED_WIRE,
        WeaponKind::LavaBreath => &CALDERA,
        WeaponKind::GlassBead => &LAST_BOUNCE,
        WeaponKind::PlumbBob => &SHED_BOB,
    })
}

/// The icon [`draw_evolved`] actually paints for `icon`: a base weapon icon
/// becomes its evolution's own icon, anything else is drawn as given.
///
/// Every caller hands [`draw_evolved`] `weapon(kind)`, so the swap lives here
/// and the evolved art reaches the HUD rack, the cards and the keep plate
/// without those callers changing.
fn evolved_form(icon: &Icon) -> &Icon {
    for kind in WeaponKind::all() {
        if icon == weapon(kind) {
            return evolution(kind).unwrap_or(icon);
        }
    }
    icon
}

/// Look up the icon for a passive.
///
/// Every passive is matched explicitly (no wildcard), like [`weapon`]: a new
/// `PassiveKind` must pick its icon here to compile.
pub fn passive(kind: PassiveKind) -> &'static Icon {
    match kind {
        PassiveKind::MaxHp => &MAX_HP,
        PassiveKind::MoveSpeed => &MOVE_SPEED,
        PassiveKind::Damage => &DAMAGE,
        PassiveKind::FireRate => &FIRE_RATE,
        PassiveKind::Magnet => &MAGNET,
        PassiveKind::Regen => &REGEN,
        PassiveKind::Armor => &ARMOR,
        // Spring Heel is the dash-cooldown passive the icon pass pre-drew.
        PassiveKind::SpringHeel => &DASH_COOLDOWN,
        PassiveKind::Counterweight => &COUNTERWEIGHT,
        PassiveKind::LongFuse => &LONG_FUSE,
        PassiveKind::SecondScent => &SECOND_SCENT,
        PassiveKind::LensCap => &LENS_CAP,
    }
}

/// Look up the icon for a gene splice (`game::shop::SpliceKind`).
///
/// Each of the ten splices has its own authored silhouette (the body part its
/// name says), distinct from every weapon and passive icon, so a splice never
/// reads as the passive that sells the same stat.
pub fn splice(kind: crate::game::shop::SpliceKind) -> &'static Icon {
    use crate::game::shop::SpliceKind;
    match kind {
        SpliceKind::GlassIncisors => &GLASS_INCISORS,
        SpliceKind::BoilerHeart => &BOILER_HEART,
        SpliceKind::HollowBones => &HOLLOW_BONES,
        SpliceKind::DoubleLid => &DOUBLE_LID,
        SpliceKind::HoardGland => &HOARD_GLAND,
        SpliceKind::CopperNerves => &COPPER_NERVES,
        SpliceKind::NightShift => &NIGHT_SHIFT,
        SpliceKind::BlackboardBelly => &BLACKBOARD_BELLY,
        SpliceKind::BallastSac => &BALLAST_SAC,
        SpliceKind::LongFingers => &LONG_FINGERS,
    }
}

/// The neutral fallback icon for not-yet-authored items.
#[inline]
pub const fn placeholder() -> &'static Icon {
    &PLACEHOLDER
}

/// Look up an icon by its canonical display name (used by display-only views
/// such as `render/campaign.rs::UpgradeEntry` that carry strings rather than
/// simulation enums). Returns [`placeholder`] if unrecognized.
pub fn by_name(name: &str) -> &'static Icon {
    match name {
        "Gnaw" => &GNAW,
        "Cheese Spray" => &CHEESE_SPRAY,
        "Squeak Nova" => &SQUEAK_NOVA,
        "Spore Orbit" => &SPORE_ORBIT,
        "Tail Whip" => &TAIL_WHIP,
        "Acid Spit" => &ACID_SPIT,
        "Tail Thrash" => &TAIL_THRASH,
        "Sonic Squeak" => &SONIC_SQUEAK,
        "Acid Rain" => &ACID_RAIN,
        "Hantavirus" => &HANTAVIRUS,
        "Laser Eyes" => &LASER_EYES,
        "Super Dash" => &SUPER_DASH,
        "Spiked Tail" => &SPIKED_TAIL,
        "Fat Reserves" => &MAX_HP,
        "Quick Paws" => &MOVE_SPEED,
        "Sharp Teeth" => &DAMAGE,
        "Frenzy" => &FIRE_RATE,
        "Whiskers" => &MAGNET,
        "Resilience" => &REGEN,
        "Thick Hide" => &ARMOR,
        "Dash Cooldown" | "Spring Heel" => &DASH_COOLDOWN,
        "Counterweight" => &COUNTERWEIGHT,
        "Long Fuse" => &LONG_FUSE,
        "Second Scent" => &SECOND_SCENT,
        "Lens Cap" => &LENS_CAP,
        "Lava Breath" => &LAVA_BREATH,
        "Caldera" => &CALDERA,
        "Glass Bead" => &GLASS_BEAD,
        "Last Bounce" => &LAST_BOUNCE,
        "Plumb Bob" => &PLUMB_BOB,
        "Shed Bob" => &SHED_BOB,
        _ => &PLACEHOLDER,
    }
}

const LAVA_PAL: &[(u8, Rgb)] = &[
    (b'W', (255, 238, 154)),
    (b'O', (255, 140, 40)),
    (b'R', (186, 52, 24)),
];

const LAVA_BREATH: Icon = Icon {
    large: grid8(
        [
            b"....WO..",
            b"....OR..",
            b"........",
            b".WO...O.",
            b".OR..WOR",
            b".....OR.",
            b"........",
            b"........",
        ],
        LAVA_PAL,
    ),
    small: grid4([b"..W.", b".O..", b"W..O", b"R..R"], LAVA_PAL),
    accent: (255, 140, 40),
};

const CALDERA: Icon = Icon {
    large: grid8(
        [
            b"........",
            b"...O....",
            b"..W..O..",
            b"........",
            b"..R..R..",
            b".ROWORR.",
            b"..ROOR..",
            b"...RR...",
        ],
        LAVA_PAL,
    ),
    small: grid4([b".W..", b"..O.", b"ROWR", b".RR."], LAVA_PAL),
    accent: (255, 175, 82),
};

// ============================================================================
// The nest (permanent upgrade tracks)
// ============================================================================

/// `Track::WeaponSlots` / `Track::PassiveSlots` — three rack slots on a rail
/// with one lit gold bay, i.e. "room for one more".
const SLOT_RACK: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (238, 244, 246)), // lit slot edge
        (b'S', (128, 152, 160)), // cold steel bay
        (b'L', (86, 104, 112)),  // shaded rail
        (b'G', (248, 206, 96)),  // the open bay's gold glint
        (b'D', (30, 40, 46)),    // dark rim
    ];
    Icon {
        large: grid8(
            [
                b"........",
                b"DSWSDSSD",
                b"DSWSDSSD",
                b"DSWSDGSD",
                b"DSWSDGSD",
                b"DSWSDSSD",
                b"LSSSSSSL",
                b".DDDDDD.",
            ],
            PAL,
        ),
        small: grid4([b"SWGD", b"SWGD", b"SWGD", b"LDDD"], PAL),
        accent: (176, 198, 204),
    }
};

/// `Track::Revive` — a heart with a white cross: get up once more.
const REVIVE: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'W', (255, 248, 250)), // the cross
        (b'H', (246, 76, 98)),   // heart body
        (b'R', (176, 32, 54)),   // shaded heart
        (b'D', (104, 16, 30)),   // deep maroon
    ];
    Icon {
        large: grid8(
            [
                b"........",
                b".HHWWHH.",
                b"HHHWWHHR",
                b"HHWWWWHR",
                b".HHWWHHR",
                b"..HHHRR.",
                b"...RD...",
                b"........",
            ],
            PAL,
        ),
        small: grid4([b".W..", b"HWHH", b".HH.", b"..D."], PAL),
        accent: (222, 120, 132),
    }
};

/// `Track::CarryOver` — a satchel with a gold buckle: the pack that carries
/// more powerups to the next floor.
const PACK: Icon = {
    const PAL: &[(u8, Rgb)] = &[
        (b'L', (150, 128, 96)),  // leather strap and rim
        (b'S', (110, 88, 62)),   // satchel body
        (b'G', (248, 206, 96)),  // gold buckle
        (b'D', (54, 42, 30)),    // dark seam
    ];
    Icon {
        large: grid8(
            [
                b"..LLLL..",
                b".L....L.",
                b"LLLLLLLL",
                b"LSSSSSSL",
                b"LSGGGGSL",
                b"LSSSSSSL",
                b"LSSSSSSL",
                b".DDDDDD.",
            ],
            PAL,
        ),
        small: grid4([b".LL.", b"LSSL", b"LGGD", b"LDDD"], PAL),
        accent: (196, 168, 120),
    }
};

/// Look up the icon for a nest track.
///
/// The stat tracks borrow the authored passive icon of the stat they sell (the
/// same icon the level-up card prints, so the nest reads as the same currency);
/// the tracks with no in-run twin get their own authored sprite above.
pub fn meta(track: crate::meta::Track) -> &'static Icon {
    use crate::meta::Track;
    match track {
        Track::WeaponSlots => &SLOT_RACK,
        Track::PassiveSlots => &SLOT_RACK,
        Track::MoveSpeed => &MOVE_SPEED,
        Track::PickupRange => &MAGNET,
        Track::FireRate => &FIRE_RATE,
        Track::MaxHp => &MAX_HP,
        Track::Regen => &REGEN,
        Track::Armor => &ARMOR,
        Track::Damage => &DAMAGE,
        Track::DashCooldown => &DASH_COOLDOWN,
        Track::XpGain => &SECOND_SCENT,
        Track::GoldGain => &GOLD_COIN,
        Track::Luck => &CHEST,
        Track::CarryOver => &PACK,
        Track::Revive => &REVIVE,
    }
}

// ============================================================================
// Compositing Renderer (Zero Allocation)
// ============================================================================

/// Composite `icon` into `buf` inside `area`, using the resolved terminal
/// colour depth.
///
/// Automatically picks the authored **large** (`8 × 8` px = `8 × 4` cells)
/// sprite when `area` is at least `8 × 4` cells, and the authored **small**
/// (`4 × 4` px = `4 × 2` cells) sprite otherwise. Clips cleanly to `area` and
/// `buf.area()`, composites transparent pixels over existing content, emits
/// only safe block glyphs (`▀` / `▄`), and never allocates.
pub fn draw(buf: &mut Buffer, area: Rect, icon: &Icon) {
    draw_with_depth(buf, area, icon, crate::term::color_depth());
}

/// Like [`draw`], with an explicit [`ColorDepth`] for deterministic testing.
pub fn draw_with_depth(buf: &mut Buffer, area: Rect, icon: &Icon, depth: ColorDepth) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.width >= LARGE_CELL_W && area.height >= LARGE_CELL_H {
        blit_grid::<LARGE_PX_W, LARGE_PX_H>(buf, area, &icon.large, depth);
    } else {
        blit_grid::<SMALL_PX_W, SMALL_PX_H>(buf, area, &icon.small, depth);
    }
}

/// Explicitly draw the authored **large** (`8 × 8` px) form clipped to `area`.
pub fn draw_large(buf: &mut Buffer, area: Rect, icon: &Icon) {
    draw_large_with_depth(buf, area, icon, crate::term::color_depth());
}

/// Like [`draw_large`], with an explicit [`ColorDepth`].
pub fn draw_large_with_depth(buf: &mut Buffer, area: Rect, icon: &Icon, depth: ColorDepth) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    blit_grid::<LARGE_PX_W, LARGE_PX_H>(buf, area, &icon.large, depth);
}

/// Explicitly draw the authored **small** (`4 × 4` px) form clipped to `area`.
pub fn draw_small(buf: &mut Buffer, area: Rect, icon: &Icon) {
    draw_small_with_depth(buf, area, icon, crate::term::color_depth());
}

/// Like [`draw_small`], with an explicit [`ColorDepth`].
pub fn draw_small_with_depth(buf: &mut Buffer, area: Rect, icon: &Icon, depth: ColorDepth) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    blit_grid::<SMALL_PX_W, SMALL_PX_H>(buf, area, &icon.small, depth);
}

/// Composite `icon` in its **evolved** form.
///
/// A base weapon icon is swapped for its evolution's own icon
/// ([`evolved_form`]), and the result sits on a gilded plate so "evolved"
/// reads at every size without covering a single icon pixel:
///
/// - at `10 × 5` cells or more, a complete one-pixel gold border (lit top-left,
///   shaded bottom-right) runs round the centred `8 × 8` icon;
/// - at `8 × 4` (large) and below (small), every transparent icon pixel is
///   filled with a dark-gold plate and the free corners are bright gold, so the
///   whole footprint — not four corner pixels — says "evolved".
pub fn draw_evolved(buf: &mut Buffer, area: Rect, icon: &Icon) {
    draw_evolved_with_depth(buf, area, icon, crate::term::color_depth());
}

/// Gilded-plate colours: the glint, the lit and shaded gold of the border, and
/// the two dark-gold tones of the backing plate (lit upper half, shaded lower).
const GILT_GLINT: Rgb = (255, 254, 226);
const GILT_LIT: Rgb = (255, 216, 62);
const GILT_SHADE: Rgb = (214, 142, 20);
const PLATE_LIT: Rgb = (124, 86, 14);
const PLATE_SHADE: Rgb = (86, 56, 8);

/// `grid` on the evolved backing plate: transparent pixels become dark gold,
/// transparent corners bright gold. Icon pixels are never replaced.
fn plated<const W: usize, const H: usize>(
    grid: &[[Option<Rgb>; W]; H],
) -> [[Option<Rgb>; W]; H] {
    let mut out = *grid;
    for (y, row) in out.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            if px.is_some() {
                continue;
            }
            let (left, top) = (x == 0, y == 0);
            let (right, bottom) = (x + 1 == W, y + 1 == H);
            *px = Some(if left && top {
                GILT_GLINT
            } else if (right && top) || (left && bottom) {
                GILT_LIT
            } else if right && bottom {
                GILT_SHADE
            } else if y * 2 < H {
                PLATE_LIT
            } else {
                PLATE_SHADE
            });
        }
    }
    out
}

/// Like [`draw_evolved`], with an explicit [`ColorDepth`].
pub fn draw_evolved_with_depth(buf: &mut Buffer, area: Rect, icon: &Icon, depth: ColorDepth) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let icon = evolved_form(icon);
    if area.width >= EVOLVED_FRAME_CELL_W && area.height >= EVOLVED_FRAME_CELL_H {
        // The 8×8 icon centred in a 10×10 box (one column, one half-row in),
        // inside a complete gold border.
        let mut combined = [[None; 10]; 10];
        #[allow(clippy::needless_range_loop)]
        for y in 0..10 {
            for x in 0..10 {
                if x == 0 || y == 0 || x == 9 || y == 9 {
                    let corner = (x == 0 || x == 9) && (y == 0 || y == 9);
                    combined[y][x] = Some(if corner && x == 0 && y == 0 {
                        GILT_GLINT
                    } else if x == 9 || y == 9 {
                        GILT_SHADE
                    } else {
                        GILT_LIT
                    });
                }
            }
        }
        for y in 0..LARGE_PX_H {
            for x in 0..LARGE_PX_W {
                combined[y + 1][x + 1] = icon.large[y][x];
            }
        }
        blit_grid::<10, 10>(buf, area, &combined, depth);
    } else if area.width >= LARGE_CELL_W && area.height >= LARGE_CELL_H {
        blit_grid::<LARGE_PX_W, LARGE_PX_H>(buf, area, &plated(&icon.large), depth);
    } else {
        blit_grid::<SMALL_PX_W, SMALL_PX_H>(buf, area, &plated(&icon.small), depth);
    }
}

/// Core half-block compositor matching `render/cursor.rs` semantics:
/// - Respects `Modifier::REVERSED` on existing UI cells before stripping modifiers.
/// - Preserves underlying half-block pixels (`▀` / `▄`) and UI backgrounds when
///   one half of a cell is transparent.
/// - Uses `▄` over text/UI cells when only the bottom half is opaque so
///   `Color::Reset` backgrounds remain intact.
/// - Clips strictly to `area ∩ buf.area()`.
fn blit_grid<const W: usize, const H: usize>(
    buf: &mut Buffer,
    area: Rect,
    grid: &[[Option<Rgb>; W]; H],
    depth: ColorDepth,
) {
    let clip = area.intersection(*buf.area());
    if clip.is_empty() {
        return;
    }
    let cell_h = (H / 2) as u16;
    let cell_w = W as u16;

    for dy in 0..cell_h {
        let Some(cy) = area.y.checked_add(dy) else {
            break;
        };
        if cy < clip.y || cy >= clip.bottom() {
            continue;
        }
        let top_row = (dy as usize) * 2;
        let bot_row = top_row + 1;

        for dx in 0..cell_w {
            let Some(cx) = area.x.checked_add(dx) else {
                break;
            };
            if cx < clip.x || cx >= clip.right() {
                continue;
            }
            let top = grid[top_row][dx as usize];
            let bot = grid[bot_row][dx as usize];
            if top.is_none() && bot.is_none() {
                continue;
            }
            let Some(cell) = buf.cell_mut((cx, cy)) else {
                continue;
            };

            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (raw_fg, raw_bg) = if reversed {
                (cell.bg, cell.fg)
            } else {
                (cell.fg, cell.bg)
            };
            let is_upper_half = cell.symbol() == "▀";
            let is_lower_half = cell.symbol() == "▄";
            let (old_top, old_bot) = if is_upper_half {
                (raw_fg, raw_bg)
            } else if is_lower_half {
                (raw_bg, raw_fg)
            } else {
                (raw_bg, raw_bg)
            };

            cell.modifier = Modifier::empty();

            match (top, bot) {
                (Some(t), Some(b)) => {
                    let tf = palette::to_term(t, depth);
                    let bf = palette::to_term(b, depth);
                    cell.set_symbol("▀");
                    cell.set_fg(tf);
                    cell.set_bg(bf);
                }
                (Some(t), None) => {
                    let tf = palette::to_term(t, depth);
                    cell.set_symbol("▀");
                    cell.set_fg(tf);
                    cell.set_bg(old_bot);
                }
                (None, Some(b)) => {
                    let bf = palette::to_term(b, depth);
                    if is_upper_half {
                        cell.set_symbol("▀");
                        cell.set_fg(old_top);
                        cell.set_bg(bf);
                    } else {
                        cell.set_symbol("▄");
                        cell.set_fg(bf);
                        cell.set_bg(old_top);
                    }
                }
                (None, None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{sanitize_with, GlyphSet};
    use ratatui::style::Color;

    fn all_named_icons() -> Vec<(&'static str, &'static Icon)> {
        vec![
            ("Gnaw", &GNAW),
            ("CheeseSpray", &CHEESE_SPRAY),
            ("SqueakNova", &SQUEAK_NOVA),
            ("SporeOrbit", &SPORE_ORBIT),
            ("TailWhip", &TAIL_WHIP),
            ("AcidSpit", &ACID_SPIT),
            ("TailThrash", &TAIL_THRASH),
            ("SonicSqueak", &SONIC_SQUEAK),
            ("MaxHp", &MAX_HP),
            ("MoveSpeed", &MOVE_SPEED),
            ("Damage", &DAMAGE),
            ("FireRate", &FIRE_RATE),
            ("Magnet", &MAGNET),
            ("Regen", &REGEN),
            ("Armor", &ARMOR),
            ("AcidRain", &ACID_RAIN),
            ("Hantavirus", &HANTAVIRUS),
            ("LaserEyes", &LASER_EYES),
            ("SuperDash", &SUPER_DASH),
            ("DashCooldown", &DASH_COOLDOWN),
            ("Counterweight", &COUNTERWEIGHT),
            ("LongFuse", &LONG_FUSE),
            ("SecondScent", &SECOND_SCENT),
            ("LensCap", &LENS_CAP),
            ("GoldCoin", &GOLD_COIN),
            ("Chest", &CHEST),
            ("EvolvedFrame", &EVOLVED_FRAME),
            ("SpikedTail", &SPIKED_TAIL),
            ("Placeholder", &PLACEHOLDER),
            ("MilkTeeth", &MILK_TEETH),
            ("FondueFork", &FONDUE_FORK),
            ("EchoChamber", &ECHO_CHAMBER),
            ("FairyRing", &FAIRY_RING),
            ("RatchetTail", &RATCHET_TAIL),
            ("DrainCleaner", &DRAIN_CLEANER),
            ("Flywheel", &FLYWHEEL),
            ("RollCall", &ROLL_CALL),
            ("Downspout", &DOWNSPOUT),
            ("DeadLetter", &DEAD_LETTER),
            ("CarbonCopy", &CARBON_COPY),
            ("Ripcord", &RIPCORD),
            ("Barbed Wire", &BARBED_WIRE),
            ("GlassIncisors", &GLASS_INCISORS),
            ("BoilerHeart", &BOILER_HEART),
            ("HollowBones", &HOLLOW_BONES),
            ("DoubleLid", &DOUBLE_LID),
            ("HoardGland", &HOARD_GLAND),
            ("CopperNerves", &COPPER_NERVES),
            ("NightShift", &NIGHT_SHIFT),
            ("BlackboardBelly", &BLACKBOARD_BELLY),
            ("BallastSac", &BALLAST_SAC),
            ("LongFingers", &LONG_FINGERS),
            ("Lava Breath", &LAVA_BREATH),
            ("Caldera", &CALDERA),
        ]
    }

    #[test]
    fn every_existing_weapon_and_passive_has_a_non_placeholder_icon() {
        for kind in WeaponKind::all() {
            let ic = weapon(kind);
            assert!(
                !std::ptr::eq(ic, placeholder()) && ic != placeholder(),
                "weapon {kind:?} returned placeholder"
            );
            assert_eq!(
                by_name(kind.name()),
                ic,
                "by_name({:?}) must match weapon({kind:?})",
                kind.name()
            );
        }
        for kind in PassiveKind::all() {
            let ic = passive(kind);
            assert!(
                !std::ptr::eq(ic, placeholder()) && ic != placeholder(),
                "passive {kind:?} returned placeholder"
            );
            assert_eq!(
                by_name(kind.name()),
                ic,
                "by_name({:?}) must match passive({kind:?})",
                kind.name()
            );
        }
        assert_eq!(by_name("UnknownFutureThing"), placeholder());
    }

    /// Every weapon with an evolution row has an evolved icon and no other
    /// weapon does; `draw_evolved` paints that icon, not the base one, and at
    /// HUD size the evolved tile is fully opaque (the gilded plate), so it can
    /// never be mistaken for the base icon drawn by `draw`.
    #[test]
    fn evolutions_have_their_own_icons_and_a_full_plate() {
        for kind in WeaponKind::all() {
            assert_eq!(
                evolution(kind).is_some(),
                kind.evolution().is_some(),
                "{kind:?}: evolved icon and evolution row disagree"
            );
            let Some(evo) = evolution(kind) else {
                assert!(std::ptr::eq(evolved_form(weapon(kind)), weapon(kind)));
                continue;
            };
            assert!(evo != weapon(kind), "{kind:?} evolves into its own icon");
            assert_eq!(evolved_form(weapon(kind)), evo);

            for (w, h) in [(4u16, 2u16), (8, 4), (10, 5)] {
                let area = Rect::new(0, 0, w, h);
                let mut base = Buffer::empty(area);
                let mut evolved = Buffer::empty(area);
                draw_with_depth(&mut base, area, weapon(kind), ColorDepth::Truecolor);
                draw_evolved_with_depth(&mut evolved, area, weapon(kind), ColorDepth::Truecolor);
                assert!(base != evolved, "{kind:?} at {w}x{h}: evolved looks like the base");
                if w < 10 {
                    for y in 0..h {
                        for x in 0..w {
                            assert_eq!(
                                evolved[(x, y)].symbol(),
                                "▀",
                                "{kind:?} at {w}x{h}: hole in the plate at ({x},{y})"
                            );
                        }
                    }
                }
            }
        }
        // An icon that is not a base weapon's is plated as given.
        assert!(std::ptr::eq(evolved_form(&MAGNET), &MAGNET));
    }

    /// The plate fills only what the icon leaves transparent.
    #[test]
    fn the_evolved_plate_never_replaces_an_icon_pixel() {
        for (name, icon) in all_named_icons() {
            let big = plated(&icon.large);
            for y in 0..LARGE_PX_H {
                for x in 0..LARGE_PX_W {
                    assert!(big[y][x].is_some(), "{name}: unplated pixel");
                    if let Some(c) = icon.large[y][x] {
                        assert_eq!(big[y][x], Some(c), "{name}: plate covered ({x},{y})");
                    }
                }
            }
            let small = plated(&icon.small);
            for y in 0..SMALL_PX_H {
                for x in 0..SMALL_PX_W {
                    if let Some(c) = icon.small[y][x] {
                        assert_eq!(small[y][x], Some(c), "{name}: plate covered ({x},{y})");
                    }
                }
            }
        }
    }

    /// A splice must never wear a weapon's or a passive's icon (it used to
    /// borrow the passive of the stat it sells), nor another splice's.
    #[test]
    fn every_splice_has_its_own_icon() {
        use crate::game::shop::SpliceKind;
        let splices = SpliceKind::ALL;
        for (i, &kind) in splices.iter().enumerate() {
            let ic = splice(kind);
            assert!(ic != placeholder(), "splice {kind:?} returned placeholder");
            for w in WeaponKind::all() {
                assert!(ic != weapon(w), "splice {kind:?} shares {w:?}'s icon");
            }
            for p in PassiveKind::all() {
                assert!(ic != passive(p), "splice {kind:?} shares {p:?}'s icon");
            }
            for &other in &splices[i + 1..] {
                let o = splice(other);
                assert!(ic.large != o.large, "{kind:?} and {other:?} share a large icon");
                assert!(ic.small != o.small, "{kind:?} and {other:?} share a small icon");
            }
        }
    }

    #[test]
    fn all_icons_are_pairwise_distinct_at_both_sizes() {
        let icons = all_named_icons();
        for i in 0..icons.len() {
            let (name_a, a) = icons[i];
            // Every icon must have non-empty pixels in both forms.
            assert!(
                a.large.iter().flatten().any(|p| p.is_some()),
                "{name_a} large grid is empty"
            );
            assert!(
                a.small.iter().flatten().any(|p| p.is_some()),
                "{name_a} small grid is empty"
            );
            for (name_b, b) in &icons[i + 1..] {
                assert_ne!(
                    a.large, b.large,
                    "large icons for {name_a} and {name_b} are identical"
                );
                assert_ne!(
                    a.small, b.small,
                    "small icons for {name_a} and {name_b} are identical"
                );
            }
        }
    }

    #[test]
    fn draw_clips_cleanly_at_every_edge_and_degenerate_sizes() {
        let icon = weapon(WeaponKind::CheeseSpray);

        // 0×0 buffer and 0×0 area: no-op, never panics.
        let zero = Rect::new(0, 0, 0, 0);
        let mut zbuf = Buffer::empty(zero);
        draw_with_depth(&mut zbuf, zero, icon, ColorDepth::Truecolor);
        draw_evolved_with_depth(&mut zbuf, zero, icon, ColorDepth::Truecolor);

        // 1×1 buffer: clips cleanly to a single cell without panicking.
        let one = Rect::new(0, 0, 1, 1);
        let mut obuf = Buffer::empty(one);
        draw_with_depth(&mut obuf, one, weapon(WeaponKind::Gnaw), ColorDepth::Truecolor);
        assert_eq!(obuf[(0, 0)].symbol(), "▀");

        // Clipping at all four edges, corners, partial overlaps, and u16::MAX overflow.
        let full = Rect::new(0, 0, 12, 8);
        let cases = [
            Rect::new(0, 0, 8, 4),
            Rect::new(10, 0, 8, 4),  // clips right edge
            Rect::new(0, 6, 8, 4),   // clips bottom edge
            Rect::new(10, 6, 8, 4),  // clips bottom-right corner
            Rect::new(11, 7, 4, 2),  // single bottom-right cell
            Rect::new(12, 8, 8, 4),  // completely out of bounds
            Rect::new(0, 0, 3, 1),   // sub-small width & height
            Rect::new(0, 0, 6, 3),   // between small and large (uses small)
            Rect::new(u16::MAX, u16::MAX, 8, 4),
            Rect::new(u16::MAX - 1, u16::MAX - 1, 10, 5),
        ];
        for r in cases {
            let mut buf = Buffer::empty(full);
            draw_with_depth(&mut buf, r, icon, ColorDepth::Truecolor);
            draw_large_with_depth(&mut buf, r, icon, ColorDepth::Truecolor);
            draw_small_with_depth(&mut buf, r, icon, ColorDepth::Truecolor);
            draw_evolved_with_depth(&mut buf, r, icon, ColorDepth::Truecolor);
        }

        // Strict containment: drawing inside a sub-rect must never touch outside cells.
        let mut buf = Buffer::empty(full);
        let sub = Rect::new(3, 2, 4, 2);
        draw_with_depth(&mut buf, sub, weapon(WeaponKind::AcidSpit), ColorDepth::Truecolor);
        for y in 0..full.height {
            for x in 0..full.width {
                let inside = x >= sub.x && x < sub.right() && y >= sub.y && y < sub.bottom();
                if !inside {
                    assert_eq!(
                        buf[(x, y)].symbol(),
                        " ",
                        "cell ({x},{y}) outside clip rect was modified"
                    );
                }
            }
        }
    }

    #[test]
    fn composites_transparent_pixels_and_strips_reversed_modifier() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);

        // Pre-fill cell (0, 0) with a half-block playfield pixel.
        // For CHEESE_SPRAY small, (0,0) and (0,1) are both None, while (2,0) is None
        // and (2,1) is Some(O) — wait: let's check CHEESE_SPRAY small:
        // row 0: `..TR` -> dx=2 is Some(T), row 1: `.HOR` -> dx=1 is Some(H), dx=0 is None.
        // For (1, 0): top = grid[0][1] = None, bot = grid[1][1] = Some(H).
        let bg_top = Color::Rgb(14, 22, 20);
        let bg_bot = Color::Rgb(40, 70, 64);
        buf[(1, 0)].set_symbol("▀").set_fg(bg_top).set_bg(bg_bot);

        // Pre-fill (2, 0) with REVERSED text cell.
        buf[(2, 0)]
            .set_symbol("X")
            .set_fg(Color::Rgb(11, 15, 17))
            .set_bg(Color::Rgb(235, 232, 232));
        buf[(2, 0)].modifier = Modifier::REVERSED | Modifier::BOLD;

        draw_small_with_depth(&mut buf, Rect::new(0, 0, 4, 2), &CHEESE_SPRAY, ColorDepth::Truecolor);

        // (1,0) had transparent top + opaque bottom over `"▀"`: preserves bg_top!
        assert_eq!(buf[(1, 0)].symbol(), "▀");
        assert_eq!(buf[(1, 0)].fg, bg_top);
        assert_ne!(buf[(1, 0)].bg, bg_bot);

        // (2,0) had REVERSED stripped.
        assert_eq!(buf[(2, 0)].modifier, Modifier::empty());
        assert_eq!(buf[(2, 0)].symbol(), "▀");
    }

    #[test]
    fn color_depth_fallbacks_and_safe_glyphs_stay_readable() {
        let area = Rect::new(0, 0, 8, 4);

        for (name, icon) in all_named_icons() {
            // 256-colour: every painted cell uses Indexed (never raw Rgb).
            let mut b256 = Buffer::empty(area);
            draw_large_with_depth(&mut b256, area, icon, ColorDepth::Ansi256);
            let mut painted_256 = 0usize;
            for y in 0..4 {
                for x in 0..8 {
                    let c = &b256[(x, y)];
                    if c.symbol() == "▀" || c.symbol() == "▄" {
                        painted_256 += 1;
                        assert!(
                            matches!(c.fg, Color::Indexed(_)),
                            "{name} 256-col fg not Indexed at ({x},{y}): {:?}",
                            c.fg
                        );
                    }
                }
            }
            assert!(painted_256 > 0, "{name} drew nothing in 256-colour mode");

            // 16-colour: every painted cell uses a named ANSI16 colour (never Rgb or Indexed).
            let mut b16 = Buffer::empty(area);
            draw_large_with_depth(&mut b16, area, icon, ColorDepth::Ansi16);
            let mut distinct_fg = Vec::new();
            for y in 0..4 {
                for x in 0..8 {
                    let c = &b16[(x, y)];
                    if c.symbol() == "▀" || c.symbol() == "▄" {
                        assert!(
                            !matches!(c.fg, Color::Rgb(_, _, _) | Color::Indexed(_)),
                            "{name} 16-col fg leaked non-ANSI16 at ({x},{y}): {:?}",
                            c.fg
                        );
                        if !distinct_fg.contains(&c.fg) {
                            distinct_fg.push(c.fg);
                        }
                    }
                }
            }
            assert!(
                !distinct_fg.is_empty(),
                "{name} drew nothing in 16-colour mode"
            );

            // Safe glyph set keeps every icon glyph intact (`▀` and `▄` are safe).
            let mut bsafe = Buffer::empty(area);
            draw_large_with_depth(&mut bsafe, area, icon, ColorDepth::Truecolor);
            let snapshot: Vec<String> = (0..4)
                .flat_map(|y| (0..8).map(move |x| (x, y)))
                .map(|(x, y)| bsafe[(x, y)].symbol().to_string())
                .collect();
            sanitize_with(&mut bsafe, GlyphSet::Safe, ColorDepth::Truecolor);
            let after: Vec<String> = (0..4)
                .flat_map(|y| (0..8).map(move |x| (x, y)))
                .map(|(x, y)| bsafe[(x, y)].symbol().to_string())
                .collect();
            assert_eq!(snapshot, after, "{name} used non-safe glyphs");
        }
    }

    /// Gated behind `RATRUN_ICON_DIR`: renders an icon sheet of every icon at
    /// both large (`8 × 8`) and small (`4 × 4`) sizes (plus evolved showcase)
    /// in Truecolor, 256-colour, and 16-colour ANSI files for visual review.
    #[test]
    fn dump_icon_sheet() {
        use std::fmt::Write as _;

        let Some(dir) = std::env::var_os("RATRUN_ICON_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();

        let color_rgb = |c: Color| -> (u8, u8, u8) {
            const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
            match c {
                Color::Rgb(r, g, b) => (r, g, b),
                Color::Black | Color::Reset => (19, 24, 27),
                Color::Red => (128, 0, 0),
                Color::Green => (0, 128, 0),
                Color::Yellow => (128, 128, 0),
                Color::Blue => (0, 0, 128),
                Color::Magenta => (128, 0, 128),
                Color::Cyan => (0, 128, 128),
                Color::Gray => (192, 192, 192),
                Color::DarkGray => (96, 96, 96),
                Color::LightRed => (255, 0, 0),
                Color::LightGreen => (0, 255, 0),
                Color::LightYellow => (255, 255, 0),
                Color::LightBlue => (0, 0, 255),
                Color::LightMagenta => (255, 0, 255),
                Color::LightCyan => (0, 255, 255),
                Color::White => (255, 255, 255),
                Color::Indexed(idx) if idx < 16 => match idx {
                    0 => (0, 0, 0),
                    1 => (128, 0, 0),
                    2 => (0, 128, 0),
                    3 => (128, 128, 0),
                    4 => (0, 0, 128),
                    5 => (128, 0, 128),
                    6 => (0, 128, 128),
                    7 => (192, 192, 192),
                    8 => (128, 128, 128),
                    9 => (255, 0, 0),
                    10 => (0, 255, 0),
                    11 => (255, 255, 0),
                    12 => (0, 0, 255),
                    13 => (255, 0, 255),
                    14 => (0, 255, 255),
                    _ => (255, 255, 255),
                },
                Color::Indexed(idx) if idx < 232 => {
                    let i = (idx - 16) as usize;
                    (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
                }
                Color::Indexed(idx) => {
                    let v = 8 + 10 * (idx - 232);
                    (v, v, v)
                }
            }
        };

        let icons = all_named_icons();
        let cols: u16 = 6;
        let cell_w: u16 = 18;
        let cell_h: u16 = 8;
        let rows_count = (icons.len() as u16).div_ceil(cols) + 1; // +1 row for evolved samples
        let sheet_w = cols * cell_w + 2;
        let sheet_h = rows_count * cell_h + 3;

        for (tag, depth) in [
            ("truecolor", ColorDepth::Truecolor),
            ("256", ColorDepth::Ansi256),
            ("16", ColorDepth::Ansi16),
        ] {
            let area = Rect::new(0, 0, sheet_w, sheet_h);
            let mut buf = Buffer::empty(area);
            let plate = palette::to_term((19, 24, 27), depth);
            let card_bg = palette::to_term((28, 34, 39), depth);
            let coat = palette::to_term((235, 232, 232), depth);
            let muted = palette::to_term((140, 152, 156), depth);
            buf.set_style(area, ratatui::style::Style::default().fg(coat).bg(plate));

            let header = format!(
                " RAT RUN ICON SHEET ({tag})  —  LARGE 8x8 (8x4 cells) + SMALL 4x4 (4x2 cells) "
            );
            buf.set_stringn(
                1,
                0,
                &header,
                (sheet_w - 2) as usize,
                ratatui::style::Style::default()
                    .fg(palette::to_term((255, 210, 90), depth))
                    .bg(plate),
            );

            for (idx, (name, icon)) in icons.iter().enumerate() {
                let col = (idx as u16) % cols;
                let row = (idx as u16) / cols;
                let x0 = 1 + col * cell_w;
                let y0 = 2 + row * cell_h;

                // Tile background plate
                let tile = Rect::new(x0, y0, cell_w - 1, cell_h - 1);
                buf.set_style(tile, ratatui::style::Style::default().fg(coat).bg(card_bg));

                // Label
                buf.set_stringn(
                    x0 + 1,
                    y0,
                    name,
                    (cell_w - 2) as usize,
                    ratatui::style::Style::default()
                        .fg(palette::to_term(icon.accent, depth))
                        .bg(card_bg),
                );

                // Large 8×8 icon (8×4 cells)
                draw_large_with_depth(&mut buf, Rect::new(x0 + 1, y0 + 2, 8, 4), icon, depth);

                // Small 4×4 icon (4×2 cells) beside it
                draw_small_with_depth(&mut buf, Rect::new(x0 + 11, y0 + 3, 4, 2), icon, depth);
                buf.set_stringn(
                    x0 + 11,
                    y0 + 1,
                    "4x4",
                    4,
                    ratatui::style::Style::default().fg(muted).bg(card_bg),
                );
            }

            // Bottom showcase row: Evolved frame treatment on sample weapons (both 10x5, 8x4, 4x2)
            let evo_row = (icons.len() as u16).div_ceil(cols);
            let ey = 2 + evo_row * cell_h;
            let samples = [
                ("Evo Gnaw", &GNAW),
                ("Evo Cheese", &CHEESE_SPRAY),
                ("Evo Nova", &SQUEAK_NOVA),
                ("Evo Acid", &ACID_SPIT),
                ("Evo Thrash", &TAIL_THRASH),
                ("Evo Laser", &LASER_EYES),
            ];
            for (i, (label, icon)) in samples.iter().enumerate() {
                let x0 = 1 + (i as u16) * cell_w;
                let tile = Rect::new(x0, ey, cell_w - 1, cell_h - 1);
                buf.set_style(tile, ratatui::style::Style::default().fg(coat).bg(card_bg));
                buf.set_stringn(
                    x0 + 1,
                    ey,
                    label,
                    (cell_w - 2) as usize,
                    ratatui::style::Style::default()
                        .fg(palette::to_term((255, 224, 88), depth))
                        .bg(card_bg),
                );
                // 10×5 framed evolved large icon
                draw_evolved_with_depth(&mut buf, Rect::new(x0 + 1, ey + 1, 10, 5), icon, depth);
                // 4×2 evolved small icon beside it
                draw_evolved_with_depth(&mut buf, Rect::new(x0 + 12, ey + 3, 4, 2), icon, depth);
            }

            let mut ansi = String::new();
            for y in 0..sheet_h {
                for x in 0..sheet_w {
                    let c = &buf[(x, y)];
                    let (r, g, b) = color_rgb(c.fg);
                    let (br, bg, bb) = color_rgb(c.bg);
                    write!(
                        ansi,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    )
                    .unwrap();
                }
                ansi.push_str("\x1b[0m\n");
            }
            let out = dir.join(format!("{sheet_w}x{sheet_h}-icons-{tag}.ansi"));
            std::fs::write(out, ansi).unwrap();
        }

        // Also capture after-integration UI screens (HUD, Level-Up, Pause, Keep-Three)
        // at 57×20, 80×24, and 120×40 into the same scratch directory.
        use crate::game::level::Theme;
        use crate::game::loadout::{Loadout, Upgrade};
        use crate::game::weapon::Weapon;
        use crate::game::world::World;
        use crate::render::{campaign, hud, menu};
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let dump_buf = |buf: &Buffer, w: u16, h: u16, name: &str| {
            let mut ansi = String::new();
            for y in 0..h {
                for x in 0..w {
                    let c = &buf[(x, y)];
                    let (r, g, b) = color_rgb(c.fg);
                    let (br, bg, bb) = color_rgb(c.bg);
                    write!(
                        ansi,
                        "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m{}",
                        c.symbol()
                    )
                    .unwrap();
                }
                ansi.push_str("\x1b[0m\n");
            }
            let out = dir.join(format!("{w}x{h}-screen-{name}.ansi"));
            std::fs::write(out, ansi).unwrap();
        };

        let mut world = World::new(Theme::Sewer, 7);
        world.elapsed = 115.0;
        world.player.level = 8;
        world.kills = 184;
        world.score = 2640;
        world.player.hp = 104.0;
        world.player.xp = (world.player.xp_to_next * 3) / 5;
        world.player.loadout.weapons = vec![
            Weapon::new(WeaponKind::Gnaw),
            Weapon::new(WeaponKind::CheeseSpray),
            Weapon::new(WeaponKind::AcidSpit),
            Weapon::new(WeaponKind::TailThrash),
            Weapon::new(WeaponKind::SonicSqueak),
        ];
        world.player.loadout.weapons[0].level = WeaponKind::Gnaw.max_level();
        world.player.loadout.weapons[1].level = 4;
        world.player.loadout.weapons[2].level = 2;
        world.player.loadout.weapons[3].level = 3;
        world.player.loadout.weapons[3].timer = 0.0; // READY
        world.player.loadout.weapons[4].level = 2;
        world.player.loadout.weapons[4].timer = 2.5; // charging
        for &(pk, count) in &[
            (PassiveKind::MaxHp, 3u32),
            (PassiveKind::MoveSpeed, 2),
            (PassiveKind::Damage, 4),
            (PassiveKind::FireRate, 2),
            (PassiveKind::Armor, 6), // capped -> gold
        ] {
            for _ in 0..count {
                world.player.loadout.apply(&Upgrade::Passive(pk));
            }
        }

        let mut hud_state = hud::HudState::default();
        hud_state.tick(1.0 / 60.0, &world, &[], false);
        hud_state.tick(1.0 / 60.0, &world, &[], false);

        let upgrades = vec![
            Upgrade::NewWeapon(WeaponKind::SqueakNova),
            Upgrade::LevelWeapon(1),
            Upgrade::Passive(PassiveKind::Magnet),
        ];
        let desc: Vec<_> = upgrades
            .iter()
            .map(|u| world.player.loadout.describe(u))
            .collect();

        for (w, h) in [(57u16, 20u16), (80, 24), (120, 40)] {
            // 1. HUD + Playfield
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| {
                let area = f.area();
                let top_h = hud::top_height(area);
                let top = Rect::new(area.x, area.y, area.width, top_h);
                let game = Rect::new(
                    area.x,
                    area.y + top_h,
                    area.width,
                    area.height.saturating_sub(top_h + 1),
                );
                let bottom = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
                let mut pb = crate::render::framebuffer::PixelBuffer::new(
                    game.width as usize,
                    game.height as usize * 2,
                );
                world.draw(&mut pb);
                pb.render_to(game, f.buffer_mut());
                hud::draw_top(f, top, &world, &mut hud_state);
                hud::draw_bottom(f, bottom, &world, &mut hud_state);
            })
            .unwrap();
            dump_buf(term.backend().buffer(), w, h, "hud");

            // 2. Level-Up screen over playfield
            let mut hits = menu::HitMap::default();
            term.draw(|f| {
                let area = f.area();
                let top_h = hud::top_height(area);
                let top = Rect::new(area.x, area.y, area.width, top_h);
                let bottom = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
                f.buffer_mut().set_style(
                    area,
                    ratatui::style::Style::default().bg(Color::Rgb(11, 15, 17)),
                );
                hud::draw_top(f, top, &world, &mut hud_state);
                hud::draw_bottom(f, bottom, &world, &mut hud_state);
                menu::draw_levelup_with_loadout(
                    f,
                    area,
                    world.player.level,
                    &desc,
                    &upgrades,
                    &world.player.loadout,
                    0,
                    false,
                    "U",
                    &mut hits,
                );
            })
            .unwrap();
            dump_buf(term.backend().buffer(), w, h, "levelup");

            // 3. Pause screen
            hits.clear();
            term.draw(|f| {
                let area = f.area();
                f.buffer_mut().set_style(
                    area,
                    ratatui::style::Style::default().bg(Color::Rgb(11, 15, 17)),
                );
                menu::draw_pause_with_run(
                    f,
                    area,
                    &world,
                    false,
                    &crate::controls::Controls::default(),
                    &mut hits,
                );
            })
            .unwrap();
            dump_buf(term.backend().buffer(), w, h, "pause");

            // 4. Campaign Keep-Three carry-over manifest (80×24 & 120×40)
            if w >= 80 {
                let items = [
                    campaign::UpgradeEntry {
                        name: "Gnaw",
                        glyph: '▸',
                        color: WeaponKind::Gnaw.color(),
                        category: campaign::UpgradeCategory::Weapon { level: 5, max_level: 5 },
                        detail: WeaponKind::Gnaw.desc(),
                    },
                    campaign::UpgradeEntry {
                        name: "Cheese Spray",
                        glyph: '»',
                        color: WeaponKind::CheeseSpray.color(),
                        category: campaign::UpgradeCategory::Weapon { level: 4, max_level: 5 },
                        detail: WeaponKind::CheeseSpray.desc(),
                    },
                    campaign::UpgradeEntry {
                        name: "Tail Thrash",
                        glyph: 'ϟ',
                        color: WeaponKind::TailThrash.color(),
                        category: campaign::UpgradeCategory::Weapon { level: 3, max_level: 5 },
                        detail: WeaponKind::TailThrash.desc(),
                    },
                    campaign::UpgradeEntry {
                        name: "Fat Reserves",
                        glyph: '+',
                        color: MAX_HP.accent,
                        category: campaign::UpgradeCategory::Passive { stacks: 3 },
                        detail: PassiveKind::MaxHp.desc(),
                    },
                    campaign::UpgradeEntry {
                        name: "Thick Hide",
                        glyph: '#',
                        color: ARMOR.accent,
                        category: campaign::UpgradeCategory::Passive { stacks: 6 },
                        detail: PassiveKind::Armor.desc(),
                    },
                ];
                let sel = [true, false, true, true, false];
                let view = campaign::KeepThreeView {
                    from_level: campaign::CampaignLevel::Sewer,
                    next_level: campaign::CampaignLevel::Kitchen,
                    route: [None, None],
                    gold: 12,
                    items: &items,
                    selected: &sel,
                    max_keep: 3,
                    cursor: 1,
                    scroll: 0,
                    hovered: None,
                    splices: &[],
                };
                term.draw(|f| {
                    let area = f.area();
                    f.buffer_mut().set_style(
                        area,
                        ratatui::style::Style::default().bg(Color::Rgb(11, 15, 17)),
                    );
                    campaign::draw_keep_three(f, area, &view);
                })
                .unwrap();
                dump_buf(term.backend().buffer(), w, h, "keep3");
            }
        }
        let _ = Loadout::starting();
    }
}
