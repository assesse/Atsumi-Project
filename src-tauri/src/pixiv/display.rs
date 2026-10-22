//! Local discovery preference, separate from the server's access decisions.
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) struct DisplayPolicy {
    authenticated: bool,
    safe_only: bool,
}

impl DisplayPolicy {
    pub fn new(authenticated: bool, safe_only: bool) -> Self {
        Self {
            authenticated,
            safe_only: safe_only || !authenticated,
        }
    }

    pub fn query_mode(self) -> &'static str {
        if self.safe_only {
            "safe"
        } else {
            "all"
        }
    }

    pub fn excludes(self, row: &Value) -> bool {
        // A rating is not an access denial. The verified session's official
        // response decides eligibility; masked/private/error responses do not.
        !valid_access_markers(row)
            || super::number(&row["restrict"]) > 0
            || (!self.authenticated && row["isLoginOnly"].as_bool() == Some(true))
            || (self.safe_only
                && (super::number(&row["xRestrict"]) > 0
                    || super::number(&row["sl"]) >= 4
                    || row["illust_content"]["sexual"].as_bool() == Some(true)))
    }
}

fn valid_access_markers(row: &Value) -> bool {
    for (field, maximum) in [("xRestrict", 2), ("restrict", 2), ("sl", 6)] {
        if row
            .get(field)
            .is_some_and(|value| value.as_u64().is_none_or(|value| value > maximum))
        {
            return false;
        }
    }
    for field in ["isMasked", "isLoginOnly"] {
        if row.get(field).is_some_and(|value| !value.is_boolean()) {
            return false;
        }
    }
    if let Some(content) = row.get("illust_content") {
        if !content.is_object()
            || content
                .get("sexual")
                .is_some_and(|value| !value.is_boolean())
        {
            return false;
        }
    }
    row["isMasked"].as_bool() != Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preference_and_verified_account_choose_query_mode() {
        assert_eq!(DisplayPolicy::new(true, false).query_mode(), "all");
        for (authenticated, safe_only) in [(false, false), (false, true), (true, true)] {
            assert_eq!(
                DisplayPolicy::new(authenticated, safe_only).query_mode(),
                "safe"
            );
        }
    }

    #[test]
    fn rating_preference_is_not_a_private_or_masked_access_override() {
        for rating in [
            json!({"xRestrict":1}),
            json!({"xRestrict":2}),
            json!({"sl":4}),
            json!({"illust_content":{"sexual":true}}),
        ] {
            assert!(!DisplayPolicy::new(true, false).excludes(&rating));
            assert!(DisplayPolicy::new(true, true).excludes(&rating));
            assert!(DisplayPolicy::new(false, false).excludes(&rating));
        }
        for denied in [
            json!({"restrict":1}),
            json!({"isMasked":true}),
            json!({"xRestrict":"1"}),
            json!({"isMasked":"false"}),
        ] {
            for safe_only in [false, true] {
                assert!(DisplayPolicy::new(true, safe_only).excludes(&denied));
            }
        }
        // Safe-only must not accidentally discard ordinary login-only work.
        assert!(!DisplayPolicy::new(true, true).excludes(&json!({"isLoginOnly":true})));
        assert!(DisplayPolicy::new(false, false).excludes(&json!({"isLoginOnly":true})));
    }
}
