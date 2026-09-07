//! Plan-gated features — the single source of truth for which dashboard pages and
//! management-API writes require which subscription plan.
//!
//! Two things read this, and only this: `routes::management::billing_plan` (so the
//! dashboard's nav filtering never hardcodes the mapping — it just reads what the API says
//! this org's plan includes), and the write-endpoint guards in `routes::management` that
//! make a page being hidden in the UI an actual restriction rather than a suggestion. Before
//! this module existed, nothing on the backend checked plan at all: a Free-tier org could
//! call `POST /api/org/teams` (or budgets, or policies, or a BYOK credential) directly and
//! it would simply succeed, regardless of what the dashboard chose to show.
//!
//! The tier ladder and feature set below follow `MASTER_BUILD.md` Part 2's own pricing
//! table (Free $0 basic dashboard < Pro $29/mo BYOK + savings dashboard < Team $299/mo org
//! features/budgets/policies < Enterprise $2k+/mo SSO/SCIM/audit/residency) — this module
//! does not invent a new product shape, it encodes the one already sold.

use serde::Serialize;

/// Subscription tier, ordered low to high so a feature's minimum requirement is one
/// comparison rather than a match arm per plan per feature. Mirrors `types::ModelTier`'s
/// shape and derive set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanTier {
    Free,
    Pro,
    Team,
    Enterprise,
}

impl PlanTier {
    /// Parse a plan name as stored on `organizations.plan`. Unknown values — including the
    /// legacy/headless `"api"` plan, which is sold but was never part of the dashboard tier
    /// ladder `MASTER_BUILD.md` describes — fall to `Free`: the conservative reading, since
    /// it under-promises rather than over-grants a feature nobody paid for.
    pub fn parse(plan: &str) -> PlanTier {
        match plan {
            "pro" => PlanTier::Pro,
            "team" => PlanTier::Team,
            "enterprise" => PlanTier::Enterprise,
            _ => PlanTier::Free,
        }
    }

    /// The plan name as sold — round-trips through `parse` for every real tier (not `"api"`,
    /// which `parse` treats as `Free` and has no tier of its own to round-trip back to).
    pub fn as_str(self) -> &'static str {
        match self {
            PlanTier::Free => "free",
            PlanTier::Pro => "pro",
            PlanTier::Team => "team",
            PlanTier::Enterprise => "enterprise",
        }
    }
}

/// A dashboard page or management-API write gated behind a minimum plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// Bring-your-own-key provider credentials — Pro's own "savings dashboard" and BYOK
    /// bullet, `MASTER_BUILD.md` Part 2.
    Byok,
    /// The savings/ROI breakdown page.
    Savings,
    /// Per-project and per-person budgets.
    Budgets,
    /// The organisation routing-policy engine.
    Policies,
    /// Creating/renaming/deleting projects (teams) and managing their rosters.
    TeamManagement,
    /// SSO/SCIM/audit-log/data-residency surfaces — Enterprise only, no dedicated dashboard
    /// page today, so this currently gates a section inside Settings rather than a route.
    Enterprise,
}

impl Feature {
    pub const ALL: [Feature; 6] = [
        Feature::Byok,
        Feature::Savings,
        Feature::Budgets,
        Feature::Policies,
        Feature::TeamManagement,
        Feature::Enterprise,
    ];

    /// The lowest plan tier that includes this feature.
    pub fn min_plan(self) -> PlanTier {
        match self {
            Feature::Byok | Feature::Savings => PlanTier::Pro,
            Feature::Budgets | Feature::Policies | Feature::TeamManagement => PlanTier::Team,
            Feature::Enterprise => PlanTier::Enterprise,
        }
    }

    /// Stable, machine-readable name — used both as the JSON key in `GET /api/billing/plan`
    /// and as `AegisError::PlanRestricted`'s `feature` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Byok => "byok",
            Feature::Savings => "savings",
            Feature::Budgets => "budgets",
            Feature::Policies => "policies",
            Feature::TeamManagement => "team_management",
            Feature::Enterprise => "enterprise",
        }
    }
}

/// Whether a plan includes a feature.
pub fn plan_includes(plan: &str, feature: Feature) -> bool {
    PlanTier::parse(plan) >= feature.min_plan()
}

/// Every feature a plan includes, in `Feature::ALL` order — the payload
/// `GET /api/billing/plan` hands the dashboard so it never hardcodes this mapping itself.
pub fn plan_features(plan: &str) -> Vec<Feature> {
    let tier = PlanTier::parse(plan);
    Feature::ALL
        .into_iter()
        .filter(|f| tier >= f.min_plan())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_names_parse_to_the_documented_tier() {
        assert_eq!(PlanTier::parse("free"), PlanTier::Free);
        assert_eq!(PlanTier::parse("pro"), PlanTier::Pro);
        assert_eq!(PlanTier::parse("team"), PlanTier::Team);
        assert_eq!(PlanTier::parse("enterprise"), PlanTier::Enterprise);
    }

    #[test]
    fn an_unrecognised_plan_name_is_treated_as_free_not_trusted() {
        // Including "api" — sold, but never part of the dashboard tier ladder — and any
        // typo or future plan this module hasn't been taught yet. Defaulting to Free is
        // the conservative reading: it can only under-grant, never over-grant.
        for plan in ["api", "", "Pro", "PRO", "unlimited", "trial"] {
            assert_eq!(PlanTier::parse(plan), PlanTier::Free, "plan {plan:?}");
        }
    }

    #[test]
    fn plan_tier_names_round_trip_through_parse() {
        for tier in [
            PlanTier::Free,
            PlanTier::Pro,
            PlanTier::Team,
            PlanTier::Enterprise,
        ] {
            assert_eq!(PlanTier::parse(tier.as_str()), tier);
        }
    }

    #[test]
    fn tiers_are_ordered_free_through_enterprise() {
        assert!(PlanTier::Free < PlanTier::Pro);
        assert!(PlanTier::Pro < PlanTier::Team);
        assert!(PlanTier::Team < PlanTier::Enterprise);
    }

    #[test]
    fn free_includes_nothing_gated() {
        for feature in Feature::ALL {
            assert!(!plan_includes("free", feature), "{feature:?}");
        }
        assert!(plan_features("free").is_empty());
    }

    #[test]
    fn pro_includes_byok_and_savings_but_not_team_or_enterprise_features() {
        assert!(plan_includes("pro", Feature::Byok));
        assert!(plan_includes("pro", Feature::Savings));
        assert!(!plan_includes("pro", Feature::Budgets));
        assert!(!plan_includes("pro", Feature::Policies));
        assert!(!plan_includes("pro", Feature::TeamManagement));
        assert!(!plan_includes("pro", Feature::Enterprise));
    }

    #[test]
    fn team_includes_everything_pro_does_plus_org_features_but_not_enterprise() {
        for feature in [
            Feature::Byok,
            Feature::Savings,
            Feature::Budgets,
            Feature::Policies,
            Feature::TeamManagement,
        ] {
            assert!(plan_includes("team", feature), "{feature:?}");
        }
        assert!(!plan_includes("team", Feature::Enterprise));
    }

    #[test]
    fn enterprise_includes_every_feature() {
        for feature in Feature::ALL {
            assert!(plan_includes("enterprise", feature), "{feature:?}");
        }
        assert_eq!(plan_features("enterprise").len(), Feature::ALL.len());
    }

    #[test]
    fn plan_features_lists_grow_monotonically_up_the_ladder() {
        let free = plan_features("free");
        let pro = plan_features("pro");
        let team = plan_features("team");
        let enterprise = plan_features("enterprise");
        assert!(free.len() < pro.len());
        assert!(pro.len() < team.len());
        assert!(team.len() < enterprise.len());
        // Every feature a lower tier has, a higher tier also has -- no feature is ever
        // taken away by upgrading.
        for f in &pro {
            assert!(team.contains(f), "{f:?} present in pro but missing in team");
        }
        for f in &team {
            assert!(
                enterprise.contains(f),
                "{f:?} present in team but missing in enterprise"
            );
        }
    }

    #[test]
    fn feature_names_are_stable_strings() {
        assert_eq!(Feature::Byok.as_str(), "byok");
        assert_eq!(Feature::Savings.as_str(), "savings");
        assert_eq!(Feature::Budgets.as_str(), "budgets");
        assert_eq!(Feature::Policies.as_str(), "policies");
        assert_eq!(Feature::TeamManagement.as_str(), "team_management");
        assert_eq!(Feature::Enterprise.as_str(), "enterprise");
    }
}
