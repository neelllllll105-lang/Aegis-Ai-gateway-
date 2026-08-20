#!/usr/bin/env python3
"""Fit the V2 complexity classifier weights.

The V2 scorer is a linear model over the same feature vector the V1 heuristics use. This
script fits the weights offline; the gateway only ever evaluates them, so there is no
training code, no model file, and no extra latency in the request path.

Usage
-----
    cargo run --bin dump_features > features.json
    python scripts/train_classifier.py features.json

Prints a Rust array to paste into `V2_WEIGHTS` in
`apps/gateway/src/engine/classifier.rs`, along with the accuracy the fit achieves.

Why ordinal regression on target scores rather than a classifier
----------------------------------------------------------------
The gateway needs a continuous 0..1 score, not just a label: the score is stored on every
usage record and shown to customers as the reason a request was routed. Fitting to target
scores keeps the model's output directly comparable to the V1 heuristic and to the
thresholds in `Complexity::from_score`.

The asymmetric loss below encodes the product rule from `MASTER_BUILD.md` Part 5:
under-estimating complexity degrades a customer's answer, while over-estimating it only
costs us margin. Errors in the cheap direction are therefore penalised harder.
"""

from __future__ import annotations

import json
import sys

# Band midpoints. Thresholds are 0.35 and 0.70, so these sit clear of both boundaries.
TARGETS = {"simple": 0.15, "medium": 0.50, "complex": 0.85}

FEATURE_NAMES = [
    "length",
    "turns",
    "code_block",
    "code_terms",
    "code_intent",
    "explanatory",
    "multi_step",
    "reasoning",
    "system_length",
    "trivial",
    "short_question",
    "tools",
]

# Penalty multiplier for predicting lower complexity than the truth.
UNDER_ESTIMATE_PENALTY = 2.5

LEARNING_RATE = 0.08
EPOCHS = 6000
L2 = 0.02

# Sign constraints: +1 means the feature can only ever increase complexity, -1 only ever
# decrease it.
#
# With fewer than a hundred correlated examples, an unconstrained fit happily learns
# nonsense that scores well on the fixtures — in an early run it gave `system_length` a
# negative weight and `length` almost none, implying a longer prompt with a longer system
# message is a *simpler* request. That model would have scored 99% here and routed real
# traffic badly. These constraints encode what we actually know about the domain and cost
# roughly a point of fixture accuracy, which is the right trade.
SIGNS = {
    "length": +1,
    "turns": +1,
    "code_block": +1,
    "code_terms": +1,
    "code_intent": +1,
    "explanatory": +1,
    "multi_step": +1,
    "reasoning": +1,
    "system_length": +1,
    "trivial": -1,
    "short_question": -1,
    "tools": +1,
}


def band(score: float) -> str:
    if score < 0.35:
        return "simple"
    if score <= 0.70:
        return "medium"
    return "complex"


def fit(rows):
    n_features = len(FEATURE_NAMES)
    weights = [0.0] * n_features
    bias = 0.3

    for _ in range(EPOCHS):
        gradients = [0.0] * n_features
        bias_gradient = 0.0

        for features, target in rows:
            prediction = bias + sum(w * f for w, f in zip(weights, features))
            error = prediction - target
            # Predicting too low is the expensive mistake; weight it accordingly.
            weight = UNDER_ESTIMATE_PENALTY if error < 0 else 1.0
            scaled = error * weight

            for i, f in enumerate(features):
                gradients[i] += scaled * f
            bias_gradient += scaled

        count = len(rows)
        for i in range(n_features):
            gradient = gradients[i] / count + L2 * weights[i]
            weights[i] -= LEARNING_RATE * gradient
            # Project back onto the allowed sign, and cap magnitude so no single feature
            # can dominate the score.
            sign = SIGNS[FEATURE_NAMES[i]]
            if sign > 0:
                weights[i] = min(max(weights[i], 0.0), 0.60)
            else:
                weights[i] = max(min(weights[i], 0.0), -0.60)
        bias -= LEARNING_RATE * (bias_gradient / count)
        bias = min(max(bias, 0.0), 0.5)

    return weights, bias


def evaluate(rows_with_labels, weights, bias):
    correct = 0
    misses = []
    for name, features, label in rows_with_labels:
        score = min(1.0, max(0.0, bias + sum(w * f for w, f in zip(weights, features))))
        predicted = band(score)
        if predicted == label:
            correct += 1
        else:
            misses.append(f"{name}: expected {label}, got {predicted} ({score:.3f})")
    return correct / len(rows_with_labels) * 100.0, misses


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2

    with open(sys.argv[1], encoding="utf-8") as handle:
        data = json.load(handle)

    # Tool requests never reach the linear model — they short-circuit to a fixed floor —
    # so including them would fit the weights to a decision the model never makes.
    trainable = [row for row in data if not row["tool_shortcircuit"]]
    if not trainable:
        print("no trainable rows", file=sys.stderr)
        return 1

    training_rows = [(row["features"], TARGETS[row["label"]]) for row in trainable]
    labelled_rows = [(row["name"], row["features"], row["label"]) for row in trainable]

    weights, bias = fit(training_rows)
    accuracy, misses = evaluate(labelled_rows, weights, bias)

    print(f"// Fitted on {len(trainable)} non-tool cases. Accuracy: {accuracy:.1f}%")
    if misses:
        print("// Misclassified:")
        for miss in misses:
            print(f"//   {miss}")
    print()
    print(f"pub const V2_WEIGHTS: [f32; {len(FEATURE_NAMES)}] = [")
    for name, weight in zip(FEATURE_NAMES, weights):
        print(f"    {weight:+.4f}, // {name}")
    print("];")
    print()
    print(f"pub const V2_BIAS: f32 = {bias:.4f};")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
