//! Data residency (Phase 6, `P6.5`).
//!
//! An organisation can pin its traffic to a region. For an EU bank or a German health
//! insurer this is not a preference but a contractual requirement, and getting it wrong is
//! a regulatory incident rather than a bug.
//!
//! The enforcement model is deliberately simple: each gateway instance knows its own
//! region, and refuses to serve an organisation pinned elsewhere. Routing traffic to the
//! right region is Cloudflare load balancing; this module is the backstop that makes a
//! misrouted request fail loudly instead of quietly processing data in the wrong
//! jurisdiction.

use crate::error::{AegisError, Result};

/// Regions we operate in.
pub const KNOWN_REGIONS: &[&str] = &["eu-central", "eu-north", "us-east", "ap-southeast"];

/// Whether a region is one we recognise.
pub fn is_known_region(region: &str) -> bool {
    KNOWN_REGIONS.contains(&region)
}

/// Whether data for an organisation may be processed by this instance.
pub fn may_process(instance_region: &str, org_region: &str) -> bool {
    instance_region.eq_ignore_ascii_case(org_region)
}

/// Enforce residency, returning a clear error when a request reached the wrong region.
pub fn enforce(instance_region: &str, org_region: &str) -> Result<()> {
    if may_process(instance_region, org_region) {
        return Ok(());
    }
    Err(AegisError::Forbidden(format!(
        "this organisation is pinned to the {org_region} region and cannot be served by \
         the {instance_region} gateway. Point your client at the {org_region} endpoint."
    )))
}

/// Whether a region sits inside the EU data boundary.
///
/// Used by the compliance pack to answer, per organisation, whether personal data leaves
/// the EU.
pub fn is_eu(region: &str) -> bool {
    region.to_ascii_lowercase().starts_with("eu-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_regions_may_process() {
        assert!(may_process("eu-central", "eu-central"));
        assert!(enforce("eu-central", "eu-central").is_ok());
    }

    #[test]
    fn region_matching_is_case_insensitive() {
        assert!(may_process("EU-Central", "eu-central"));
    }

    #[test]
    fn a_mismatched_region_is_refused_with_an_actionable_message() {
        // A misrouted request must fail loudly, not quietly process data in the wrong
        // jurisdiction.
        let err = enforce("us-east", "eu-central").unwrap_err();
        assert_eq!(err.error_type(), "forbidden");

        let message = format!("{err}");
        assert!(message.contains("eu-central"), "{message}");
        assert!(message.contains("us-east"), "{message}");
        assert!(message.contains("Point your client"), "{message}");
    }

    #[test]
    fn known_regions_are_recognised() {
        assert!(is_known_region("eu-central"));
        assert!(is_known_region("us-east"));
        assert!(!is_known_region("mars-north"));
    }

    #[test]
    fn eu_regions_are_identified_for_the_compliance_pack() {
        assert!(is_eu("eu-central"));
        assert!(is_eu("eu-north"));
        assert!(!is_eu("us-east"));
        assert!(!is_eu("ap-southeast"));
    }
}
