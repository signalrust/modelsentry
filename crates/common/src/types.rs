use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generates a newtype ID wrapper over `Uuid` with standard derives and impls.
macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            #[must_use]
            pub fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(ProbeId);
define_id!(BaselineId);
define_id!(RunId);
define_id!(AlertRuleId);

/// Newtype over String. Never implements Display or Debug that shows the value.
#[derive(Clone)]
pub struct ApiKey(SecretString);

impl ApiKey {
    #[must_use]
    pub fn new(raw: String) -> Self {
        Self(raw.into())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([REDACTED])")
    }
}

impl Serialize for ApiKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("[REDACTED]")
    }
}

impl<'de> Deserialize<'de> for ApiKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::new(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ProbeId ---
    #[test]
    fn probe_id_new_is_unique() {
        let a = ProbeId::new();
        let b = ProbeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn probe_id_display_is_uuid_string() {
        let id = ProbeId::new();
        let s = id.to_string();
        assert_eq!(s.len(), 36);
        assert_eq!(s.chars().filter(|&c| c == '-').count(), 4);
    }

    #[test]
    fn probe_id_roundtrip_json() {
        let id = ProbeId::new();
        let json = serde_json::to_string(&id).unwrap();
        let id2: ProbeId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn probe_id_from_uuid() {
        let u = Uuid::new_v4();
        let id = ProbeId::from_uuid(u);
        assert_eq!(id.to_string(), u.to_string());
    }

    // --- BaselineId ---
    #[test]
    fn baseline_id_new_is_unique() {
        assert_ne!(BaselineId::new(), BaselineId::new());
    }

    #[test]
    fn baseline_id_roundtrip_json() {
        let id = BaselineId::new();
        let json = serde_json::to_string(&id).unwrap();
        let id2: BaselineId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn baseline_id_display_is_uuid_string() {
        let id = BaselineId::new();
        let s = id.to_string();
        assert_eq!(s.len(), 36);
    }

    // --- RunId ---
    #[test]
    fn run_id_new_is_unique() {
        assert_ne!(RunId::new(), RunId::new());
    }

    #[test]
    fn run_id_roundtrip_json() {
        let id = RunId::new();
        let json = serde_json::to_string(&id).unwrap();
        let id2: RunId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, id2);
    }

    // --- AlertRuleId ---
    #[test]
    fn alert_rule_id_new_is_unique() {
        assert_ne!(AlertRuleId::new(), AlertRuleId::new());
    }

    #[test]
    fn alert_rule_id_roundtrip_json() {
        let id = AlertRuleId::new();
        let json = serde_json::to_string(&id).unwrap();
        let id2: AlertRuleId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, id2);
    }

    // --- ApiKey ---
    #[test]
    fn api_key_debug_does_not_expose_secret() {
        let key = ApiKey::new("super-secret-key-12345".to_string());
        let debug = format!("{key:?}");
        assert!(!debug.contains("super-secret-key-12345"));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn api_key_serialize_does_not_expose_secret() {
        let key = ApiKey::new("super-secret-key-12345".to_string());
        let json = serde_json::to_string(&key).unwrap();
        assert!(!json.contains("super-secret-key-12345"));
        assert!(json.contains("REDACTED"));
    }

    #[test]
    fn api_key_expose_returns_raw_value() {
        let key = ApiKey::new("my-key-123".to_string());
        assert_eq!(key.expose(), "my-key-123");
    }
}
