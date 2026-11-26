"""Compare a Modal shard with an independent run of the native release binary."""

import argparse
import hashlib
import json
from pathlib import Path

from app import OUTPUT, atomic_write, read_manifest, run_command


def canonical(raw):
    report = json.loads(raw)
    del report["wall_seconds"]
    del report["sim_per_wall_second_per_thread"]
    report["tuning"].pop("path", None)
    return json.dumps(report, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path)
    source.add_argument("--native-report", type=Path, help="Previously run native shard JSON")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--id", default="defaults-expert-1000")
    args = parser.parse_args()
    pilot = args.output / "raw" / "pilot"
    manifest = read_manifest(pilot / "manifest.json")
    job = next(job for job in manifest["jobs"] if job["id"] == args.id)
    if args.binary:
        _, raw, elapsed = run_command(args.binary.resolve(), job)
    else:
        raw = args.native_report.read_bytes()
        elapsed = json.loads(raw)["wall_seconds"]
    local = canonical(raw)
    remote = canonical((pilot / f"{args.id}.json").read_bytes())
    atomic_write(pilot / f"{args.id}.native.json", raw)
    record = {
        "source_revision": manifest["source_revision"],
        "job_id": args.id,
        "bytes_equal": local == remote,
        "local_sha256": hashlib.sha256(local).hexdigest(),
        "modal_sha256": hashlib.sha256(remote).hexdigest(),
        "native_seconds": elapsed,
    }
    atomic_write(args.output / "determinism.json", json.dumps(record, indent=2).encode())
    if local != remote:
        print("Differing report fields:", [
            key for key in json.loads(local) if json.loads(local)[key] != json.loads(remote)[key]
        ])
        raise SystemExit(1)
    print(f"Simulation bytes match, SHA-256 {record['local_sha256']}")


if __name__ == "__main__":
    main()
