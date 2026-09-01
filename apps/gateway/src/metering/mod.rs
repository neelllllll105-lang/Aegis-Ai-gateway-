//! Metering: pricing, usage events, and savings attribution.
//!
//! Principle 2 lives here. Every request that touches the gateway produces a
//! [`usage::UsageEvent`] priced by [`pricing::PricingTable`] and attributed by
//! [`savings::SavingsBreakdown`].

pub mod openrouter_reference;
pub mod pricing;
pub mod savings;
pub mod usage;
