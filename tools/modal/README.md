# Rat Run difficulty on Modal

Use Python 3.12 or newer. Run these commands from the repository root. The
workspace owner creates the Modal profile and completes browser authorization.

```sh
python3 -m venv .venv
.venv/bin/pip install -r tools/modal/requirements.txt
.venv/bin/modal token new
```

Set `RESULTS` to a directory for the study and `DEFAULTS` to a tuning dump from
the build being studied. Set `BINARY` to the native release executable. Commit
the Rust sources before dispatching so results can be tied to a revision.

```sh
"$BINARY" --tuning-dump "$DEFAULTS"
.venv/bin/python tools/modal/design.py pilot --defaults "$DEFAULTS" --output "$RESULTS"
.venv/bin/modal run tools/modal/app.py --manifest "$RESULTS/pilot-manifest.json" --output "$RESULTS"
.venv/bin/python tools/modal/verify.py --binary "$BINARY" --output "$RESULTS"
.venv/bin/python tools/modal/study.py --defaults "$DEFAULTS" --output "$RESULTS"
```

The pilot times two campaigns per skill. The byte comparison normalizes the
tuning path and two wall-clock statistics. The study records a pilot-based
cost estimate before screening. The timeout estimate includes one retry, an
optional second sampling round and final confirmation. The receipts track
successful subprocess time. Startup, idle time, image building and failed
attempts are outside the receipt-based estimate.

Screening uses 176 settings, eight common seeds and three skills. The study
withholds whole settings for validation and solves the per-skill response
surfaces jointly. If the solution is within 2% of a range edge, the study
expands that range and runs 33 additional settings. The fitted candidate gets
300 campaigns per skill on fresh seeds. The report and tuning overlay are
written after those campaigns finish.

Supply `--resume-screen` to the study only after the screen manifest already
exists. Completed shards have a checksummed receipt and are skipped on
retry. Use a new output directory for a new simulator revision or design.
