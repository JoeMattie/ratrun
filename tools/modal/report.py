"""Write the confirmed tuning recommendation, validation report and figures."""

import argparse
from collections import Counter
import csv
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.patches import Patch
import numpy as np

from app import OUTPUT, RATE, SKILLS
from design import CONFIRM_RUNS, CONFIRM_SEED, EXPERIMENT_RUNS, KNOBS, META_RUNS, SCREEN_POINTS, SCREEN_SEED
from fit import (
    BOSSES, FLOORS, FLOOR_DEATH, PHASES, WEIGHTS, WINDOW_DEATH, WIN_BANDS,
    load_stage, objective, observe, sensitivity_step,
)

COLORS = ("#17705b", "#2d6397", "#b75328")
PRIOR_STYLES = (("#8f9399", ":"), ("#353a40", "-."), ("#775c8b", "--"))


def wilson(rate, n, z=1.959964):
    denom = 1 + z * z / n
    center = (rate + z * z / (2 * n)) / denom
    radius = z * np.sqrt(rate * (1 - rate) / n + z * z / (4 * n * n)) / denom
    return np.clip(center - radius, 0., rate), np.clip(center + radius, rate, 1.)


def percent(value):
    return "—" if value is None else f"{value:.1%}"


def number(value, decimals=1):
    return "—" if value is None else f"{value:.{decimals}f}"


def measured(profile, key):
    value, count = profile[key]
    return value if count else None


def write_plots(output, result, actual, previous):
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
            for (round_no, profile), (ink, stroke) in zip(previous, PRIOR_STYLES):
                prior = [measured(profile[skill], key) for key in keys]
                ax.plot(seconds / 60, prior, stroke, color=ink, linewidth=1.0, label=f"Round {round_no} measured")
            ax.plot(seconds / 60, forecast, "--", color=color, linewidth=1.2, label="Round 4 predicted")
            ax.plot(seconds / 60, observed, color=color, linewidth=1.6, label="Round 4 measured")
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
        for j, ((_, profile), (ink, _)) in enumerate(zip(previous, PRIOR_STYLES)):
            prior, prior_n = profile[skill]["win"]
            prior_low, prior_high = wilson(prior, prior_n)
            ax.errorbar(prior, i + (j + 1) * .10, xerr=[[prior - prior_low], [prior_high - prior]], fmt="o",
                        color=ink, markerfacecolor="white", capsize=3)
    ax.set(yticks=range(3), yticklabels=[s.title() for s in SKILLS], xlabel="Win rate", xlim=(0, 1))
    ax.invert_yaxis()
    ax.set_title("Fresh-seed win rates")
    handles = [
        Patch(facecolor="#dedede", label="Target band"),
        Line2D([], [], color="#333333", marker="x", linestyle="none", label="Model prediction"),
        Line2D([], [], color="#333333", marker="o", linestyle="none", label="Round 4, 95% CI"),
    ]
    for (round_no, _), (ink, _) in zip(previous, PRIOR_STYLES):
        handles.append(Line2D([], [], color=ink, marker="o", markerfacecolor="white",
                              linestyle="none", label=f"Round {round_no}, 95% CI"))
    ax.legend(handles=handles, loc="upper left", frameon=False, fontsize=8)
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


def confirmation(lines, cohort, actual, prediction, previous):
    lines += ["## Fresh-seed confirmation", "", "Win bands apply to point estimates. "
              "Intervals are 95% Wilson intervals."]

    def comparison(skill, key, formatter):
        cells = [formatter(profile[skill][key]) for _, profile in previous]
        return " → ".join([*cells, formatter(actual[skill][key])])

    def record(pair):
        rate, n = pair
        return f"{round(rate * n)}/{n} ({rate:.1%})" if n else "—"

    def win_record(pair):
        rate, n = pair
        low, high = wilson(rate, n)
        return f"{record(pair)}, CI {low:.1%}–{high:.1%}"

    def share(pair):
        rate, n = pair
        return percent(rate if n else None)

    def mean_with_n(pair):
        mean, n = pair
        return f"{number(mean)} ({n})" if n else "—"

    def peak(skill, values, keys, window):
        profile = values[skill]
        key = max(keys, key=lambda k: profile[k][0])
        if not profile[key][1]:
            return "—"
        idx = keys.index(key)
        if not window:
            label = FLOORS[idx]
        elif idx == len(keys) - 1:
            label = f"{3 * idx}:00+"
        else:
            label = f"{3 * idx}:00–{3 * (idx + 1)}:00"
        return f"{label}, {share(profile[key])}"

    change = " → ".join([*(f"Round {round_no}" for round_no, _ in previous), "Round 4"])
    rows = []
    for i, skill in enumerate(SKILLS):
        low, high = WIN_BANDS[i]
        rows.append((skill, f"{low:.0%}–{high:.0%}", percent(prediction[skill]["win"]),
                     comparison(skill, "win", win_record),
                     "Within band" if low <= actual[skill]["win"][0] <= high else "Outside band"))
    table(lines, ("Skill", "Target", "Round 4 predicted", change, "Round 4 result"), rows)
    lines += ["![Win bands and floor deaths](targets.png)", "",
              "Floor and window shares use losses as the denominator. Each three-minute window "
              "starts on the floor clock and pools losses across floors."]
    table(lines, ("Skill", "Floor", f"Reach ({change})", f"Death share ({change})"), [
        (skill, floor, comparison(skill, f"reach_{i + 1}", share),
         comparison(skill, FLOOR_DEATH[i], share))
        for skill in SKILLS for i, floor in enumerate(FLOORS)
    ])
    rows = []
    for skill in SKILLS:
        for name, keys, window in (("Floor", FLOOR_DEATH, False), ("3-minute window", WINDOW_DEATH, True)):
            values = [peak(skill, profile, keys, window) for _, profile in previous]
            values.append(peak(skill, actual, keys, window))
            rates = [actual[skill][k][0] for k in keys]
            available = actual[skill][keys[0]][1] > 0
            within_band = available and max(rates) <= .40 and (window or min(rates) >= .05)
            rows.append((skill, name, " → ".join(values), "Within target" if within_band else "Outside target"))
    table(lines, ("Skill", "Peak death share", change, "Round 4 result"), rows)
    for skill in SKILLS:
        causes = Counter(r["death"]["cause"] for r in cohort[skill] if r["death"])
        if causes:
            cause, count = causes.most_common(1)[0]
            lines += [f"- {skill.title()} deaths most often had cause {cause} "
                      f"({count}/{causes.total()}, {count / causes.total():.1%})."]
    lines += ["", "### Level-ups and boss duration", "",
              f"Each cell shows {change.lower()} means and sample counts. First-card means include "
              "campaigns that received a card. Card counts include campaigns that reached 15:00. "
              "Fight durations include boss clears. Targets are 15–21 s, 13–17 cards and 40–80 s."]
    table(lines, ("Skill", "Floor", "First card (s)", "Cards by 15:00", "Boss fight (s)"), [
        (skill, floor, *(comparison(skill, f"{key}_{f}", mean_with_n)
                         for key in ("first_card", "cards_15m", "boss_seconds")))
        for skill in SKILLS for f, floor in enumerate(FLOORS, 1)
    ])
    table(lines, ("Boss", "Expert fight (s)", "Skilled fight (s)", "Casual fight (s)", "Casual clears/encounters"), [
        (boss, *(comparison(skill, f"boss_seconds_name_{boss}", mean_with_n) for skill in SKILLS),
         comparison("casual", f"clear_{boss}", record)) for boss in BOSSES
    ])
    lines += ["### HP tension", "",
              "Phase means include campaigns observed through the entire phase. Both dips require a drop "
              "of at least five percentage points into each shaded window and a recovery of at least five "
              "points between them. The plot uses 10-second means among rats still alive at each instant.",
              "", f"Entries show {change.lower()}."]
    rows = []
    for skill in SKILLS:
        for f, floor in enumerate(FLOORS, 1):
            keys = [f"hp_phase_{f}_{i}" for i in range(4)]
            hp = [measured(actual[skill], key) for key in keys]
            dips = [hp[0] - hp[1], hp[2] - hp[1], hp[2] - hp[3]] if all(x is not None for x in hp) else []
            rows.append((skill, floor, *(comparison(skill, k, share) for k in keys),
                         "Both dips" if dips and min(dips) >= .05 else "Outside target"))
    table(lines, ("Skill", "Floor", "1:00–3:00", "3:00–6:00", "6:00–11:00", "11:00–14:00", "Round 4 result"), rows)
    lines += ["![Predicted and measured HP curves](health.png)"]
    table(lines, ("Skill", "Win below 30% HP, predicted", f"Win below 30% HP ({change})", "Round 4 result"), [
        (skill, percent(prediction[skill]["close_win"]), comparison(skill, "close_win", record),
         "Within target" if actual[skill]["close_win"][1] and actual[skill]["close_win"][0] >= .55 else "Outside target")
        for skill in SKILLS
    ])


def design_and_fit(lines, result):
    lines += ["## Design and fit", "",
              f"The design used {SCREEN_POINTS - 1} randomized Latin hypercube settings optimized for "
              f"low discrepancy, plus one nominal enabled setting. Each point used the same eight seeds ({SCREEN_SEED}–{SCREEN_SEED + 7}) "
              f"for each skill, for {SCREEN_POINTS * 24:,} fit campaigns. Round 3 and the experiment were independent controls. "
              "Later floors inherited the actual keep choices.", "",
              f"The fit had {len(result['train'])} training settings and {len(result['holdout'])} untouched holdout settings. "
              "Four-fold validation within training chose regularization and linear or quadratic features by "
              "skill. Win, close-call and boss-clear probabilities use logistic regressions. Floor and "
              "window death shares use multinomial logistic regressions conditional on death. Floor survival "
              "follows from those shares and win probability. Weighted ridge regressions fit timing, HP, "
              "damage and card metrics by floor using the number of observed campaign records as weights.", "",
              "The holdout tests new tuning settings on common seeds. Fresh confirmation tests the "
              "candidate on independent seeds. Small late-floor cohorts can make predictions noisy."]
    if result["excluded"]:
        lines += ["", "The fit excluded every setting with a campaign still running at the simulation cap. "
                  "Those runs are recorded in the raw shards and do not count as deaths."]
        table(lines, ("Setting", "Skill", "Seed", "Floor", "Boss", "Simulated seconds"), [
            (point, row["skill"], row["seed"], row["floor"], row["boss"], number(row["seconds"], 0))
            for point, rows in result["excluded"].items() for row in rows
        ])
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
              "The optimizer kept all feel bounds fixed. Objective weights were "
              + ", ".join(f"{k.replace('_', ' ')} {w}%" for k, w in WEIGHTS.items()) + ". "
              "Death targets use a 40% concentration ceiling and a 5% minimum share per floor. "
              "Boss timing uses the floor estimate when fewer than twelve clears support a named-boss estimate. "
              "The Casual clear target is 5% per encounter, and the close-call target is 55% of wins.", "",
              f"The weighted objective loss is {result['loss']:.3f}. "
              + " ".join(f"{k.replace('_', ' ').capitalize()} contributed {v:.3f}." for k, v in result["loss_by_target"].items()),
              ""]
    table(lines, ("Knob", "Fixed range", "Candidate"), [
        (k.name, f"{lo:g}–{hi:g}{k.unit}", f"{v:.3f}{k.unit}")
        for k, (lo, hi), v in zip(KNOBS, result["bounds"], result["values"], strict=True)
    ])
    edges = result["edges"]
    if edges:
        lines += ["The candidate was within 2% of the range edge for the knobs below. Each row "
                  "moves that knob to 10% inside the range with the other settings fixed. Positive "
                  "changes worsen the target and show what the edge buys. Negative changes show "
                  "the target the candidate gives up by staying at the edge."]
        labels = [key.replace("_", " ").capitalize() for key in WEIGHTS]
        table(lines, ("Knob", "Edge", "Inward value", *labels), [
            (knob, edge["side"], number(edge["interior"], 3),
             *(f"{edge['loss_delta'][key]:+.3f}" for key in WEIGHTS))
            for knob, edge in edges.items()
        ])
    lines += ["Sensitivity is the average modeled change for a +0.10 multiplier, +0.10 s of "
              "immunity, +10 s of phase timing, or +10 starting HP. Rates use percentage points, and durations use seconds. "
              "The CSV includes every modeled outcome."]
    for skill in SKILLS:
        rows = []
        for knob, effects in result["sensitivity"][skill].items():
            cell = lambda key, scale=1: number(None if effects[key] is None else scale * effects[key], 2)
            rows.append((knob, cell("win", 100), cell("reach_4", 100), cell("close_win", 100),
                         cell("first_card_1"), cell("boss_seconds_4"), cell("cards_15m_1")))
        lines += ["", f"### {skill.title()}"]
        table(lines, ("Knob", "Win pp", "Reach Flatbed pp", "Winning close call pp",
                      "Sewer first card s", "Flatbed boss s", "Sewer cards"), rows)


class MeasuredProfile:
    """Expose campaign observations through the model's objective interface."""

    def __init__(self, observed):
        self.observed = observed
        self.support = {key: n for key, (_, n) in observed.items()}

    def predict(self, _values, *, curves):
        return {key: np.array([value if n or key.startswith("clear_") else np.nan])
                for key, (value, n) in self.observed.items()}


def confirmed_cohorts(campaigns):
    sizes = {"candidate": CONFIRM_RUNS, "round3": CONFIRM_RUNS, "power-off": CONFIRM_RUNS,
             "half-hp-double-enemy": EXPERIMENT_RUNS, "candidate-mid": META_RUNS, "candidate-max": META_RUNS}
    cohorts = {}
    for point, size in sizes.items():
        cohorts[point] = {}
        for skill in SKILLS:
            runs = sorted((r for r in campaigns[point] if r["skill"] == skill), key=lambda r: r["seed"])
            if [r["seed"] for r in runs] != list(range(CONFIRM_SEED, CONFIRM_SEED + size)):
                raise ValueError(f"confirmation cohort mismatch for {point}/{skill}")
            cohorts[point][skill] = runs
    return cohorts


def floor_clears(cohort):
    counts = [[0, 0] for _ in FLOORS]
    for run in cohort:
        for floor in run["levels"]:
            count = counts[floor["level"]]
            count[0] += floor["cleared"]
            count[1] += 1
    return counts


def profile_comparison(lines, cohorts, profiles):
    lines += ["## Same-seed controls and Nest ladder", "",
              "The half-HP experiment, mid Nest, and max Nest use the first 96 seeds of the fresh cohort. "
              "Power-off keeps the candidate's other tuning settings. Deaths through 5:00 use the Sewer floor clock."]
    rows = []
    for point, cohort in cohorts.items():
        for skill in SKILLS:
            runs = cohort[skill]
            profile = profiles[point][skill]
            win, count = profile["win"]
            lo, hi = wilson(win, count)
            first = [r["levels"][0]["second_weapon_at"] for r in runs
                     if r["levels"][0]["second_weapon_at"] is not None]
            first_card_weapon = sum(
                bool(floor["level_up_times"]) and floor["level_up_times"][0] == floor["second_weapon_at"]
                for floor in (r["levels"][0] for r in runs)
            )
            early = [r["death"]["at"] for r in runs if r["death"] and r["death"]["level"] == 0]
            card = measured(profile, "first_card_1")
            rows.append((point, skill, f"{round(win * count)}/{count} ({win:.1%})",
                         f"{lo:.1%}–{hi:.1%}", number(card),
                         f"{number(float(np.median(first)))} ({len(first)}/{count})" if first else "—",
                         f"{first_card_weapon}/{count}",
                         f"{sum(t < 180 for t in early)}/{sum(180 <= t < 300 for t in early)} / {count} runs"))
    table(lines, ("Profile", "Skill", "Wins", "95% CI", "Mean first card (s)",
                  "Median second weapon (s)", "Second weapon on first card", "Sewer deaths before 3:00 / 3:00–5:00"), rows)
    lines += ["Floor clear rates are conditional on reaching that floor."]
    clears = {point: {skill: floor_clears(runs) for skill, runs in cohort.items()}
              for point, cohort in cohorts.items()}
    table(lines, ("Profile", "Skill", *FLOORS), [
        (point, skill, *(f"{wins}/{reached} ({wins / reached:.1%})" if reached else "—"
                         for wins, reached in counts))
        for point, by_skill in clears.items() for skill, counts in by_skill.items()
    ])
    rows = []
    for point, control in (("candidate", "round3"), ("candidate", "power-off"),
                           ("half-hp-double-enemy", "round3"),
                           ("candidate-mid", "candidate"), ("candidate-max", "candidate")):
        for skill in SKILLS:
            cohort = cohorts[point][skill]
            paired = list(zip(cohort, cohorts[control][skill][:len(cohort)], strict=True))
            gains = sum(c["won"] and not b["won"] for c, b in paired)
            losses = sum(b["won"] and not c["won"] for c, b in paired)
            rows.append((point, control, skill, len(paired), gains, losses,
                         f"{(gains - losses) * 100 / len(paired):+.1f}"))
    table(lines, ("Profile", "Control", "Skill", "Pairs", "Profile-only wins", "Control-only wins", "Win change (pp)"), rows)
    rows = []
    for point in ("candidate", "power-off", "round3"):
        models = [MeasuredProfile(profiles[point][skill]) for skill in SKILLS]
        scores = {name: float(loss[0]) for name, loss in objective(models, None).items()}
        rows.append((point, sum(scores.values()), scores))
    table(lines, ("Profile", "Weighted loss", *WEIGHTS), [
        (point, number(total, 3), *(number(scores[name], 3) for name in WEIGHTS))
        for point, total, scores in rows
    ])
    return {point: {"total": total, **scores} for point, total, scores in rows}, clears


def report(output, baselines=()):
    result = json.loads((output / "solution.json").read_bytes())
    budget = json.loads((output / "cost-estimate.json").read_bytes())
    proof = json.loads((output / "determinism.json").read_bytes())
    pilot, _, pilot_seconds = load_stage(output, "pilot")
    confirm, campaigns, confirm_seconds = load_stage(output, "confirm")
    if (confirm["source_revision"] != proof["source_revision"]
            or pilot["source_revision"] != proof["source_revision"] or not proof["bytes_equal"]):
        raise ValueError("confirmation must use the revision of the matched pilot")
    if confirm["points"]["candidate"] != result["values"]:
        raise ValueError("confirmation used a different candidate")
    candidate_jobs = [job for job in confirm["jobs"] if job["point"] == "candidate"]
    if any(job["tuning"] != result["defaults_overlay"] for job in candidate_jobs):
        raise ValueError("confirmation changed the fitted candidate overlay")
    cohorts = confirmed_cohorts(campaigns)
    profiles = {point: {s: observe(cases[s]) for s in SKILLS} for point, cases in cohorts.items()}
    actual = profiles["candidate"]
    previous = []
    lines = ["# Rat Run horde power study, round 4", "",
             f"Round 4 screened `{result['revision']}` and confirmed `{confirm['source_revision']}`. "
             "Campaigns followed Sewer, Kitchen, Lab and Flatbed with a fresh Nest. "
             f"Confirmation used seeds {CONFIRM_SEED}–{CONFIRM_SEED + CONFIRM_RUNS - 1} for every skill.", ""]
    for round_no, baseline in enumerate(baselines, 1):
        prior = json.loads(baseline.read_bytes())
        previous.append((round_no, prior["observed_mean_and_count"]))
        seed = 4000 + round_no * 1000
        lines += [f"Round {round_no} used `{prior['source_revision']}` and seeds {seed}–{seed + 299}.", ""]
    confirmation(lines, cohorts["candidate"], actual, result["candidate"], previous)
    measured_scores, clears = profile_comparison(lines, cohorts, profiles)
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
              "", "Integrated-by: power-scaling (Sol worker)", ""]
    with (output / "sensitivity.csv").open("w", newline="") as dest:
        writer = csv.writer(dest)
        writer.writerow(["skill", "knob", "knob_delta", "outcome", "modeled_effect"])
        deltas = {knob.name: f"{sensitivity_step(knob):g}{knob.unit}" for knob in KNOBS}
        for skill, knobs in result["sensitivity"].items():
            for knob, metrics in knobs.items():
                writer.writerows((skill, knob, deltas[knob], key, value) for key, value in metrics.items())
    write_plots(output, result, actual, previous)
    (output / "confirmation.json").write_text(json.dumps({
        "source_revision": confirm["source_revision"],
        "screen_source_revision": result["revision"],
        "predicted": result["candidate"],
        "observed_mean_and_count": actual,
        "skill_win_wilson95": {s: wilson(*actual[s]["win"]) for s in SKILLS},
        "observed_profiles": profiles,
        "measured_weighted_loss": measured_scores,
        "floor_clear_counts": clears,
    }, indent=2))
    (output / "report.md").write_text("\n".join(lines))
    summary = ", ".join(f"{s} {actual[s]['win'][0]:.1%}" for s in SKILLS)
    (output / "fitted-candidate.toml").write_text(
        "# Rat Run round 4 power-scaling candidate.\n"
        f"# Fresh seeds {CONFIRM_SEED}–{CONFIRM_SEED + CONFIRM_RUNS - 1}, measured campaign wins {summary}.\n"
        "# See report.md for target conformance, uncertainty and predictions.\n\n"
        + result["defaults_overlay"],
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--baseline", type=Path, action="append", default=[],
                        help="Earlier confirmation.json, in order from round 1")
    args = parser.parse_args()
    report(args.output, args.baseline)


if __name__ == "__main__":
    main()
