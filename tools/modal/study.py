"""Run bounded screening and confirm on fresh Modal seeds."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib

from threadpoolctl import threadpool_limits

from app import OUTPUT, RATE, TIMEOUT, estimate_cost
from design import KNOBS, manifest, study_anchor
from fit import fit, load_stage
from report import report


def write_json(path, data):
    path.write_text(json.dumps(data, separators=(",", ":"), allow_nan=False))


def cost_estimate(output, screen, confirm):
    pilot, _, _ = load_stage(output, "pilot")
    pilot_path = output / "raw" / "pilot"
    active_seconds = {}
    for job in pilot["jobs"]:
        receipt = json.loads((pilot_path / f"{job['id']}.receipt").read_bytes())
        active_seconds[job["skill"]] = receipt["seconds"] / job["runs"]
    planned = [*pilot["jobs"], *screen["jobs"], *confirm["jobs"]]
    return {
        "seconds_per_campaign_by_skill": active_seconds,
        "projected_usd": estimate_cost(active_seconds, planned),
        "ceiling_usd": len(planned) * 2 * TIMEOUT * RATE,
    }


def run_stage(output, spec):
    name = spec["name"]
    raw = output / "raw" / name
    if all((raw / f"{job['id']}.receipt").is_file() for job in spec["jobs"]):
        stored, _, _ = load_stage(output, name)
        if json.dumps({key: stored[key] for key in spec}, sort_keys=True) != json.dumps(spec, sort_keys=True):
            raise ValueError(f"Existing {name} receipts belong to a different manifest")
        return
    path = output / f"{name}-manifest.json"
    write_json(path, spec)
    app = Path(__file__).with_name("app.py")
    modal = Path(sys.executable).with_name("modal")
    subprocess.run(
        [str(modal), "run", str(app), "--manifest", str(path), "--output", str(output)],
        check=True,
    )


def study(output, anchor, experiment, max_compute_usd, resume_screen, baseline):
    proof = json.loads((output / "determinism.json").read_bytes())
    if not proof["bytes_equal"]:
        raise ValueError("Complete the native–Modal pilot comparison before dispatching")
    if not experiment.strip():
        raise ValueError("Provide the owner's experiment overlay")
    screen = manifest("screen", anchor, experiment=experiment)
    if resume_screen and not (output / "raw" / "screen" / "manifest.json").is_file():
        raise ValueError("The screen has no recorded manifest to resume")
    plan = manifest("confirm", anchor, {"values": [k.shipped for k in KNOBS]}, experiment)
    budget = cost_estimate(output, screen, plan)
    write_json(output / "cost-estimate.json", budget)
    print(f"Projected USD {budget['projected_usd']:.2f}, timeout ceiling USD {budget['ceiling_usd']:.2f}", flush=True)
    if budget["ceiling_usd"] > max_compute_usd:
        raise ValueError("The timeout ceiling exceeds the approved compute budget")
    run_stage(output, screen)
    with threadpool_limits(limits=1):
        result = fit(output, anchor)
    write_json(output / "solution.json", result)
    print(f"Confirming candidate with weighted loss {result['loss']:.3f}", flush=True)
    run_stage(output, manifest("confirm", anchor, result, experiment))
    report(output, baseline)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--defaults", required=True, type=Path)
    parser.add_argument("--anchor", required=True, type=Path, help="Round 3 screening's original defaults")
    parser.add_argument("--experiment", required=True, type=Path, help="Owner's comparison overlay")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--max-compute-usd", type=float, default=20.)
    parser.add_argument("--resume-screen", action="store_true")
    parser.add_argument("--baseline", type=Path, action="append", default=[],
                        help="Earlier confirmation.json, in order from round 1")
    args = parser.parse_args()
    anchor = study_anchor(tomllib.loads(args.defaults.read_text()), tomllib.loads(args.anchor.read_text()))
    study(args.output.resolve(), anchor, args.experiment.read_text(),
          args.max_compute_usd, args.resume_screen, args.baseline)


if __name__ == "__main__":
    main()
