# Rat Run difficulty on Modal

Use Python 3.12 or newer and run these commands from the repository root.
The runner uses the active Modal profile.

```sh
python3 -m venv .venv
.venv/bin/pip install -r tools/modal/requirements.txt
```

Set `RESULTS` to a directory for the study and `BINARY` to the native release
executable with the round 3 surge defaults and the power switch off. Set
`DEFAULTS` to a tuning dump from that build and `ANCHOR` to the original round
3 screening dump, which defines the absolute feel limits. Set `EXPERIMENT`
to the owner's half-HP, double-enemy-HP TOML overlay. Commit the Rust sources
before dispatching so results can be tied to a revision.

```sh
"$BINARY" --tuning-dump "$DEFAULTS"
.venv/bin/python tools/modal/design.py pilot --defaults "$DEFAULTS" --anchor "$ANCHOR" --output "$RESULTS"
.venv/bin/modal run tools/modal/app.py --manifest "$RESULTS/pilot-manifest.json" --output "$RESULTS"
.venv/bin/python tools/modal/verify.py --binary "$BINARY" --output "$RESULTS" --id power-expert-1000
.venv/bin/python tools/modal/study.py --defaults "$DEFAULTS" --anchor "$ANCHOR" --experiment "$EXPERIMENT" --output "$RESULTS"
```

The pilot times two campaigns per skill with round 3 defaults and two with
power scaling enabled on top of those defaults. The byte comparison normalizes the
tuning path and two wall-clock statistics. The study records a pilot-based
cost estimate before screening. The timeout estimate includes one retry and
final confirmation. The receipts track successful subprocess time. Startup,
idle time, image building and failed attempts are outside the receipt-based
estimate.

Screening uses 134 Latin hypercube settings and one nominal enabled setting,
with eight common seeds (4000–4007) across each of three skills. It also runs
the round 3 defaults and the owner experiment as comparison points. Fresh
campaigns start with an empty Nest on the default route. The design sweeps
power scaling and the existing balance and pressure-beat controls within
the original feel limits. The study withholds whole settings for validation
and solves the per-skill response models jointly. Fresh Nest win targets are
80–90% Expert, 50–65% Skilled, and 20–30% Casual. The other targets and all
weights carry over from round 2.

The fitted candidate, its power-off ablation, and the round 3 defaults each
get 300 campaigns per skill on fresh seeds 8000–8299. The candidate also
gets 96 campaigns per skill at mid and max Nest. The owner experiment gets
96 fresh campaigns per skill. Pass one `--baseline /path/to/confirmation.json`
per earlier round in order to include historical measured results.
The report writes the model's confirmed overlay to `fitted-candidate.toml`.

Supply `--resume-screen` to the study only after the screen manifest already
exists. Completed shards have a checksummed receipt and are skipped on
retry. The runner gives each campaign up to 7,200 simulation seconds to
reach an outcome. The fit excludes a screening setting if any of its
campaigns are still running at that cap. Confirmation requires every
campaign to finish.
Use a new output directory for a new simulator revision or design.
