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
SCREEN_POINTS = 150
CONFIRM_RUNS = 300
SCREEN_SEED = 2000
CONFIRM_SEED = 7000


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
)
BOUNDS = [(k.low, k.high) for k in KNOBS]


def overlay(values, defaults):
    grouped = {}

    def add(table, key, value, base, reason):
        if value != base:
            grouped.setdefault(table, {})[key] = value, base, reason

    def scaled(value, scale):
        return round(value * scale, 8)

    specs = {knob.name: knob for knob in KNOBS}
    params = dict(zip(specs, values, strict=True))
    director = defaults["director"]
    if params["pressure_rate_bonus"] > 0:
        add("director", "pressure_beats", True, False, "Enable pressure beats.")
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
        ):
            add("director", key, round(params[key], 8), director[key], specs[key].reason)
        carried = params["pressure_carried_scale"]
        scales = [params["pressure_sewer_scale"], carried * .85, carried, carried * 1.15]
        add("director", "pressure_sector_scale", [round(x, 8) for x in scales],
            director["pressure_sector_scale"], specs["pressure_carried_scale"].reason)
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
        base = defaults[table][key]
        spec = specs[knob]
        value = scaled(base, params[knob]) if spec.unit == "×" else round(float(params[knob]), 8)
        add(table, key, value, base, spec.reason)

    # The director scales boss HP by build_hp_level. Cancel those sector
    # changes so effective boss HP follows its own multiplier alone.
    build = defaults["bosses"]["build_level"]
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
            f"# Shipped {json.dumps(base)}. {reason}\n{key} = {json.dumps(value)}"
            for key, (value, base, reason) in rows.items()
        )
        for table, rows in grouped.items()
    ) + ("\n" if grouped else "")


def manifest(kind, defaults, solution=None):
    if kind == "pilot":
        points = {"defaults": [], "beats": []}
        seed, n = 1000, 2
    elif kind == "screen":
        design = qmc.LatinHypercube(d=len(KNOBS), seed=1729, optimization="random-cd")
        lower, upper = zip(*BOUNDS, strict=True)
        values = [[knob.shipped for knob in KNOBS], *qmc.scale(design.random(SCREEN_POINTS - 1), lower, upper).tolist()]
        points = {f"p{i:03}": point for i, point in enumerate(values)}
        seed, n = SCREEN_SEED, 8
    elif kind == "confirm":
        points = {"candidate": solution["values"]}
        seed, n = CONFIRM_SEED, CONFIRM_RUNS
    else:
        raise ValueError(kind)

    jobs = []
    for point, settings in points.items():
        tuning = "[director]\npressure_beats = true\n" if point == "beats" else (
            overlay(settings, defaults) if settings else ""
        )
        for skill in SKILLS:
            for start in range(seed, seed + n, 8):
                jobs.append({
                    "id": f"{point}-{skill}-{start}", "point": point, "tuning": tuning,
                    "seed": start, "skill": skill, "runs": min(8, seed + n - start),
                })
    spec = {"name": kind, "points": points, "jobs": jobs}
    if kind != "pilot":
        spec["bounds"] = BOUNDS
    return spec


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["pilot", "screen", "confirm"])
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--solution", type=Path)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    defaults = tomllib.loads(args.defaults.read_text())
    choice = None
    if args.kind == "confirm":
        choice = json.loads((args.solution or args.output / "solution.json").read_bytes())
    spec = manifest(args.kind, defaults, choice)
    args.output.mkdir(parents=True, exist_ok=True)
    dest = args.output / f"{args.kind}-manifest.json"
    dest.write_text(json.dumps(spec, indent=2))
    print(f"{len(spec['jobs'])} shards and {sum(j['runs'] for j in spec['jobs'])} campaigns in {dest}")


if __name__ == "__main__":
    main()
