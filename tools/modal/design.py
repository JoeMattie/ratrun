"""Generate the feel-bounded screening and confirmation manifests."""

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import tomllib

from scipy.stats import qmc

from app import OUTPUT, SKILLS

SECTORS = ("sewer", "kitchen", "lab", "flatbed")
ROUTE_BOSSES = ("Cat", "HorseshoeCrab", "Broom", "MegaRat", "Mower", "Cobra", "Magpie", "Hawk")
SCREEN_POINTS = 135
CONFIRM_RUNS = 300
META_RUNS = 96
EXPERIMENT_RUNS = 96
SCREEN_SEED = 4000
CONFIRM_SEED = 8000


@dataclass(frozen=True)
class Knob:
    name: str
    low: float
    high: float
    reason: str
    unit: str = "×"
    shipped: float = 1.


KNOBS = (
    *(
        Knob(f"{floor}_{key}", low, high, reason)
        for floor in SECTORS
        for key, low, high, reason in (
            ("enemy_hp", .60, 1.50, "Ordinary enemy health relative to shipped."),
            ("contact_endpoint", .60, 1.45, "End-of-buildup contact damage relative to shipped."),
            ("live_budget", .70, 1.35, "Live ordinary-body budget relative to shipped."),
            ("spawn_interval", .70, 1.40, "Seconds between ordinary pulses relative to shipped."),
            ("elite_interval", .75, 1.35, "Elite arrival interval relative to shipped."),
        )
    ),
    Knob("enemy_speed", .85, 1.15, "Ordinary movement speed relative to shipped."),
    Knob("player_hp", 100, 160, "Starting health before upgrades.", " HP", 130),
    Knob("hit_iframes", .50, 1.20, "Protection after a projectile, boss or hazard hit.", " s", .90),
    Knob("contact_iframes", .50, 1.20, "Protection after ordinary enemy contact.", " s", .65),
    Knob("regen", .75, 1.25, "Starting regeneration relative to shipped."),
    Knob("xp_cost", .80, 1.25, "XP cost of each geometric progression step relative to shipped."),
    Knob("boss_hp", .70, 1.60, "Effective health of each boss relative to shipped."),
    Knob("boss_attack_damage", .75, 1.35, "Default-route boss hazard damage relative to shipped."),
    Knob("boss_tell_time", .80, 1.35, "Default-route boss hazard warning duration relative to shipped."),
    Knob("jaguar_swipe_damage", .75, 1.25, "Jaguar swipe base damage before boss hazard scaling."),
    Knob("jaguar_swipe_windup", .80, 1.25, "Jaguar swipe windup before boss warning scaling."),
    Knob("pressure_early_at", 185, 225, "First surge start.", " s", 205),
    Knob("pressure_late_at", 660, 700, "Second surge start.", " s", 675),
    Knob("pressure_early_len", 100, 135, "First surge length.", " s", 125),
    Knob("pressure_late_len", 100, 130, "Second surge length.", " s", 125),
    Knob("pressure_recover_early", 195, 270, "First recovery length.", " s", 220),
    Knob("pressure_recover_late", 55, 70, "Second recovery length.", " s", 65),
    Knob("pressure_rate_bonus", 0, 1.1, "Fractional cadence uplift, automatically balanced by recovery.", shipped=0),
    Knob("pressure_budget_bonus", .15, .70, "Fractional live-budget uplift in a surge.", shipped=.35),
    Knob("pressure_contact_bonus", .25, 1.25, "Fractional contact damage uplift in a surge.", shipped=.4),
    Knob("pressure_recovery_budget_cut", .15, .65, "Fractional live-budget cut in recovery.", shipped=.25),
    Knob("pressure_recovery_heal", .15, .75, "Fraction of other rare finds converted to Medkits in recovery.", shipped=.25),
    Knob("pressure_sewer_scale", .30, .90, "Sewer beat strength.", shipped=.65),
    Knob("pressure_carried_scale", .75, 1.45, "Beat strength for Kitchen, Lab and Flatbed, with later-floor weighting.", shipped=1.15),
    Knob("pressure_sewer_opening_secs", 120, 215, "Time for Sewer opening contact grace.", " s", 150),
    Knob("pressure_sewer_opening_cut", .25, .85, "Fractional contact damage cut in the Sewer opening.", shipped=.35),
    Knob("power_hp_exponent", .25, .85, "Strength of the response to relative weapon throughput.", shipped=.45),
    Knob("power_hp_floor", .55, .90, "Minimum horde HP multiplier.", shipped=.70),
    Knob("power_hp_ceiling", 1.15, 1.65, "Maximum horde HP multiplier.", shipped=1.40),
    Knob("power_memory_secs", 14, 24, "Damage averaging time constant.", " s", 18),
    Knob("power_reference_scale", .80, 1.20, "Shift the neutral DPS reference in all sectors."),
)
BOUNDS = [(k.low, k.high) for k in KNOBS]


def overlay(values, anchor):
    grouped = {}

    def add(table, key, value, base, reason):
        grouped.setdefault(table, {})[key] = value, base, reason

    def scaled(value, scale):
        return round(value * scale, 8)

    specs = {knob.name: knob for knob in KNOBS}
    params = dict(zip(specs, values, strict=True))
    director = anchor["director"]
    add("director", "pressure_beats", True, director["pressure_beats"], "Enable pressure beats.")
    for key, names in (
        ("pressure_starts", ("pressure_early_at", "pressure_late_at")),
        ("pressure_lengths", ("pressure_early_len", "pressure_late_len")),
        ("pressure_recovery_lengths", ("pressure_recover_early", "pressure_recover_late")),
    ):
        add("director", key, [round(params[name], 3) for name in names], director[key],
            "Pair of buildup windows in simulation seconds.")
    for key in (
        "pressure_rate_bonus", "pressure_budget_bonus", "pressure_contact_bonus",
        "pressure_recovery_budget_cut", "pressure_recovery_heal",
        "pressure_sewer_opening_secs", "pressure_sewer_opening_cut",
        "power_hp_exponent", "power_hp_floor", "power_hp_ceiling", "power_memory_secs",
    ):
        add("director", key, round(params[key], 8), director[key], specs[key].reason)
    carried = params["pressure_carried_scale"]
    scales = [params["pressure_sewer_scale"], carried * .85, carried, carried * 1.15]
    add("director", "pressure_sector_scale", [round(x, 8) for x in scales],
        director["pressure_sector_scale"], specs["pressure_carried_scale"].reason)
    add("director", "power_scaling", True, False, "Match incoming horde HP to measured damage.")
    reference = director["power_reference_hp"]
    add("director", "power_reference_hp", [scaled(hp, params["power_reference_scale"]) for hp in reference],
        reference, specs["power_reference_scale"].reason)
    hp_scales = [params[f"{floor}_enemy_hp"] for floor in SECTORS]
    damage_scales = [params[f"{floor}_contact_endpoint"] for floor in SECTORS]
    for key, scales, reason in (
        ("build_hp_level", hp_scales, "Independent ordinary enemy HP scales by sector."),
        ("level_damage_top", damage_scales, "Independent end-of-buildup contact damage scales by sector."),
    ):
        base = director[key]
        add("director", key, [scaled(a, b) for a, b in zip(base, scales, strict=True)], base, reason)

    budget = director["live_budget"]
    # The shipped base interval is linear throughout the 900 s buildup. Using
    # its knots at these clocks preserves that shape and changes frequency alone.
    pulse = [[at, round(max(.35, 1.6 - at / 900), 8)] for at, _ in director["bodies_per_second"]]
    elite = director["elite_interval_mult"]
    for i, floor in enumerate(SECTORS, start=1):
        table = f"director.sectors.{i}"
        add(table, "live_budget", [[at, round(count * params[f"{floor}_live_budget"])] for at, count in budget],
            budget, f"{floor.title()} live budget by buildup time.")
        add(table, "spawn_interval", [[at, scaled(delay, params[f"{floor}_spawn_interval"])] for at, delay in pulse],
            pulse, f"{floor.title()} pulse interval.")
        add(table, "elite_interval_mult", scaled(elite, params[f"{floor}_elite_interval"]),
            elite, f"{floor.title()} elite arrival interval.")

    for knob, table, key in (
        ("enemy_speed", "enemies", "speed_mult"), ("player_hp", "player", "max_hp"),
        ("hit_iframes", "player", "hit_iframes"), ("contact_iframes", "player", "contact_iframes"),
        ("regen", "player", "regen"), ("xp_cost", "xp", "base"), ("boss_hp", "bosses", "hp_mult"),
    ):
        base = anchor[table][key]
        spec = specs[knob]
        value = scaled(base, params[knob]) if spec.unit == "×" else round(float(params[knob]), 8)
        add(table, key, value, base, spec.reason)

    # The director scales boss HP by build_hp_level. Cancel those sector
    # changes so effective boss HP follows its own multiplier alone.
    build = anchor["bosses"]["build_level"]
    add("bosses", "build_level", [scaled(a, 1. / b) for a, b in zip(build, hp_scales, strict=True)],
        build, "Cancels sector enemy HP scaling in the boss HP formula.")

    for boss in ROUTE_BOSSES:
        table = f"bosses.kinds.{boss}"
        add(table, "attack_damage_mult", round(float(params["boss_attack_damage"]), 8), 1.,
            "Multiplier for this boss's hazard damage.")
        add(table, "tell_time_mult", round(float(params["boss_tell_time"]), 8), 1.,
            "Multiplier for this boss's hazard warning time.")
    add("bosses.kinds.Cat", "swipe_damage", scaled(22., params["jaguar_swipe_damage"]), 22.,
        "Jaguar swipe damage before the attack multiplier and armor.")
    add("bosses.kinds.Cat", "swipe_windup_mult", round(float(params["jaguar_swipe_windup"]), 8), 1.,
        "Jaguar swipe windup multiplier, compounded with tell_time_mult.")

    return "\n\n".join(
        f"[{table}]\n" + "\n".join(
            f"# Feel anchor {json.dumps(base)}. {reason}\n{key} = {json.dumps(value)}"
            for key, (value, base, reason) in rows.items()
        )
        for table, rows in grouped.items()
    ) + ("\n" if grouped else "")


def study_anchor(defaults, anchor):
    if not defaults["director"]["pressure_beats"] or defaults["director"]["power_scaling"]:
        raise ValueError("Use the shipped round 3 surge defaults with power scaling off")
    director = anchor["director"]
    director.update({key: value for key, value in defaults["director"].items() if key.startswith("power_")})
    return anchor


def jobs_for(point, tuning, seed, n, meta="none"):
    return [
        {"id": f"{point}-{skill}-{start}", "point": point, "tuning": tuning,
         "seed": start, "skill": skill, "meta": meta, "runs": min(8, seed + n - start)}
        for skill in SKILLS for start in range(seed, seed + n, 8)
    ]


def manifest(kind, anchor, solution=None, experiment=""):
    if kind == "pilot":
        points = {"round3": [], "power": []}
        jobs = jobs_for("round3", "", 1000, 2) + jobs_for("power", "[director]\npower_scaling = true\n", 1000, 2)
    elif kind == "screen":
        design = qmc.LatinHypercube(d=len(KNOBS), seed=1729, optimization="random-cd")
        lower, upper = zip(*BOUNDS, strict=True)
        values = [[knob.shipped for knob in KNOBS], *qmc.scale(design.random(SCREEN_POINTS - 1), lower, upper).tolist()]
        points = {f"p{i:03}": point for i, point in enumerate(values)}
        jobs = [job for point, values in points.items()
                for job in jobs_for(point, overlay(values, anchor), SCREEN_SEED, 8)]
        jobs += jobs_for("round3", "", SCREEN_SEED, 8)
        jobs += jobs_for("half-hp-double-enemy", experiment, SCREEN_SEED, 8)
    elif kind == "confirm":
        points = {"candidate": solution["values"]}
        tuning = overlay(solution["values"], anchor)
        jobs = jobs_for("candidate", tuning, CONFIRM_SEED, CONFIRM_RUNS)
        jobs += jobs_for("power-off", tuning.replace("power_scaling = true", "power_scaling = false"),
                         CONFIRM_SEED, CONFIRM_RUNS)
        jobs += jobs_for("round3", "", CONFIRM_SEED, CONFIRM_RUNS)
        jobs += jobs_for("half-hp-double-enemy", experiment, CONFIRM_SEED, EXPERIMENT_RUNS)
        for meta in ("mid", "max"):
            jobs += jobs_for(f"candidate-{meta}", tuning, CONFIRM_SEED, META_RUNS, meta)
    else:
        raise ValueError(kind)
    spec = {"name": kind, "points": points, "jobs": jobs}
    if kind != "pilot":
        spec["bounds"] = BOUNDS
    return spec


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["pilot", "screen", "confirm"])
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--anchor", type=Path, required=True, help="Round 3 screening's original defaults")
    parser.add_argument("--experiment", type=Path, help="Owner's half-HP, double-enemy-HP overlay")
    parser.add_argument("--solution", type=Path)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    anchor = study_anchor(tomllib.loads(args.defaults.read_text()), tomllib.loads(args.anchor.read_text()))
    if args.kind != "pilot" and not args.experiment:
        parser.error("--experiment is required for screening or confirmation")
    choice = None
    if args.kind == "confirm":
        choice = json.loads((args.solution or args.output / "solution.json").read_bytes())
    spec = manifest(args.kind, anchor, choice, args.experiment.read_text() if args.experiment else "")
    args.output.mkdir(parents=True, exist_ok=True)
    dest = args.output / f"{args.kind}-manifest.json"
    dest.write_text(json.dumps(spec, indent=2))
    print(f"{len(spec['jobs'])} shards and {sum(j['runs'] for j in spec['jobs'])} campaigns in {dest}")


if __name__ == "__main__":
    main()
