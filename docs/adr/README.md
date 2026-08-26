# Architecture Decision Records

Every significant technical decision, and **every deviation from `MASTER_BUILD.md`**, is
recorded here. `MASTER_BUILD.md` Part 15 rule 3 requires it: when the blueprint is
ambiguous or wrong, the resolution gets written down rather than silently applied.

The point is not ceremony. It is that six months from now, someone — possibly you — will
look at a piece of code that seems obviously wrong, and the only thing standing between
them and re-introducing the bug it was written to avoid is a record of why.

## Index

| ADR | Title | Status | Deviates from blueprint? |
|-----|-------|--------|--------------------------|
| [001](0001-rust-axum-gateway.md) | Rust and Axum for the gateway | Accepted | No — confirms Part 2 |
| [002](0002-money-as-integers.md) | Money as micro-cent integers | Accepted | No — implements Principle 9 |
| [003](0003-deployment-topology.md) | Single-server Hetzner topology | Accepted | No — confirms Part 10 |
| [004](0004-runtime-checked-sql.md) | Runtime-checked SQL instead of `query!` macros | Accepted | **Yes** — Part 2 and Part 12 |
| [005](0005-store-abstraction.md) | Key-value store behind a trait | Accepted | **Yes** — extends Part 2 |
| [006](0006-classifier-versioning.md) | Classifier versions and sign-constrained training | Accepted | Partial — reframes P5.3 |
| [007](0007-vertex-ai-jwt-signing.md) | `jsonwebtoken` for Vertex AI service-account auth | Accepted | **Yes** — new dependency outside Part 2 |

## Writing a new one

Copy the structure of an existing record. Keep it short — a page is plenty. The sections
that matter are **Context** (what forced a decision), **Decision** (what was chosen), and
**Consequences** (what this costs, including what it makes harder).

A record that lists only advantages is not an ADR, it is marketing. State the trade-off
honestly, including the circumstances under which the decision should be revisited.

Number records sequentially. Never edit an accepted record to change its decision —
supersede it with a new one and update the status of the old.
