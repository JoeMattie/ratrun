# Rat Run difficulty on Modal

Use Python 3.12 or newer and run these commands from the repository root.
The runner uses the active Modal profile.

```sh
python3 -m venv .venv
.venv/bin/pip install -r tools/modal/requirements.txt
```

Set `RESULTS` to a directory for the study and `DEFAULTS` to a tuning dump from
the build being studied. Set `BINARY` to the native release executable. Commit
the Rust sources before dispatching so results can be tied to a revision.

```sh
"$BINARY" --tuning-dump "$DEFAULTS"
.venv/bin/python tools/modal/design.py pilot --defaults "$DEFAULTS" --output "$RESULTS"
.venv/bin/modal run tools/modal/app.py --manifest "$RESULTS/pilot-manifest.json" --output "$RESULTS"
.venv/bin/python tools/modal/verify.py --binary "$BINARY" --output "$RESULTS" --id beats-expert-1000
.venv/bin/python tools/modal/study.py --defaults "$DEFAULTS" --output "$RESULTS"
```

The pilot times two campaigns per skill with shipped defaults and two with
pressure beats enabled. The byte comparison normalizes the
tuning path and two wall-clock statistics. The study records a pilot-based
cost estimate before screening. The timeout estimate includes one retry and
final confirmation. The receipts track successful subprocess time. Startup,
idle time, image building and failed attempts are outside the receipt-based
estimate.

Screening uses shipped defaults and 149 Latin hypercube settings, with eight
common seeds (2000–2007) across each of three skills. Campaigns follow the
default route with a fresh Nest. The screening varies the pressure windows,
live budget, cadence, contact damage, recovery healing and Sewer opening grace
alongside the existing balance controls. The study withholds whole settings
for validation and solves the per-skill response models jointly. The feel
targets and their weights carry over from round 2. The report quantifies which
modeled targets move when each near-edge setting moves inward. The fitted
candidate gets 300 campaigns per skill on fresh seeds 7000–7299. Pass
`--baseline /path/to/round1/confirmation.json` and
`--baseline /path/to/round2/confirmation.json` in order to include both
earlier studies' measured results.

Supply `--resume-screen` to the study only after the screen manifest already
exists. Completed shards have a checksummed receipt and are skipped on
retry. The runner gives each campaign up to 7,200 simulation seconds to
reach an outcome. The fit excludes a screening setting if any of its
campaigns are still running at that cap. Confirmation requires every
campaign to finish.
Use a new output directory for a new simulator revision or design.
