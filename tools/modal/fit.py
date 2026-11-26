"""Fit and solve Rat Run's campaign difficulty response surface."""

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import tomllib

import numpy as np
from scipy.optimize import differential_evolution
from sklearn.linear_model import LogisticRegression, Ridge
from sklearn.model_selection import KFold, train_test_split
from sklearn.preprocessing import PolynomialFeatures
from threadpoolctl import threadpool_limits

from app import OUTPUT, SKILLS
from design import BOUNDS, KNOBS, overlay

FLOORS = ("Sewer", "Kitchen", "Lab", "Flatbed")
BOSSES = ("jaguar", "horseshoe crab", "broom human", "subject r-01",
          "mower", "king cobra", "magpie", "the redtail")
PHASES = ((60, 180), (180, 360), (360, 660), (660, 850))
WIN_BANDS = np.array([(0.75, 0.85), (0.40, 0.55), (0.10, 0.20)])
WEIGHTS = {"wins": 35, "death_spread": 20, "hp_tension": 20,
           "level_ups": 10, "bosses": 10, "close_calls": 5}
FLOOR_DEATH = [f"death_floor_{f}" for f in range(1, 5)]
# The batch caps total simulated time at 4,800 s.
WINDOW_DEATH = [f"death_window_{w}" for w in range(1, 28)]
BINARY = [["win", "loss"], ["close_win", "no_close_win"]]
BINARY += [[f"clear_{b}", f"fall_{b}"] for b in BOSSES]
PROBABILITY_GROUPS = [*BINARY, FLOOR_DEATH, WINDOW_DEATH]


def load_stage(output, name):
    directory = output / "raw" / name
    spec = json.loads((directory / "manifest.json").read_bytes())
    points = defaultdict(list)
    estimated_seconds = 0
    for job in spec["jobs"]:
        receipt = json.loads((directory / f"{job['id']}.receipt").read_bytes())
        raw = (directory / f"{job['id']}.json").read_bytes()
        if hashlib.sha256(raw).hexdigest() != receipt["sha256"]:
            raise ValueError(f"checksum mismatch for {job['id']}")
        runs = json.loads(raw)["run_summaries"]
        seeds = range(job["seed"], job["seed"] + job["runs"])
        if [run["seed"] for run in runs] != list(seeds):
            raise ValueError(f"incomplete shard {job['id']}")
        points[job["point"]].extend(runs)
        estimated_seconds += receipt["seconds"]
    return spec, points, estimated_seconds


def observe(runs):
    """Return means and denominators. Missing cohorts have zero observations."""
    groups = [*PROBABILITY_GROUPS]
    samples = {key: [] for group in groups for key in group}
    for boss in BOSSES:
        samples[f"boss_seconds_name_{boss}"] = []
        samples[f"boss_hp_lost_name_{boss}"] = []
    for f in range(1, 5):
        samples.update({f"{key}_{f}": [] for key in (
            "reach", "boss_reach", "boss_seconds", "boss_hp_lost", "min_hp", "hp_lost",
            "first_card", "fifth_card", "tenth_card", "fifteenth_card", "cards_15m",
        )})
        samples.update({f"hp_phase_{f}_{p}": [] for p in range(len(PHASES))})
        samples.update({f"hp_{f}_{t}": [] for t in range(10, 901, 10)})

    def category(group, chosen):
        for key in group:
            samples[key].append(float(key == chosen))

    for run in runs:
        won = run["won"]
        reached = run["floor_reached"]
        category(BINARY[0], "win" if won else "loss")
        for f in range(1, 5):
            samples[f"reach_{f}"].append(float(reached >= f))
        if won:
            category(BINARY[1], "close_win" if run["dropped_below_30_percent_hp"] else "no_close_win")
        else:
            death = run["death"]
            category(FLOOR_DEATH, f"death_floor_{death['level'] + 1}")
            category(WINDOW_DEATH, f"death_window_{int(death['at'] // 180) + 1}")

        for floor in run["levels"]:
            f = floor["level"] + 1
            boss = floor["boss"]
            cleared = floor["boss_defeated"]
            samples[f"boss_reach_{f}"].append(float(boss is not None))
            if boss is not None:
                group = [f"clear_{boss}", f"fall_{boss}"]
                category(group, group[0 if cleared else 1])
                samples[f"boss_hp_lost_{f}"].append(floor["boss_hp_lost"])
                samples[f"boss_hp_lost_name_{boss}"].append(floor["boss_hp_lost"])
            if cleared:
                samples[f"boss_seconds_{f}"].append(floor["boss_seconds"])
                if boss is not None:
                    samples[f"boss_seconds_name_{boss}"].append(floor["boss_seconds"])
            samples[f"hp_lost_{f}"].append(floor["hp_lost"])
            samples[f"min_hp_{f}"].append(floor["min_hp_fraction"])
            hp = floor["hp_samples_10s"]
            for i, value in enumerate(hp[:90], start=1):
                samples[f"hp_{f}_{i * 10}"].append(value)
            for phase, (early, late) in enumerate(PHASES):
                start, end = early // 10 - 1, late // 10 - 1
                if len(hp) >= end:
                    samples[f"hp_phase_{f}_{phase}"].append(float(np.mean(hp[start:end])))
            cards = floor["level_up_times"]
            for rank, name in [(1, "first"), (5, "fifth"), (10, "tenth"), (15, "fifteenth")]:
                if len(cards) >= rank:
                    samples[f"{name}_card_{f}"].append(cards[rank - 1])
            if floor["time"] >= 900:
                samples[f"cards_15m_{f}"].append(sum(t <= 900 for t in cards))
    return {
        key: (float(np.mean(values)), len(values)) if values else (0., 0)
        for key, values in samples.items()
    }


class Probability:
    """A binomial or multinomial response on point-level class counts."""

    def __init__(self, x, counts, regularization):
        # A few pseudo-observations at the fit center keep unseen classes
        # representable and the rare-boss fit from predicting certainties.
        classes = counts.shape[1]
        xs = np.vstack([np.repeat(x, classes, axis=0), np.zeros((classes, x.shape[1]))])
        labels = np.tile(np.arange(classes), len(x) + 1)
        weights = np.r_[counts.ravel(), np.full(classes, min(.25, 1. / classes))]
        valid = weights > 0
        self.model = LogisticRegression(C=regularization, max_iter=1000, solver="lbfgs")
        self.model.fit(xs[valid], labels[valid], sample_weight=weights[valid])

    def predict(self, x):
        return self.model.predict_proba(x)


def split_responses(profiles, keys):
    means = np.array([[row[key][0] for key in keys] for row in profiles])
    counts = np.array([[row[key][1] for key in keys] for row in profiles])
    return means, counts


class Surface:
    def __init__(self, values, profiles, bounds):
        low, high = np.array(bounds).T
        self.center = (high + low) / 2
        self.norm = (high - low) / 2
        train_x = (values - self.center) / self.norm
        self.quadratic = PolynomialFeatures(degree=2, include_bias=False)
        train_q = self.quadratic.fit_transform(train_x)
        features = (train_x, train_q)
        folds = list(KFold(n_splits=4, shuffle=True, random_state=198).split(train_x))
        self.probability = {}
        for group in PROBABILITY_GROUPS:
            means, counts = split_responses(profiles, group)
            y = means * counts
            choices = [(0, .08), (0, .4), (1, .04), (1, .20)]
            losses = []
            for order, regularization in choices:
                error = 0.
                for train, validation in folds:
                    x = features[order]
                    model = Probability(x[train], y[train], regularization)
                    p = model.predict(x[validation])
                    error -= np.sum(y[validation] * np.log(np.clip(p, 1e-12, 1.)))
                losses.append(error)
            order, regularization = choices[int(np.argmin(losses))]
            self.probability[tuple(group)] = (
                order, regularization, Probability(features[order], y, regularization),
            )

        probability_keys = {key for group in PROBABILITY_GROUPS for key in group}
        self.regression_keys = [key for key in profiles[0] if key not in probability_keys]
        y, weights = split_responses(profiles, self.regression_keys)
        scales = [i for i, key in enumerate(self.regression_keys)
                  if any(key.startswith(s) for s in ("hp_phase_", "first_card_", "cards_15m_", "boss_seconds_"))]
        choices = [(0, 6.), (0, 20.), (1, 12.), (1, 40.)]
        losses = []
        for order, alpha in choices:
            error = []
            x = features[order]
            for train, validation in folds:
                for col in scales:
                    w = weights[validation, col]
                    if not np.any(w) or not np.any(weights[train, col]):
                        continue
                    beta, intercept = self.ridge(x[train], y[train, col], weights[train, col], alpha)
                    train_y = y[train, col][weights[train, col] > 0]
                    spread = max(float(np.std(train_y)), 1e-6)
                    error.append(np.average(((x[validation] @ beta + intercept - y[validation, col])
                                             / spread) ** 2, weights=w))
            losses.append(np.mean(error) if error else np.inf)
        self.regression_order, self.alpha = choices[int(np.argmin(losses))]
        train = features[self.regression_order]
        rows = [self.ridge(train, y[:, i], weights[:, i], self.alpha)
                for i in range(len(self.regression_keys))]
        self.beta = np.stack([beta for beta, _ in rows], axis=1)
        self.intercept = np.array([intercept for _, intercept in rows])
        self.support = dict(zip(self.regression_keys, weights.sum(axis=0).tolist(), strict=True))

    @staticmethod
    def ridge(x, y, w, alpha):
        valid = w > 0
        if not np.any(valid):
            return np.zeros(x.shape[1]), np.nan
        weight = w[valid] / np.mean(w[valid])
        model = Ridge(alpha=alpha).fit(x[valid], y[valid], sample_weight=weight)
        return model.coef_, model.intercept_

    def predict(self, values, curves=True):
        x = (np.atleast_2d(values) - self.center) / self.norm
        features = (x, self.quadratic.transform(x))
        out = {}
        for group, (order, _, model) in self.probability.items():
            out.update(zip(group, model.predict(features[order]).T, strict=True))
        labels = self.regression_keys
        use = np.ones(len(labels), dtype=bool) if curves else np.array([
            not (k.startswith("hp_") and k.split("_")[1].isdigit()) for k in labels
        ])
        prediction = features[self.regression_order] @ self.beta[:, use] + self.intercept[use]
        for label, value in zip(np.array(labels)[use], prediction.T, strict=True):
            high = 1. if label.startswith(("hp_", "reach_", "boss_reach_", "min_hp_")) and not label.startswith("hp_lost_") else np.inf
            out[label] = np.clip(value, 0., high)
        # Survival follows directly from win/death mass.
        for f in range(1, 5):
            out[f"reach_{f}"] = 1. - out["loss"] * sum(out[k] for k in FLOOR_DEATH[:f - 1])
        return out


def objective(models, values):
    def penalty_band(y, bottom, top, scale):
        return (np.maximum(bottom - y, 0.) / scale) ** 2 + (np.maximum(y - top, 0.) / scale) ** 2

    def mean(items):
        return np.mean(np.asarray(items), axis=0)

    result = defaultdict(list)
    for i, model in enumerate(models):
        p = model.predict(values, curves=False)
        low, high = WIN_BANDS[i]
        win = p["win"]
        result["wins"].append(penalty_band(win, low, high, .1)
                              + .05 * ((win - (low + high) / 2) / .1) ** 2)
        floor_death = [p[k] for k in FLOOR_DEATH]
        window_death = [p[k] for k in WINDOW_DEATH]
        result["death_spread"].append(
            penalty_band(np.max(floor_death, axis=0), 0., .40, .2)
            + penalty_band(np.max(window_death, axis=0), 0., .40, .2)
            + mean([penalty_band(y, .05, 1., .05) for y in floor_death]),
        )
        for f in range(1, 5):
            hp = [p[f"hp_phase_{f}_{j}"] for j in range(len(PHASES))]
            contrasts = [hp[0] - hp[1], hp[2] - hp[1], hp[2] - hp[3]]
            result["hp_tension"].append(mean([
                penalty_band(np.nan_to_num(y, nan=-.10), .05, 1., .1) for y in contrasts
            ]))
            first = np.nan_to_num(p[f"first_card_{f}"], nan=36.)
            cards = np.nan_to_num(p[f"cards_15m_{f}"], nan=0.)
            result["level_ups"].append(penalty_band(first, 15., 21., 10.)
                                       + penalty_band(cards, 13., 17., 5.))
        for j, name in enumerate(BOSSES):
            key = f"boss_seconds_name_{name}"
            duration = p[key] if model.support[key] >= 12 else p[f"boss_seconds_{j // 2 + 1}"]
            result["bosses"].append(penalty_band(np.nan_to_num(duration, nan=120.), 40., 80., 40.))
        if SKILLS[i] == "casual":
            result["bosses"].append(mean([
                penalty_band(p[f"clear_{boss}"], .05, 1., .05) for boss in BOSSES
            ]))
        result["close_calls"].append(penalty_band(p["close_win"], .55, 1., .25))
    return {key: mean(items) * WEIGHTS[key] for key, items in result.items()}


def solve(models, bounds):
    def loss(vectors):
        return sum(objective(models, np.atleast_2d(vectors.T)).values())

    best = differential_evolution(
        loss, bounds, vectorized=True, updating="deferred",
        seed=2026, popsize=12, maxiter=300, polish=False, tol=0.002,
    )
    return best.x, float(best.fun)


def validation(model, x, profiles, baseline):
    prediction = model.predict(x)
    results = {}
    for label, values in prediction.items():
        mean, count = split_responses(profiles, [label])
        y, w = mean[:, 0], count[:, 0]
        valid = (w > 0) & np.isfinite(values)
        if not np.any(valid):
            continue
        y, w, p = y[valid], w[valid], values[valid]
        base_y, base_w = split_responses(baseline, [label])
        null = np.average(base_y[:, 0], weights=base_w[:, 0]) if np.any(base_w) else float("nan")
        row = {"points": int(np.sum(valid)), "runs": int(w.sum()),
               "mae": float(np.average(np.abs(y - p), weights=w)),
               "rmse": float(np.sqrt(np.average((y - p) ** 2, weights=w))),
               "baseline_rmse": float(np.sqrt(np.average((y - null) ** 2, weights=w)))}
        if label == "win":
            # y is the observed binomial fraction, w is the campaign count.
            row["brier"] = float(np.average(y * (1. - p) ** 2 + (1. - y) * p ** 2, weights=w))
            row["baseline_brier"] = float(np.average(
                y * (1. - null) ** 2 + (1. - y) * null ** 2, weights=w,
            ))
            row["predicted"] = float(np.average(p, weights=w))
            row["observed"] = float(np.average(y, weights=w))
        results[label] = row
    return results


def boundary_tradeoffs(models, values, bounds, candidate_loss):
    """Measure which targets change on moving each near-edge knob inward."""
    edges = {}
    for i, (value, (low, high)) in enumerate(zip(values, bounds, strict=True)):
        width = high - low
        if value - low <= width * .02:
            side, interior = "lower", low + width * .10
        elif high - value <= width * .02:
            side, interior = "upper", high - width * .10
        else:
            continue
        moved = values.copy()
        moved[i] = interior
        edges[KNOBS[i].name] = {
            "side": side, "interior": interior,
            "loss_delta": {key: loss[0] - candidate_loss[key]
                           for key, loss in objective(models, moved).items()},
        }
    return edges


def clean(value):
    """Encode unavailable predictions as JSON null."""
    if isinstance(value, dict):
        return {k: clean(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, np.ndarray)):
        return [clean(v) for v in value]
    if isinstance(value, (float, np.floating)) and not np.isfinite(value):
        return None
    return value


def fit(output, defaults):
    screen, data, seconds = load_stage(output, "screen")
    ids = list(screen["points"])
    train, test = train_test_split(ids, test_size=.20, random_state=23)
    values = screen["points"]
    bounds = BOUNDS
    if not np.array_equal(screen["bounds"], bounds):
        raise ValueError("screen manifest has different feel bounds")

    def x(ids):
        return np.array([values[i] for i in ids])

    profiles = {skill: {
        point: observe([run for run in runs if run["skill"] == skill])
        for point, runs in data.items()
    } for skill in SKILLS}
    models, scores, choices, heldout_wins = [], {}, {}, {}
    for skill in SKILLS:
        fit_rows = [profiles[skill][i] for i in train]
        model = Surface(x(train), fit_rows, bounds)
        models.append(model)
        scores[skill] = validation(model, x(test), [profiles[skill][i] for i in test], fit_rows)
        heldout_wins[skill] = {
            "predicted": model.predict(x(test), curves=False)["win"],
            "observed": [profiles[skill][i]["win"][0] for i in test],
        }
        choices[skill] = {
            "ridge": [model.regression_order + 1, model.alpha],
            "probability": {keys[0]: [order + 1, c] for keys, (order, c, _) in model.probability.items()},
            "continuous_support": model.support,
        }
    candidate, loss = solve(models, bounds)
    target_loss = {key: value[0] for key, value in objective(models, candidate).items()}
    edges = boundary_tradeoffs(models, candidate, bounds, target_loss)
    prediction = {s: {k: y[0] for k, y in m.predict(candidate).items()}
                  for s, m in zip(SKILLS, models, strict=True)}
    sensitivity = {}
    for i, knob in enumerate(KNOBS):
        before, after = x(train), x(train)
        low, high = bounds[i]
        step = 10. if knob.unit == " HP" else .10
        before[:, i] = np.maximum(low, before[:, i] - step / 2)
        after[:, i] = np.minimum(high, after[:, i] + step / 2)
        for skill, model in zip(SKILLS, models, strict=True):
            a, b = model.predict(before), model.predict(after)
            sensitivity.setdefault(skill, {})[knob.name] = {
                key: np.mean((b[key] - a[key]) * step / (after[:, i] - before[:, i])) for key in a
            }
    result = {
        "revision": screen["source_revision"], "values": candidate, "loss": loss,
        "loss_by_target": target_loss,
        "bounds": bounds, "edges": edges,
        "train": train, "holdout": test, "validation": scores, "choices": choices,
        "heldout_wins": heldout_wins,
        "candidate": prediction, "sensitivity": sensitivity, "screen_seconds": seconds,
        "training_profiles": profiles, "defaults_overlay": overlay(candidate, defaults),
    }
    return clean(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--defaults", type=Path, required=True)
    args = parser.parse_args()
    with threadpool_limits(limits=1):
        defaults = tomllib.loads(args.defaults.read_text())
        result = fit(args.output, defaults)
    (args.output / "solution.json").write_text(json.dumps(result, separators=(",", ":"), allow_nan=False))
    print(f"Loss {result['loss']:.3f}. Edges: {', '.join(result['edges']) or 'none'}.")


if __name__ == "__main__":
    main()
