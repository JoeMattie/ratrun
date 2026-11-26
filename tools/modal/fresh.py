"""Compare a fixed, owner-calibrated no-carry profile with paired controls."""

import argparse
from pathlib import Path

from app import OUTPUT, RATE, SIM_SECONDS, SKILLS, TIMEOUT
from design import jobs_for
from fit import load_stage, observe
from report import confirmed_cohorts, profile_comparison, wilson
from study import run_stage, write_json

SEED = 1000
RUNS = 24
SIZES = dict.fromkeys(("candidate", "baseline", "power-off", "half-hp-double-enemy",
                       "candidate-mid", "candidate-max"), RUNS)


def manifest(tuning, experiment):
    if tuning.count("power_scaling = true") != 1:
        raise ValueError("Expected one enabled power switch in the candidate")
    profiles = {
        "candidate": tuning,
        "baseline": "",
        "power-off": tuning.replace("power_scaling = true", "power_scaling = false"),
        "half-hp-double-enemy": experiment,
        "candidate-mid": tuning,
        "candidate-max": tuning,
    }
    meta = {"candidate-mid": "mid", "candidate-max": "max"}
    jobs = [job for point, config in profiles.items()
            for job in jobs_for(point, config, SEED, SIZES[point], meta.get(point, "none"))]
    for job in jobs:
        job["max_sim_seconds"] = 2 * SIM_SECONDS
    return {"name": "targeted", "points": profiles, "jobs": jobs}


def write_report(output, ceiling):
    spec, campaigns, seconds = load_stage(output, "targeted")
    cohorts = confirmed_cohorts(campaigns, SIZES, SEED)
    profiles = {point: {skill: observe(runs) for skill, runs in cohort.items()} for point, cohort in cohorts.items()}
    lines = ["# Rat Run no-carry targeted comparison", "",
             f"Revision {spec['source_revision']}. Each profile used {RUNS} campaigns per skill on "
             f"the same seeds {SEED}–{SEED + RUNS - 1}.", "",
             "The candidate held the owner's half-HP start and 1.10–1.41× the horde HP of the half-HP experiment. "
             "Later horde and boss pools and surge strengths were reduced from shipped values. "
             "The six original target families are included for comparison. The small cohorts measure the fixed "
             "profiles without fitting or solving another model.", ""]
    scores, clears = profile_comparison(lines, cohorts, profiles, baseline="baseline")
    lines += ["## Compute", "",
              f"Successful receipts used {seconds:,.0f} container-seconds, about USD {seconds * RATE:.2f}. "
              f"The two-attempt ceiling was USD {ceiling:.2f}.", "",
              "Integrated-by: power-scaling (Sol worker)", ""]
    (output / "targeted-report.md").write_text("\n".join(lines))
    write_json(output / "targeted-summary.json", {
        "source_revision": spec["source_revision"], "observed_profiles": profiles,
        "win_intervals": {point: {skill: wilson(*by_skill[skill]["win"]) for skill in SKILLS}
                          for point, by_skill in profiles.items()},
        "measured_weighted_loss": scores, "floor_clear_counts": clears, "compute_seconds": seconds,
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--experiment", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=OUTPUT / "no-carry")
    args = parser.parse_args()
    spec = manifest(args.candidate.read_text(), args.experiment.read_text())
    ceiling = len(spec["jobs"]) * 2 * TIMEOUT * RATE
    print(f"{len(spec['jobs'])} shards. Two-attempt compute ceiling USD {ceiling:.2f}", flush=True)
    run_stage(args.output, spec)
    write_report(args.output, ceiling)


if __name__ == "__main__":
    main()
