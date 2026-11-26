"""Write the confirmed tuning recommendation, validation report and figures."""

import argparse
from collections import Counter
import csv
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

from app import OUTPUT, RATE, SKILLS
from design import KNOBS
from fit import BOSSES, FLOORS, FLOOR_DEATH, PHASES, WEIGHTS, WINDOW_DEATH, WIN_BANDS, load_stage, observe

COLORS = ("#17705b", "#2d6397", "#b75328")


def wilson(rate, n, z=1.959964):
    denom = 1 + z * z / n
    center = (rate + z * z / (2 * n)) / denom
    radius = z * np.sqrt(rate * (1 - rate) / n + z * z / (4 * n * n)) / denom
    return center - radius, center + radius


def percent(value):
    return "—" if value is None else f"{value:.1%}"


def number(value, decimals=1):
    return "—" if value is None else f"{value:.{decimals}f}"


def measured(profile, key):
    value, count = profile[key]
    return value if count else None


def write_plots(output, result, actual):
    fig, panels = plt.subplots(1, 3, figsize=(10, 3.5), sharex=True, sharey=True, layout="constrained")
    for ax, skill, color in zip(panels, SKILLS, COLORS, strict=True):
        held = result["heldout_wins"][skill]
        ax.plot([0, 1], [0, 1], color="#a5a9ae", linewidth=1)
        ax.scatter(held["predicted"], held["observed"], color=color, alpha=.5, s=20)
        ax.set(title=skill.title(), xlabel="Predicted win rate", xlim=(0, 1), ylim=(0, 1))
    panels[0].set_ylabel("Observed win rate")
    fig.suptitle("Held-out tuning points")
    fig.savefig(output / "holdout.png", dpi=160)
    plt.close(fig)

    fig, panels = plt.subplots(3, 4, figsize=(13, 7.8), sharex=True, sharey=True, layout="constrained")
    seconds = np.arange(10, 901, 10)
    for row, (skill, color) in enumerate(zip(SKILLS, COLORS, strict=True)):
        for col, floor in enumerate(FLOORS):
            ax = panels[row, col]
            keys = [f"hp_{col + 1}_{t}" for t in seconds]
            forecast = np.array([result["candidate"][skill][key] for key in keys], dtype=float)
            observed = np.array([measured(actual[skill], key) for key in keys], dtype=float)
            for start, end in (PHASES[1], PHASES[3]):
                ax.axvspan(start / 60, (end - 10) / 60, color="#f2e4cb", linewidth=0)
            ax.plot(seconds / 60, forecast, "--", color=color, linewidth=1.2, label="Predicted")
            ax.plot(seconds / 60, observed, color=color, linewidth=1.6, label="Fresh seeds")
            ax.set(xlim=(0, 15), ylim=(0, 1.02), xticks=[0, 3, 6, 9, 12, 15])
            if not row:
                ax.set_title(floor)
            if not col:
                ax.set_ylabel(f"{skill.title()} HP fraction")
            if row == 2:
                ax.set_xlabel("Floor minutes")
    panels[0, 3].legend(loc="lower left", frameon=False)
    fig.suptitle("Mean HP among campaigns still on each floor at the sample")
    fig.savefig(output / "health.png", dpi=160)
    plt.close(fig)

    fig, panels = plt.subplots(1, 2, figsize=(10, 3.7), layout="constrained")
    ax = panels[0]
    for i, (skill, color) in enumerate(zip(SKILLS, COLORS, strict=True)):
        value, n = actual[skill]["win"]
        low, high = wilson(value, n)
        ax.plot(WIN_BANDS[i], [i, i], linewidth=10, alpha=.16, color=color, solid_capstyle="butt")
        ax.errorbar(value, i, xerr=[[value - low], [high - value]], fmt="o", color=color, capsize=3)
        ax.scatter(result["candidate"][skill]["win"], i, marker="x", color="#333333", zorder=3)
    ax.set(yticks=range(3), yticklabels=[s.title() for s in SKILLS], xlabel="Win rate", xlim=(0, 1))
    ax.invert_yaxis()
    ax.set_title("Win bands, predictions and 95% intervals")
    ax = panels[1]
    for index, (skill, color) in enumerate(zip(SKILLS, COLORS, strict=True)):
        values = [actual[skill][key][0] for key in FLOOR_DEATH]
        ax.bar(np.arange(4) + (index - 1) * .24, values, width=.22, color=color, label=skill.title())
    ax.axhline(.40, linestyle="--", linewidth=1, color="#666666", label="Target ceiling")
    ax.set(xticks=range(4), xticklabels=FLOORS, ylabel="Share of deaths", ylim=(0, 1),
           title="Fresh-seed death distribution")
    ax.legend(frameon=False, fontsize=8)
    fig.savefig(output / "targets.png", dpi=160)
    plt.close(fig)


def table(lines, headings, rows):
    lines.extend(["", "| " + " | ".join(headings) + " |",
                  "| " + " | ".join(["---"] * len(headings)) + " |"])
    lines.extend("| " + " | ".join(map(str, row)) + " |" for row in rows)
    lines.append("")


def confirmation(lines, cohort, actual, prediction):
    lines += ["## Fresh-seed confirmation", "", "Win bands are assessed on point estimates. "
              "Wilson intervals describe uncertainty at the measured tuning."]
    rows = []
    for i, skill in enumerate(SKILLS):
        rate, n = actual[skill]["win"]
        low, high = WIN_BANDS[i]
        lower, upper = wilson(rate, n)
        rows.append((skill, f"{low:.0%}–{high:.0%}", percent(prediction[skill]["win"]),
                     f"{round(rate * n)}/{n} ({rate:.1%})", f"{lower:.1%}–{upper:.1%}",
                     "Within band" if low <= rate <= high else "Outside band"))
    table(lines, ("Skill", "Target", "Predicted", "Measured", "95% CI", "Result"), rows)
    lines += ["![Win bands and floor deaths](targets.png)", "",
              "The death percentages use losses as the denominator. Corresponding three-minute "
              "windows start on each floor's clock and pool losses across floors."]
    rows = []
    for skill in SKILLS:
        for name, keys in (("Floor", FLOOR_DEATH), ("3-minute window", WINDOW_DEATH)):
            observed = [actual[skill][k][0] for k in keys]
            idx = int(np.argmax(observed))
            label = FLOORS[idx] if name == "Floor" else f"{3 * idx}:00–{3 * (idx + 1)}:00"
            spread = all(v > 0 for v in observed) if name == "Floor" else True
            available = actual[skill][keys[idx]][1] > 0
            status = "Within target" if available and observed[idx] <= .40 and spread else "Outside target"
            rows.append((skill, name, percent(max(prediction[skill][k] for k in keys)),
                         f"{label}, {percent(observed[idx])}" if available else "—", status))
    table(lines, ("Skill", "Largest death share", "Predicted maximum", "Measured maximum", "Result"), rows)
    rows = []
    for skill in SKILLS:
        for f, floor in enumerate(FLOORS, 1):
            key = f"reach_{f}"
            rows.append((skill, floor, percent(prediction[skill][key]), percent(measured(actual[skill], key)),
                         percent(measured(actual[skill], FLOOR_DEATH[f - 1]))))
    table(lines, ("Skill", "Floor", "Predicted reach", "Measured reach", "Measured death share"), rows)
    for skill in SKILLS:
        causes = Counter(r["death"]["cause"] for r in cohort[skill] if r["death"])
        if causes:
            cause, count = causes.most_common(1)[0]
            lines += [f"- {skill.title()} deaths most often had cause {cause} "
                      f"({count}/{causes.total()}, {count / causes.total():.1%})."]
    rows, hp_rows = [], []
    for skill in SKILLS:
        for f, floor in enumerate(FLOORS, 1):
            keys = [f"first_card_{f}", f"cards_15m_{f}", f"boss_seconds_{f}"]
            cells = [f"{number(prediction[skill][k])} / {number(measured(actual[skill], k))} ({actual[skill][k][1]})" for k in keys]
            rows.append((skill, floor, *cells))
            phase_keys = [f"hp_phase_{f}_{i}" for i in range(4)]
            hp = [measured(actual[skill], k) for k in phase_keys]
            dips = [hp[0] - hp[1], hp[2] - hp[1], hp[2] - hp[3]] if all(x is not None for x in hp) else []
            cells = [f"{percent(prediction[skill][k])} / {percent(y)}" for k, y in zip(phase_keys, hp, strict=True)]
            hp_rows.append((skill, floor, *cells, "Both dips" if dips and min(dips) >= .05 else "Outside target"))
    lines += ["", "### Level-ups and boss duration", "",
              "Cells give predicted / fresh-seed mean (count). First-card means include campaigns that "
              "received a card. Counts at 15:00 include campaigns that reached 15:00. Fight durations "
              "include boss clears. Targets are 15–21 s, 13–17 cards and 40–80 s, respectively."]
    table(lines, ("Skill", "Floor", "First card (s)", "Cards by 15:00", "Boss fight (s)"), rows)
    rows = []
    for boss in BOSSES:
        key = f"clear_{boss}"
        rate, n = actual["casual"][key]
        rows.append((boss, percent(prediction["casual"][key]), f"{round(rate * n)}/{n}",
                     "Cleared" if rate > 0 else "No clear observed"))
    table(lines, ("Default route boss", "Predicted Casual clear given encounter", "Fresh Casual clears/encounters", "Result"), rows)
    lines += ["Fight durations by boss below give predicted / measured seconds among completed encounters."]
    rows = []
    for boss in BOSSES:
        key = f"boss_seconds_name_{boss}"
        cells = [f"{number(prediction[s][key])} / {number(measured(actual[s], key))} ({actual[s][key][1]})" for s in SKILLS]
        rows.append((boss, *cells))
    table(lines, ("Boss", "Expert", "Skilled", "Casual"), rows)
    lines += ["### HP tension", "",
              "Each cell gives predicted / measured HP. Phase means include campaigns observed through "
              "the whole phase. Both dips require falls of five percentage points into each shaded window "
              "and a five-point recovery between them. The plot includes all 10-second samples among survivors at each instant."]
    table(lines, ("Skill", "Floor", "1:00–3:00", "3:00–6:00", "6:00–11:00", "11:00–14:00", "Result"), hp_rows)
    lines += ["![Predicted and measured HP curves](health.png)"]
    rows = []
    for skill in SKILLS:
        rate, n = actual[skill]["close_win"]
        rows.append((skill, percent(prediction[skill]["close_win"]), f"{round(rate * n)}/{n} ({rate:.1%})",
                     "Within target" if n and rate >= .55 else "Outside target"))
    table(lines, ("Wins below 30% HP", "Predicted", "Measured", "Result"), rows)


def design_and_fit(lines, result):
    lines += ["## Design and fit", "",
              "The initial design used 175 randomized Latin hypercube points and shipped defaults. Each "
              "point used the same eight seeds (1000–1007) for each skill, for 4,224 full campaigns. "
              "Later floors inherited the actual keep choices.", "",
              f"The fit had {len(result['train'])} training settings and {len(result['holdout'])} untouched holdout settings. "
              "Four-fold validation within training chose regularization and linear or quadratic features by "
              "skill. Win, close-call and boss-clear probabilities use logistic regressions. Floor and "
              "window death shares use multinomial logistic regressions conditional on death. Floor survival "
              "follows from those shares and win probability. Weighted ridge regressions fit timing, HP, "
              "damage and card metrics by floor using the number of observed campaign records as weights.", "",
              "The holdout tests new tuning settings on common seeds. Fresh confirmation tests the "
              "candidate on independent seeds. Small late-floor cohorts can make predictions noisy."]
    rows = []
    for skill in SKILLS:
        v = result["validation"][skill]["win"]
        rows.append((skill, number(v["brier"], 3), number(v["baseline_brier"], 3), percent(v["mae"]),
                     percent(v["predicted"]), percent(v["observed"])))
    table(lines, ("Skill", "Win Brier", "Constant Brier", "Point MAE", "Mean predicted", "Observed"), rows)
    families = {
        "Floor reached": [f"reach_{f}" for f in range(2, 5)],
        "Death floors": FLOOR_DEATH, "Death windows": WINDOW_DEATH,
        "10-second HP": [f"hp_{f}_{t}" for f in range(1, 5) for t in range(10, 901, 10)],
        "Winning close calls": ["close_win"],
        "Boss duration (s)": [f"boss_seconds_{f}" for f in range(1, 5)],
        "First card (s)": [f"first_card_{f}" for f in range(1, 5)],
        "Cards by 15:00": [f"cards_15m_{f}" for f in range(1, 5)],
    }
    rows = []
    for label, keys in families.items():
        row = [label]
        for skill in SKILLS:
            available = result["validation"][skill]
            stats = [available[k] for k in keys if k in available]
            error = float(np.average([v["mae"] for v in stats], weights=[v["runs"] for v in stats])) if stats else None
            row.append(number(error, 3))
        rows.append(row)
    table(lines, ("Holdout cohort-weighted MAE", "Expert", "Skilled", "Casual"), rows)
    lines += ["![Held-out win calibration](holdout.png)", "", "## Tuning and sensitivity", "",
              "The optimizer searched within sampled bounds, with integer XP steps. Objective weights were "
              + ", ".join(f"{k.replace('_', ' ')} {w}%" for k, w in WEIGHTS.items()) + ". "
              "Death targets use a 40% concentration ceiling and a 5% minimum share per floor. "
              "Boss timing uses the floor estimate when fewer than twelve clears support a named-boss estimate. "
              "The Casual clear target is 5% per encounter, and the close-call target is 55% of wins.", "",
              f"The weighted objective loss is {result['loss']:.3f}. "
              + " ".join(f"{k.replace('_', ' ').capitalize()} contributed {v:.3f}." for k, v in result["loss_by_target"].items()),
              "", "Candidate knobs within 2% of a final search edge "
              + (", ".join(result["edges"]) if result["edges"] else "none") + "."]
    table(lines, ("Knob", "Initial range", "Final range", "Candidate"), [
        (k.name, f"{k.low:g}–{k.high:g}×", f"{lo:g}–{hi:g}×", f"{v:.3f}×")
        for k, (lo, hi), v in zip(KNOBS, result["bounds"], result["values"], strict=True)
    ])
    lines += ["Sensitivity is the average modeled change per +0.10× multiplier over sampled training "
              "settings. Rates use percentage points and durations use seconds. The accompanying CSV includes every modeled outcome."]
    for skill in SKILLS:
        rows = []
        for knob, effects in result["sensitivity"][skill].items():
            cell = lambda key, scale=1: number(None if effects[key] is None else scale * effects[key], 2)
            rows.append((knob, cell("win", 100), cell("reach_4", 100), cell("close_win", 100),
                         cell("first_card_1"), cell("boss_seconds_4"), cell("cards_15m_1")))
        lines += ["", f"### {skill.title()}"]
        table(lines, ("Knob", "Win pp", "Reach Flatbed pp", "Winning close call pp",
                      "Sewer first card s", "Flatbed boss s", "Sewer cards"), rows)


def report(output):
    result = json.loads((output / "solution.json").read_bytes())
    budget = json.loads((output / "cost-estimate.json").read_bytes())
    proof = json.loads((output / "determinism.json").read_bytes())
    pilot, _, pilot_seconds = load_stage(output, "pilot")
    confirm, campaigns, confirm_seconds = load_stage(output, "confirm")
    if confirm["source_revision"] != result["revision"] or not proof["bytes_equal"]:
        raise ValueError("confirmation must use the fitted simulation revision and a matched pilot")
    if confirm["points"]["candidate"] != result["values"]:
        raise ValueError("confirmation used a different candidate")
    cohort = {s: [r for r in campaigns["candidate"] if r["skill"] == s] for s in SKILLS}
    for s in SKILLS:
        if sorted(r["seed"] for r in cohort[s]) != list(range(5000, 5300)):
            raise ValueError(f"confirmation cohort mismatch for {s}")
    actual = {s: observe(cohort[s]) for s in SKILLS}
    lines = ["# Rat Run difficulty model", "",
             f"Screening and confirmation used the v0.4.1 simulation at {result['revision']}. "
             "Campaigns followed Sewer, Kitchen, Lab and Flatbed with a fresh Nest. "
             "Confirmation used exact seeds 5000–5299 for every skill.", ""]
    confirmation(lines, cohort, actual, result["candidate"])
    design_and_fit(lines, result)
    seconds = pilot_seconds + confirm_seconds + result["screen_seconds"]
    lines += ["## Determinism and spend", "",
              f"Pilot {pilot['source_revision']} matched macOS and Modal Linux byte for byte after "
              f"removing wall time, throughput and the tuning file path. Canonical SHA-256 was {proof['modal_sha256']}.", "",
              f"Pilot timings projected USD {budget['projected_usd']:.2f} at list price before fan-out. "
              f"The compute timeout ceiling with one retry was USD {budget['ceiling_usd']:.2f}. "
              f"Successful shards consumed {seconds:,.0f} container wall-seconds, equivalent to USD {seconds * RATE:.2f} "
              "at the configured CPU and memory caps. The estimate excludes image building, startup, "
              "failed attempts and workspace credits. [Modal list pricing](https://modal.com/pricing).", "",
              "Raw JSON and checksummed receipts are in raw/. Fit scores and training cohort sizes are in solution.json.",
              "", "Integrated-by: modal-model (Sol worker)", ""]
    with (output / "sensitivity.csv").open("w", newline="") as dest:
        writer = csv.writer(dest)
        writer.writerow(["skill", "knob", "outcome", "effect_per_point_one_multiplier"])
        for skill, knobs in result["sensitivity"].items():
            for knob, metrics in knobs.items():
                writer.writerows((skill, knob, key, value) for key, value in metrics.items())
    write_plots(output, result, actual)
    (output / "confirmation.json").write_text(json.dumps({
        "source_revision": confirm["source_revision"],
        "predicted": result["candidate"],
        "observed_mean_and_count": actual,
        "skill_win_wilson95": {s: wilson(*actual[s]["win"]) for s in SKILLS},
    }, indent=2))
    (output / "report.md").write_text("\n".join(lines))
    summary = ", ".join(f"{s} {actual[s]['win'][0]:.1%}" for s in SKILLS)
    (output / "recommended.toml").write_text(
        "# Rat Run v0.4.1 difficulty model candidate.\n"
        f"# Fresh seeds 5000–5299, measured campaign wins {summary}.\n"
        "# See report.md for target conformance, uncertainty and predictions.\n\n"
        + result["defaults_overlay"],
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    report(args.output)


if __name__ == "__main__":
    main()
