"""Fit no-carry sectors against a fixed, owner-calibrated Sewer."""

import argparse
import json
from pathlib import Path
import tomllib

from scipy.stats import qmc
from threadpoolctl import threadpool_limits

from app import OUTPUT, RATE, SIM_SECONDS, SKILLS
from design import CONFIRM_RUNS, EXPERIMENT_RUNS, META_RUNS, Knob, jobs_for
from fit import fit, load_stage, observe
from report import confirmed_cohorts, profile_comparison, table, wilson
from study import cost_estimate, run_stage, write_json

FLOORS = ("kitchen", "lab", "flatbed")
SCREEN_POINTS, SCREEN_SEED, CONFIRM_SEED = 96, 9200, 10000
POWER_FLOOR, POWER_CEILING, SEWER_HP = .90, 1.15, 2.45
SIZES = {"candidate": CONFIRM_RUNS, "baseline": 144, "power-off": 144,
         "half-hp-double-enemy": EXPERIMENT_RUNS, "candidate-mid": META_RUNS, "candidate-max": META_RUNS}
KNOBS = tuple(
    Knob(f"{floor}_{field}", low, high, reason)
    for floor in FLOORS
    for field, low, high, reason in (
        ("build_hp", .65, 1., "Fresh horde HP correction."),
        ("hp_ramp", .45, 1., "Fraction of the shipped late-floor horde HP ramp."),
        ("boss_hp", .72, 1., "Combined fresh boss HP relative to shipped."),
        ("miniboss_hp", .70, 1., "Fresh miniboss HP at the four appointments."),
        ("pressure", .68, 1., "Fresh surge strength relative to shipped."),
        ("reference", .80, 1.15, "Fresh damage reference for power scaling."),
    )
)
BOUNDS = [(knob.low, knob.high) for knob in KNOBS]


def overlay(values, defaults):
    params = dict(zip((knob.name for knob in KNOBS), values, strict=True))
    d = defaults["director"]

    def factors(field):
        return [params[f"{floor}_{field}"] for floor in FLOORS]

    def scale(items, multipliers):
        return [round(a * b, 8) for a, b in zip(items, multipliers, strict=True)]

    build = factors("build_hp")
    boss = [a / b for a, b in zip(factors("boss_hp"), build, strict=True)]
    reference = d["power_reference_hp"].copy()
    reference[0] = round(reference[0] * SEWER_HP, 8)
    rows = {
        "campaign": {"carry": False},
        "player": {"sector_hp_mult": [.5, 1., 1., 1.]},
        "enemies": {"sector_hp_mult": [SEWER_HP, *([round(1 / POWER_CEILING, 8)] * 3)]},
        "director": {
            "power_scaling": True, "power_hp_exponent": .45, "power_memory_secs": 18.,
            "power_hp_floor": POWER_FLOOR, "power_hp_ceiling": POWER_CEILING,
            "power_reference_hp": reference,
            "power_fresh_reference_hp": scale(d["power_fresh_reference_hp"], factors("reference")),
            "fresh_build_hp_level": scale(d["fresh_build_hp_level"], build),
            "fresh_level_hp_extra": [round(1 + (a - 1) * b, 8) for a, b in
                                     zip(d["fresh_level_hp_extra"], factors("hp_ramp"), strict=True)],
            "fresh_pressure_sector_scale": scale(d["fresh_pressure_sector_scale"], factors("pressure")),
        },
        "bosses": {"fresh_build_level": scale(defaults["bosses"]["fresh_build_level"], boss)},
        "minibosses": {"fresh_hp": [scale(row, [p] * len(row)) for row, p in
                                   zip(defaults["minibosses"]["fresh_hp"], factors("miniboss_hp"), strict=True)]},
    }
    return "\n\n".join(f"[{section}]\n" + "\n".join(f"{key} = {json.dumps(value)}"
                      for key, value in entries.items()) for section, entries in rows.items()) + "\n"


def manifest(stage, defaults, experiment, solution=None):
    if stage == "screen":
        lo, hi = zip(*BOUNDS, strict=True)
        design = qmc.LatinHypercube(d=len(KNOBS), seed=3137, optimization="random-cd")
        values = [[knob.shipped for knob in KNOBS], *qmc.scale(design.random(SCREEN_POINTS - 1), lo, hi).tolist()]
        points = {f"p{i:03}": point for i, point in enumerate(values)}
        jobs = [job for point, vals in points.items()
                for job in jobs_for(point, overlay(vals, defaults), SCREEN_SEED, 8)]
        jobs += jobs_for("baseline", "", SCREEN_SEED, 8)
        jobs += jobs_for("half-hp-double-enemy", experiment, SCREEN_SEED, 8)
    elif stage == "confirm":
        points = {"candidate": solution}
        tuning = overlay(solution, defaults)
        profiles = {"candidate": tuning, "baseline": "",
                    "power-off": tuning.replace("power_scaling = true", "power_scaling = false"),
                    "half-hp-double-enemy": experiment, "candidate-mid": tuning, "candidate-max": tuning}
        meta = {"candidate-mid": "mid", "candidate-max": "max"}
        jobs = [job for point, config in profiles.items()
                for job in jobs_for(point, config, CONFIRM_SEED, SIZES[point], meta.get(point, "none"))]
        for job in jobs:
            job["max_sim_seconds"] = 2 * SIM_SECONDS
    else:
        raise ValueError(stage)
    return {"name": stage, "points": points, "jobs": jobs, "bounds": BOUNDS}


def write_report(output, result, budget):
    confirm, campaigns, confirm_seconds = load_stage(output, "confirm")
    pilot, _, pilot_seconds = load_stage(output, "pilot")
    if confirm["source_revision"] != result["revision"] or pilot["source_revision"] != result["revision"]:
        raise ValueError("No-carry study stages must use the same simulator")
    if confirm["points"]["candidate"] != result["values"] or any(
        job["tuning"] != result["defaults_overlay"] for job in confirm["jobs"] if job["point"] == "candidate"
    ):
        raise ValueError("No-carry confirmation used a different candidate")
    cohorts = confirmed_cohorts(campaigns, SIZES, CONFIRM_SEED)
    profiles = {point: {skill: observe(runs) for skill, runs in cohort.items()} for point, cohort in cohorts.items()}
    lines = ["# Rat Run round 4 with fresh starts", "",
             f"Revision {result['revision']}. Screener seeds {SCREEN_SEED}–{SCREEN_SEED + 7}. "
             f"Fresh confirmation began at {CONFIRM_SEED}.", "",
             "The modeled Sewer held the owner's half-HP start and 1.10–1.41× the horde HP of the half-HP experiment. "
             "The optimizer varied later-sector settings. Later ordinary and named HP stayed at or below shipped. "
             "All six target families and weights carried over.", ""]
    scores, clears = profile_comparison(lines, cohorts, profiles, baseline="baseline")
    lines += ["## Fit", "", f"{len(result['train'])} training and {len(result['holdout'])} holdout settings. "
              f"Predicted weighted loss {result['loss']:.3f}.", ""]
    table(lines, ("Skill", "Predicted wins", "Held-out Brier", "Constant Brier"), [
        (s, f"{result['candidate'][s]['win']:.1%}", f"{result['validation'][s]['win']['brier']:.3f}",
         f"{result['validation'][s]['win']['baseline_brier']:.3f}") for s in SKILLS
    ])
    elapsed = pilot_seconds + result["screen_seconds"] + confirm_seconds
    lines += ["## Compute", "", f"Successful receipts used {elapsed:,.0f} compute seconds, about "
              f"USD {elapsed * RATE:.2f}. Pilot projection USD {budget['projected_usd']:.2f}. "
              f"No-carry timeout ceiling USD {budget['ceiling_usd']:.2f}.", "",
              "Integrated-by: power-scaling (Sol worker)", ""]
    (output / "report.md").write_text("\n".join(lines))
    (output / "fitted-candidate.toml").write_text(result["defaults_overlay"])
    write_json(output / "confirmation.json", {
        "source_revision": confirm["source_revision"], "observed_profiles": profiles,
        "win_intervals": {point: {s: wilson(*p["win"]) for s, p in rows.items()}
                          for point, rows in profiles.items()},
        "measured_weighted_loss": scores, "floor_clear_counts": clears,
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--experiment", type=Path)
    parser.add_argument("--output", type=Path, default=OUTPUT / "no-carry")
    parser.add_argument("--spent-usd", type=float, help="Prior compute and overhead reserve")
    parser.add_argument("--pilot", action="store_true", help="Write the no-carry pilot manifest")
    args = parser.parse_args()
    defaults = tomllib.loads(args.defaults.read_text())
    d = defaults["director"]
    if defaults["campaign"]["carry"] or d["power_scaling"] or not d["pressure_beats"]:
        parser.error("use no-carry defaults with surges enabled and power scaling off")
    if args.pilot:
        args.output.mkdir(parents=True, exist_ok=True)
        tuning = overlay([knob.shipped for knob in KNOBS], defaults)
        jobs = jobs_for("baseline", "", 9000, 2) + jobs_for("power", tuning, 9000, 2)
        write_json(args.output / "pilot-manifest.json", {"name": "pilot", "points": {}, "jobs": jobs})
        return
    if args.experiment is None or args.spent_usd is None:
        parser.error("--experiment and --spent-usd are required for the full study")
    proof = json.loads((args.output / "determinism.json").read_bytes())
    pilot, _, _ = load_stage(args.output, "pilot")
    if not proof["bytes_equal"] or proof["source_revision"] != pilot["source_revision"]:
        raise ValueError("Complete the native–Modal pilot before dispatching")
    experiment = args.experiment.read_text()
    screen = manifest("screen", defaults, experiment)
    plan = manifest("confirm", defaults, experiment, [knob.shipped for knob in KNOBS])
    budget = cost_estimate(args.output, screen, plan)
    budget["previous_compute_and_overhead_usd"] = args.spent_usd
    write_json(args.output / "cost-estimate.json", budget)
    if args.spent_usd + budget["ceiling_usd"] > 20:
        raise ValueError("Combined timeout budget exceeds USD 20")
    print(f"Projected USD {budget['projected_usd']:.2f}. Compute ceiling USD {budget['ceiling_usd']:.2f}", flush=True)
    run_stage(args.output, screen)
    with threadpool_limits(limits=1):
        result = fit(args.output, defaults, KNOBS, overlay)
    write_json(args.output / "solution.json", result)
    run_stage(args.output, manifest("confirm", defaults, experiment, result["values"]))
    write_report(args.output, result, budget)


if __name__ == "__main__":
    main()
