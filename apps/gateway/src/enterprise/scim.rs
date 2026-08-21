//! SCIM 2.0 user provisioning (RFC 7644), Phase 6 `P6.2`.
//!
//! Enterprises manage accounts in Okta or Entra ID, not in our dashboard. SCIM is how
//! those systems push joiners, movers, and leavers automatically — and the leaver case is
//! why it is a procurement requirement: when someone is terminated, their access must be
//! revoked everywhere within minutes, without anyone remembering to log into Aegis.
//!
//! # Deactivate, never delete
//!
//! A SCIM `DELETE` deactivates the membership rather than removing rows. Their usage
//! records must survive — those are billing history — and an audit asking "who ran this
//! request in March" needs the user to still resolve.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// SCIM schema URIs.
pub const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
/// Schema URI for list responses.
pub const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
/// Schema URI for errors.
pub const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
/// Schema URI for PATCH operations.
pub const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";

/// A SCIM name object.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScimName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    #[serde(rename = "givenName", default, skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    #[serde(
        rename = "familyName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub family_name: Option<String>,
}

/// A SCIM email entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimEmail {
    pub value: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A SCIM user resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimUser {
    pub schemas: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ScimName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<ScimEmail>,
    #[serde(default = "default_active")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ScimMeta>,
}

fn default_active() -> bool {
    true
}

/// SCIM resource metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScimMeta {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(
        rename = "lastModified",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl ScimUser {
    /// Build a SCIM representation of an Aegis user.
    pub fn from_user(
        id: Uuid,
        email: &str,
        name: Option<&str>,
        active: bool,
        base_url: &str,
    ) -> ScimUser {
        ScimUser {
            schemas: vec![USER_SCHEMA.to_string()],
            id: Some(id.to_string()),
            user_name: email.to_string(),
            name: name.map(|n| ScimName {
                formatted: Some(n.to_string()),
                given_name: n.split_whitespace().next().map(|s| s.to_string()),
                family_name: n.split_whitespace().nth(1).map(|s| s.to_string()),
            }),
            emails: vec![ScimEmail {
                value: email.to_string(),
                primary: true,
                kind: Some("work".to_string()),
            }],
            active,
            meta: Some(ScimMeta {
                resource_type: "User".to_string(),
                created: None,
                last_modified: None,
                location: Some(format!(
                    "{}/scim/v2/Users/{id}",
                    base_url.trim_end_matches('/')
                )),
            }),
        }
    }

    /// The email to provision.
    ///
    /// Prefers the primary email, then any email, then `userName` — which most identity
    /// providers set to the email address anyway. Okta and Entra disagree about which
    /// field is authoritative, so all three are tried rather than assuming.
    pub fn provisioning_email(&self) -> Option<String> {
        self.emails
            .iter()
            .find(|e| e.primary)
            .or_else(|| self.emails.first())
            .map(|e| e.value.clone())
            .or_else(|| self.user_name.contains('@').then(|| self.user_name.clone()))
    }

    /// The display name, if the provider sent one.
    pub fn display_name(&self) -> Option<String> {
        self.name.as_ref().and_then(|n| {
            n.formatted
                .clone()
                .or_else(|| match (&n.given_name, &n.family_name) {
                    (Some(given), Some(family)) => Some(format!("{given} {family}")),
                    (Some(given), None) => Some(given.clone()),
                    (None, Some(family)) => Some(family.clone()),
                    (None, None) => None,
                })
        })
    }
}

/// A SCIM list response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScimListResponse {
    pub schemas: Vec<String>,
    #[serde(rename = "totalResults")]
    pub total_results: usize,
    #[serde(rename = "startIndex")]
    pub start_index: usize,
    #[serde(rename = "itemsPerPage")]
    pub items_per_page: usize,
    #[serde(rename = "Resources")]
    pub resources: Vec<ScimUser>,
}

impl ScimListResponse {
    /// Wrap a page of users.
    pub fn new(resources: Vec<ScimUser>, start_index: usize) -> ScimListResponse {
        ScimListResponse {
            schemas: vec![LIST_SCHEMA.to_string()],
            total_results: resources.len(),
            // SCIM indices are 1-based, and providers do reject 0.
            start_index: start_index.max(1),
            items_per_page: resources.len(),
            resources,
        }
    }
}

/// A SCIM error response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScimError {
    pub schemas: Vec<String>,
    pub status: String,
    #[serde(rename = "scimType", skip_serializing_if = "Option::is_none")]
    pub scim_type: Option<String>,
    pub detail: String,
}

impl ScimError {
    /// Build an error in the shape RFC 7644 requires.
    ///
    /// `status` is a *string* here, not a number. Identity providers reject a numeric
    /// status, and the resulting failure looks like an unrelated integration bug.
    pub fn new(status: u16, scim_type: Option<&str>, detail: &str) -> ScimError {
        ScimError {
            schemas: vec![ERROR_SCHEMA.to_string()],
            status: status.to_string(),
            scim_type: scim_type.map(|s| s.to_string()),
            detail: detail.to_string(),
        }
    }

    /// A 404.
    pub fn not_found(detail: &str) -> ScimError {
        ScimError::new(404, None, detail)
    }

    /// A 409 for a duplicate `userName`.
    pub fn conflict(detail: &str) -> ScimError {
        ScimError::new(409, Some("uniqueness"), detail)
    }
}

/// One operation in a SCIM PATCH request.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PatchOperation {
    /// `add`, `remove`, or `replace`.
    pub op: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// A SCIM PATCH request body.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PatchRequest {
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(rename = "Operations")]
    pub operations: Vec<PatchOperation>,
}

impl PatchRequest {
    /// Extract the intended `active` value, if this patch sets one.
    ///
    /// Deprovisioning is the operation that actually matters, and identity providers
    /// express it in inconsistent ways: Okta sends `{"op":"replace","value":{"active":false}}`
    /// while others send `{"op":"replace","path":"active","value":false}`. Both are handled,
    /// because getting this wrong means a terminated employee keeps their access.
    pub fn active_change(&self) -> Option<bool> {
        for operation in &self.operations {
            if !operation.op.eq_ignore_ascii_case("replace")
                && !operation.op.eq_ignore_ascii_case("add")
            {
                continue;
            }
            match (operation.path.as_deref(), &operation.value) {
                (Some(path), Some(value)) if path.eq_ignore_ascii_case("active") => {
                    if let Some(active) = coerce_bool(value) {
                        return Some(active);
                    }
                }
                (None, Some(serde_json::Value::Object(map))) => {
                    if let Some(active) = map.get("active").and_then(coerce_bool) {
                        return Some(active);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// Coerce a JSON value to a bool.
///
/// Some providers send the string `"False"` rather than a JSON boolean. Treating that as
/// "not a boolean, ignore it" would silently fail to deprovision the user.
fn coerce_bool(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn okta_user_json() -> serde_json::Value {
        serde_json::json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "jane.doe@acme.com",
            "name": {"givenName": "Jane", "familyName": "Doe"},
            "emails": [{"primary": true, "value": "jane.doe@acme.com", "type": "work"}],
            "active": true
        })
    }

    #[test]
    fn an_okta_user_payload_parses() {
        let user: ScimUser = serde_json::from_value(okta_user_json()).unwrap();
        assert_eq!(user.user_name, "jane.doe@acme.com");
        assert!(user.active);
        assert_eq!(
            user.provisioning_email().as_deref(),
            Some("jane.doe@acme.com")
        );
        assert_eq!(user.display_name().as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn the_primary_email_is_preferred() {
        let user: ScimUser = serde_json::from_value(serde_json::json!({
            "schemas": [USER_SCHEMA],
            "userName": "jdoe",
            "emails": [
                {"value": "personal@example.com", "primary": false},
                {"value": "work@acme.com", "primary": true}
            ]
        }))
        .unwrap();
        assert_eq!(user.provisioning_email().as_deref(), Some("work@acme.com"));
    }

    #[test]
    fn username_is_used_when_no_email_is_sent() {
        // Entra ID sometimes omits the emails array entirely.
        let user: ScimUser = serde_json::from_value(serde_json::json!({
            "schemas": [USER_SCHEMA],
            "userName": "jane@acme.com"
        }))
        .unwrap();
        assert_eq!(user.provisioning_email().as_deref(), Some("jane@acme.com"));
    }

    #[test]
    fn a_non_email_username_with_no_emails_yields_nothing() {
        let user: ScimUser = serde_json::from_value(serde_json::json!({
            "schemas": [USER_SCHEMA],
            "userName": "jdoe"
        }))
        .unwrap();
        assert_eq!(user.provisioning_email(), None);
    }

    #[test]
    fn active_defaults_to_true_when_omitted() {
        let user: ScimUser = serde_json::from_value(serde_json::json!({
            "schemas": [USER_SCHEMA],
            "userName": "jane@acme.com"
        }))
        .unwrap();
        assert!(user.active, "an omitted active flag must mean active");
    }

    #[test]
    fn deprovisioning_is_detected_in_the_okta_shape() {
        // The operation that matters most: a terminated employee losing access.
        let patch: PatchRequest = serde_json::from_value(serde_json::json!({
            "schemas": [PATCH_SCHEMA],
            "Operations": [{"op": "replace", "value": {"active": false}}]
        }))
        .unwrap();
        assert_eq!(patch.active_change(), Some(false));
    }

    #[test]
    fn deprovisioning_is_detected_in_the_path_shape() {
        let patch: PatchRequest = serde_json::from_value(serde_json::json!({
            "schemas": [PATCH_SCHEMA],
            "Operations": [{"op": "replace", "path": "active", "value": false}]
        }))
        .unwrap();
        assert_eq!(patch.active_change(), Some(false));
    }

    #[test]
    fn deprovisioning_is_detected_when_sent_as_a_string() {
        // Some providers send "False" rather than a JSON boolean. Ignoring it would leave
        // a terminated user with access.
        for value in [
            serde_json::json!("false"),
            serde_json::json!("False"),
            serde_json::json!("FALSE"),
        ] {
            let patch: PatchRequest = serde_json::from_value(serde_json::json!({
                "schemas": [PATCH_SCHEMA],
                "Operations": [{"op": "replace", "path": "active", "value": value}]
            }))
            .unwrap();
            assert_eq!(patch.active_change(), Some(false), "failed for {value}");
        }
    }

    #[test]
    fn reactivation_is_detected() {
        let patch: PatchRequest = serde_json::from_value(serde_json::json!({
            "schemas": [PATCH_SCHEMA],
            "Operations": [{"op": "replace", "path": "active", "value": true}]
        }))
        .unwrap();
        assert_eq!(patch.active_change(), Some(true));
    }

    #[test]
    fn unrelated_patches_change_nothing() {
        let patch: PatchRequest = serde_json::from_value(serde_json::json!({
            "schemas": [PATCH_SCHEMA],
            "Operations": [{"op": "replace", "path": "name.givenName", "value": "Janet"}]
        }))
        .unwrap();
        assert_eq!(patch.active_change(), None);
    }

    #[test]
    fn a_user_resource_serializes_in_the_required_shape() {
        let id = Uuid::new_v4();
        let user = ScimUser::from_user(
            id,
            "jane@acme.com",
            Some("Jane Doe"),
            true,
            "https://api.aegis.dev",
        );
        let json = serde_json::to_value(&user).unwrap();

        assert_eq!(json["schemas"][0], USER_SCHEMA);
        assert_eq!(json["userName"], "jane@acme.com");
        assert_eq!(json["id"], id.to_string());
        assert_eq!(json["active"], true);
        assert_eq!(json["meta"]["resourceType"], "User");
        assert!(json["meta"]["location"]
            .as_str()
            .unwrap()
            .contains("/scim/v2/Users/"));
    }

    #[test]
    fn list_responses_use_one_based_indices() {
        // A zero start index is rejected by real identity providers.
        let response = ScimListResponse::new(vec![], 0);
        assert_eq!(response.start_index, 1);
        assert_eq!(response.schemas[0], LIST_SCHEMA);

        let json = serde_json::to_value(&response).unwrap();
        assert!(
            json.get("Resources").is_some(),
            "Resources must be capitalised"
        );
        assert!(json.get("totalResults").is_some());
    }

    #[test]
    fn errors_carry_a_string_status_as_the_rfc_requires() {
        // A numeric status is rejected, and the failure looks like an unrelated bug.
        let error = ScimError::not_found("user not found");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["status"], "404");
        assert!(
            json["status"].is_string(),
            "status must be a string, not a number"
        );
        assert_eq!(json["schemas"][0], ERROR_SCHEMA);
    }

    #[test]
    fn conflict_errors_name_the_uniqueness_violation() {
        let error = ScimError::conflict("userName already exists");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["status"], "409");
        assert_eq!(json["scimType"], "uniqueness");
    }

    #[test]
    fn a_user_round_trips_through_serialization() {
        let user = ScimUser::from_user(
            Uuid::new_v4(),
            "jane@acme.com",
            Some("Jane Doe"),
            false,
            "https://api.aegis.dev",
        );
        let json = serde_json::to_string(&user).unwrap();
        let restored: ScimUser = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.user_name, user.user_name);
        assert!(!restored.active);
    }
}
