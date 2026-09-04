# ADR-002: Money as micro-cent integers

- **Status:** Accepted
- **Date:** 2026-08-20
- **Implements:** Principle 9 and Part 13 item 1

## Context

Aegis bills a percentage of savings. That means arithmetic on very small amounts, summed
over very many requests: a single routing decision might save $0.007, and a monthly
invoice is the sum of several million such figures.

Floating point is unfit for this. `0.1 + 0.2 != 0.3` in IEEE 754, and the error compounds
with the number of operations. Summing a million float costs produces a figure wrong in a
way that depends on the order of summation — so two runs of the same billing job can
disagree, and neither matches what a customer computes by hand from their own logs.

Part 13 item 1 states the stakes plainly: a penny-wrong invoice destroys trust.

## Decision

All money is `MicroCents`, a newtype over `i64` counting micro-cents, where
1 cent = 10,000 micro-cents and 1 USD = 1,000,000 micro-cents.

The unit is chosen so the smallest amount we ever need to represent — a fraction of a cent
for a single cheap request — is a whole number. At `i64` the range covers roughly
±9.2 trillion dollars, which is not a limit anyone will reach.

Three rules make it hold:

- Arithmetic **saturates** rather than wrapping. A billing bug that clamps is bad; one that
  wraps a large positive into a large negative is catastrophic and silent.
- Intermediates use `i128` where a product could overflow before a division brings it back
  into range (`cost_for_tokens`).
- Rates are **basis points**, not fractions. A 20% savings share is `2_000`, so the fee
  calculation is integer multiplication and division with explicit rounding, never a float
  multiply.

Floating point appears in exactly two places, both deliberate and both outside the billing
path: converting a published price sheet at load time, and formatting for display.

## Consequences

**Cost.** Every monetary value needs conversion at the presentation boundary, and the
`MicroCents` type must be threaded through the whole system. Reading a raw
`baseline_cost_mc` column requires knowing the unit.

**Benefit.** Summation is exact and order-independent.
`sum_of_many_small_costs_stays_exact` asserts that a million 3-micro-cent charges total
exactly 3,000,000 — a test that fails immediately on any float regression. A customer
recomputing their invoice from the CSV export gets the same number we did, to the
micro-cent.

**Enforced by.** Database columns are `BIGINT`, never `NUMERIC` or `DOUBLE PRECISION`. Even
non-monetary fractional values (`gateway_overhead_us`, `complexity_score_milli`) are stored
as scaled integers, so no decimal type exists anywhere near the billing tables.

## When to revisit

Multi-currency support. Micro-cents are implicitly USD; a second currency needs a currency
tag alongside the amount and an explicit policy on when conversion happens. Do not add a
second currency without revisiting this record.
