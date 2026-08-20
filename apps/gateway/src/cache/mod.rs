//! Response caching — pipeline stage [5].
//!
//! Every cache key is scoped by `org_id`. A cross-tenant cache hit would serve one
//! customer another customer's data, which Part 13 calls company-ending, so the
//! tenant scope is part of the fingerprint itself rather than a filter applied after.

pub mod exact;
pub mod fingerprint;
pub mod semantic;
