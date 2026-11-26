"""Bounded CPU-only Modal runner for Rat Run campaign shards."""

import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

import modal

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = Path("/Users/joe/dev/ratrun/.review/model")
TIMEOUT = 1_500
MAX_SHARDS = 700
CPU = 0.5
MEMORY_MIB = 256
CPU_PRICE = 0.0000131
MEMORY_PRICE = 0.00000222
RATE = CPU * CPU_PRICE + MEMORY_MIB / 1024 * MEMORY_PRICE
SKILLS = ("expert", "skilled", "casual")

image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("build-essential", "ca-certificates", "curl", "pkg-config", "libasound2-dev", "libudev-dev")
    .run_commands(
        "curl --proto '=https' --tlsv1.2 -fsS https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.100.0",
    )
    .add_local_file(ROOT / "Cargo.toml", "/build/Cargo.toml", copy=True)
    .add_local_file(ROOT / "Cargo.lock", "/build/Cargo.lock", copy=True)
    .add_local_file(ROOT / "rust-toolchain.toml", "/build/rust-toolchain.toml", copy=True)
    .add_local_file(ROOT / "assets/rat.png", "/build/assets/rat.png", copy=True)
    .add_local_dir(ROOT / "src", "/build/src", copy=True)
    .run_commands(
        "cd /build && /root/.cargo/bin/cargo build --release --locked -j 4",
        "install /build/target/release/ratrun /usr/local/bin/ratrun",
    )
)
app = modal.App("ratrun-difficulty")


def run_command(binary, job):
    with tempfile.TemporaryDirectory() as scratch:
        tuning = Path(scratch) / "tuning.toml"
        tuning.write_text(job["tuning"])
        command = [
            str(binary), "--bot-batch", str(job["runs"]), "--seed", str(job["seed"]),
            "--seed-exact", "--skill", job["skill"], "--threads", "1",
            "--tuning", str(tuning), "--json",
        ]
        if job.get("practice"):
            command.extend(["--practice", "--route", job["route"]])
        started = time.monotonic()
        process = subprocess.run(command, capture_output=True, timeout=TIMEOUT - 10)
        if process.returncode:
            raise RuntimeError(process.stderr.decode(errors="replace")[-2000:])
        raw = process.stdout
        rows = json.loads(raw)["run_summaries"]
        if [row["seed"] for row in rows] != list(range(job["seed"], job["seed"] + job["runs"])):
            raise ValueError("unexpected simulation seeds")
        if any(row["skill"] != job["skill"] for row in rows):
            raise ValueError("unexpected simulation skill")
        if any(not row["won"] and row["death"] is None for row in rows):
            raise ValueError("at least one campaign hit the simulation time cap")
        return job["id"], raw, time.monotonic() - started


@app.function(
    image=image, cpu=(CPU, CPU), memory=(MEMORY_MIB, MEMORY_MIB), timeout=TIMEOUT,
    max_containers=80, scaledown_window=5, block_network=True,
)
def shard(job):
    return run_command("/usr/local/bin/ratrun", job)


def atomic_write(path, data):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)


def read_manifest(path):
    manifest = json.loads(Path(path).read_bytes())
    jobs = manifest["jobs"]
    ids = {job["id"] for job in jobs}
    if not 0 < len(jobs) <= MAX_SHARDS or len(ids) != len(jobs):
        raise ValueError("manifest has duplicate IDs or exceeds the shard limit")
    if not re.fullmatch(r"[a-z0-9-]+", manifest["name"]):
        raise ValueError("invalid manifest name")
    for job in jobs:
        if not re.fullmatch(r"[a-z0-9-]+", job["id"]):
            raise ValueError("invalid job ID")
        if job["skill"] not in SKILLS or not 1 <= job["runs"] <= 8 or job["seed"] < 0:
            raise ValueError(f"invalid shard inputs for {job['id']}")
    return manifest


def estimate_cost(pilot_seconds, jobs):
    """Estimate compute from observed shard seconds by skill."""
    return sum(pilot_seconds[j["skill"]] * j["runs"] for j in jobs) * RATE


@app.local_entrypoint()
def main(manifest: str, output: str = str(OUTPUT)):
    spec = read_manifest(manifest)
    directory = Path(output) / "raw" / spec["name"]
    directory.mkdir(parents=True, exist_ok=True)
    recorded_manifest = directory / "manifest.json"
    source_revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True,
    ).strip()
    if subprocess.check_output(["git", "status", "--porcelain", "--", "src", "Cargo.toml", "Cargo.lock",
                                "rust-toolchain.toml", "assets", "tools/modal"], cwd=ROOT):
        raise ValueError("commit the build inputs before dispatching")
    record = json.dumps({**spec, "source_revision": source_revision}, sort_keys=True).encode()
    if recorded_manifest.exists() and recorded_manifest.read_bytes() != record:
        raise ValueError("existing results were produced by a different manifest or revision")
    atomic_write(recorded_manifest, record)

    pending = [j for j in spec["jobs"] if not (directory / f"{j['id']}.receipt").exists()]
    print(f"{len(pending)} shards pending, timeout-priced ceiling ${len(pending) * 2 * TIMEOUT * RATE:.2f}")
    for attempt in range(2):
        if not pending:
            break
        failed = []
        for job, result in zip(pending, shard.map(pending, return_exceptions=True), strict=True):
            if isinstance(result, Exception):
                print(f"{job['id']} attempt {attempt + 1}: {result}")
                failed.append(job)
                continue
            key, raw, elapsed = result
            if key != job["id"]:
                raise RuntimeError(f"expected shard {job['id']}, received {key}")
            atomic_write(directory / f"{key}.json", raw)
            receipt = {
                "sha256": hashlib.sha256(raw).hexdigest(),
                "seconds": elapsed, "attempt": attempt + 1,
            }
            atomic_write(directory / f"{key}.receipt", json.dumps(receipt).encode())
        pending = failed
    if pending:
        raise RuntimeError(f"{len(pending)} incomplete shards in {directory}")
