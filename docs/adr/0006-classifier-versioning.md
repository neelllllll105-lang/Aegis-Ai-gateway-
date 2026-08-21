# ADR-006: Classifier versions and sign-constrained training

- **Status:** Accepted
- **Date:** 2026-08-20
- **Reframes:** `MASTER_BUILD.md` P5.3 ("ONNX in-process logistic model") and the
  acceptance criterion "classifier v2 beats v1"

## Context

The blueprint specifies a heuristic classifier for Phase 3 and an ONNX-served trained model
for Phase 5, with the acceptance criterion that v2 beats v1.

Two problems emerged while building it.

**ONNX is disproportionate here.** The feature vector is twelve normalised numbers and the
model is linear. An ONNX runtime adds a substantial dependency, a model file to ship and
version, and a loading failure mode in the hot path — to evaluate a dot product. The
weights can simply be constants in the source, which is faster, reviewable in a diff, and
has no failure mode at all.

**"V2 beats V1" could not be honestly demonstrated.** Both classifiers reach 98% on the
100-case labelled fixture set. The set is too small and too clean to separate them. It
would have been easy to add fixtures until v2 won, but that measures nothing except the
fixtures.

## Decision

**Versioning:** `ClassifierVersion::V1` (hand-tuned heuristics) and `V2` (linear model over
the same features), selected by flag. Both consume an identical `Features` vector, so a
version change alters scoring only — feature extraction, and therefore the explanation
shown to a customer, is unchanged.

**No ONNX.** V2 weights are `const` arrays fitted offline by `scripts/train_classifier.py`.

**Training uses features dumped from the real extractor.** `cargo run --bin dump_features`
emits the exact vectors the gateway computes. Reimplementing extraction in Python would be
the obvious shortcut and the obvious bug: the two copies drift, and the model ends up
fitted to features nobody serves.

**The fit is sign-constrained.** Each weight is forced to the direction the domain requires.
An unconstrained fit scored 99% on the fixtures with a *negative* weight on system prompt
length and almost none on input length — implying a longer prompt with a longer system
message is a simpler request. That model would have passed the acceptance criterion and
routed real traffic badly. The constraints cost about a point of fixture accuracy.

**The acceptance criterion is restated** as "V2 does not regress and clears the 85% bar",
with the reasoning recorded in the test itself. V2 earns its place by being *retrainable*
from production outcomes without a code change, which is the Phase 7 path.

## Consequences

**Cost.** A trained classifier that does not currently outperform the heuristic is, today,
strictly more machinery for the same result. It is kept because the retraining pipeline is
the asset, not the current weights.

**Benefit.** No model file, no runtime dependency, no loading failure. Weights are visible
in code review — a change that would route traffic differently shows up as a diff on a
constant, which is the right level of scrutiny for a decision that spends customer money.

**Honesty cost.** The project cannot claim "trained classifier beats heuristics". It can
claim a retraining pipeline wired end to end that produces a model matching the heuristic
on available data. That is the true statement, and it is recorded in `MEMORY.md` under
Known Limitations so nobody later mistakes it for the stronger claim.

## When to revisit

Once real routing outcomes exist at volume, retrain on them and re-measure. If V2 then
genuinely beats V1, tighten the test to assert it. If it still does not after training on
real data, delete V2 rather than carrying it.
