//! The optimization engine — stages [6a] through [6c] and [7] of the pipeline.
//!
//! This is what makes Aegis a product rather than a proxy: classify the request, choose
//! the cheapest model that will still do the job, compress what can be compressed, and
//! fail over cleanly when a provider breaks.

pub mod bandit;
pub mod classifier;
pub mod compressor;
pub mod fallback;
pub mod governance;
pub mod policy;
pub mod router;
