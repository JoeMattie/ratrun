"""Create reproducible screening, augmentation and confirmation manifests."""

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import tomllib

import numpy as np
from scipy.stats import qmc

from app import OUTPUT, SKILLS


@dataclass(frozen=True)
class Knob:
    name: str
    low: float
    high: float
    reason: str
    fields: tuple[str, ...]


KNOBS = (
    Knob("sewer_enemy_hp", .50, 1.60, "Tune the opening floor independently of the carried build.",
         ("director.build_hp_level",)),
    Knob("later_enemy_hp", .60, 1.70, "Set ordinary enemy HP pressure after the first keep.",
         ("director.build_hp_level",)),
    Knob("enemy_contact", .60, 1.60, "Set the cost of contact and attacks that read body damage.",
         ("enemies.contact_damage_mult",)),
    Knob("enemy_speed", .75, 1.35, "Set the speed at which ordinary bodies close an escape lane.",
         ("enemies.speed_mult",)),
    Knob("live_budget", .60, 1.65, "Set the size of the largest buildup crowds.",
         ("director.live_budget",)),
    Knob("spawn_interval", .60, 1.60, "Set the delay between buildup pulses.",
         ("director.spawn_interval_mult",)),
    Knob("elite_interval", .60, 1.75, "Set the time between timed elites.",
         ("director.elite_interval_mult",)),
    Knob("player_hp", .75, 1.50, "Set the player's buffer against successive hits.",
         ("player.max_hp",)),
    Knob("regen", .45, 1.85, "Set recovery between waves of damage.", ("player.regen",)),
    Knob("iframes", .65, 1.60, "Set repeat-hit protection after both contact and other hits.",
         ("player.hit_iframes", "player.contact_iframes")),
    Knob("opening_xp_cost", .45, 1.65, "Move the first card by changing the base step cost.",
         ("xp.base",)),
    Knob("xp_steps", .50, 1.75, "Set late card pacing with two to seven steps per level.",
         ("xp.steps_per_level",)),
    Knob("level_damage", .60, 1.60, "Set the attack power earned with each level.",
         ("xp.damage_per_level",)),
    Knob("boss_hp", .50, 1.70, "Set boss fight length through the shared boss HP multiplier.",
         ("bosses.hp_mult",)),
    Knob("boss_hp_contrast", .80, 2.25, "Raise Sewer boss HP and reduce Flatbed boss HP by reciprocal factors.",
         ("bosses.build_level",)),
    Knob("pickup_healing", .55, 1.65, "Set recovery from medkits and their availability in ordinary drops.",
         ("loot.heal_pickup_fraction", "loot.pickup_chances")),
)
INITIAL_BOUNDS = [(k.low, k.high) for k in KNOBS]
SCREEN_POINTS = 176
AUGMENT_POINTS = 33
CONFIRM_RUNS = 300


def overlay(values, defaults):
    fields = {}
    grouped = {}
    for v, knob in zip(values, KNOBS, strict=True):
        for field in knob.fields:
            table, name = field.split(".")
            base = defaults[table][name]
            if knob.name == "sewer_enemy_hp":
                value = [round(base[0] * v, 6), *base[1:]]
            elif knob.name == "later_enemy_hp":
                value = [fields[field][0], *(round(x * v, 6) for x in base[1:])]
            elif name == "live_budget":
                value = [[at, round(count * v)] for at, count in base]
            elif name == "steps_per_level":
                value = max(1, round(base * v))
            elif name == "pickup_chances":
                value = [*(round(min(1, p * v), 6) for p in base[:2]), base[2]]
            elif name == "heal_pickup_fraction":
                value = round(min(1, base * v), 6)
            elif knob.name == "boss_hp_contrast":
                # The director's build_hp_level is also in the boss HP
                # formula. Compensate for it so the boss levers stay independent.
                enemy_hp = [values[0], *([values[1]] * 3)]
                contrast = [v, 1., 1., 1. / v]
                value = [round(b * c / e, 6) for b, c, e in zip(base, contrast, enemy_hp, strict=True)]
            else:
                value = round(base * v, 6)
            fields[field] = value
            reason = f"{knob.reason} Sampled at {v:.4f}×."
            if knob.name == "later_enemy_hp":
                reason = f"Sewer at {values[0]:.4f}×. {reason}"
            if knob.name == "boss_hp_contrast":
                reason += " Cancels the ordinary enemy HP scales in the boss formula."
            grouped.setdefault(table, {})[name] = value, base, reason
    return "\n".join(
        f"[{table}]\n" + "\n".join(
            f"# Shipped {json.dumps(base)}. {reason}\n{name} = {json.dumps(value)}\n"
            for name, (value, base, reason) in rows.items()
        )
        for table, rows in grouped.items()
    )


def design_points(bounds, n, seed):
    u = qmc.LatinHypercube(d=len(KNOBS), seed=seed).random(n)
    lower, upper = np.array(bounds).T
    points = qmc.scale(u, lower, upper)
    # Fit the step knob on the realizable integer choices. Optimize there too.
    step = next(i for i, knob in enumerate(KNOBS) if knob.name == "xp_steps")
    points[:, step] = np.rint(points[:, step] * 4) / 4
    return points.tolist()


def manifest(kind, defaults, solution=None):
    bounds = INITIAL_BOUNDS
    identifiers = []
    if kind == "pilot":
        values = [[1.] * len(KNOBS)]
        identifiers = ["defaults"]
        seed, n = 1000, 2
    elif kind == "screen":
        values = [[1.] * len(KNOBS), *design_points(bounds, SCREEN_POINTS - 1, 1729)]
        identifiers = [f"p{i:03}" for i in range(len(values))]
        seed, n = 1000, 8
    elif kind == "augment":
        bounds = solution["expanded_bounds"]
        near = [
            (max(lo, value - .35 * (hi - lo)), min(hi, value + .35 * (hi - lo)))
            for value, (lo, hi) in zip(solution["values"], bounds, strict=True)
        ]
        n = (AUGMENT_POINTS - 1) // 2
        values = [solution["values"], *design_points(bounds, n, 2718),
                  *design_points(near, AUGMENT_POINTS - 1 - n, 3141)]
        identifiers = [f"a{i:03}" for i in range(len(values))]
        seed, n = 1000, 8
    elif kind == "confirm":
        values = [solution["values"]]
        identifiers = ["candidate"]
        seed, n = 5000, CONFIRM_RUNS
    else:
        raise ValueError(kind)

    points = dict(zip(identifiers, values, strict=True))
    jobs = []
    for point, multipliers in points.items():
        tuning = overlay(multipliers, defaults)
        for skill in SKILLS:
            for start in range(seed, seed + n, 8):
                jobs.append({
                    "id": f"{point}-{skill}-{start}", "point": point, "tuning": tuning,
                    "seed": start, "skill": skill, "runs": min(8, seed + n - start),
                })
    return {"name": kind, "points": points, "bounds": bounds, "jobs": jobs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["pilot", "screen", "augment", "confirm"])
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--solution", type=Path, default=OUTPUT / "solution.json")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    with args.defaults.open("rb") as source:
        defaults = tomllib.load(source)
    choice = json.loads(args.solution.read_bytes()) if args.kind in {"augment", "confirm"} else None
    spec = manifest(args.kind, defaults, choice)
    args.output.mkdir(parents=True, exist_ok=True)
    dest = args.output / f"{args.kind}-manifest.json"
    dest.write_text(json.dumps(spec, indent=2))
    print(f"{len(spec['jobs'])} shards and {sum(j['runs'] for j in spec['jobs'])} campaigns in {dest}")


if __name__ == "__main__":
    main()
