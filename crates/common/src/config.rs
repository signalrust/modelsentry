use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ModelSentryError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub vault: VaultConfig,
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub alerts: AlertsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Request timeout in seconds (default: 30).
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Allowed CORS origin. Defaults to `http://localhost:5173` (Vite dev).
    /// Set to `"*"` to allow all origins (not recommended for production).
    #[serde(default = "default_cors_origin")]
    pub cors_origin: String,
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_cors_origin() -> String {
    "http://localhost:5173".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct VaultConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerConfig {
    pub default_interval_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertsConfig {
    pub drift_threshold_kl: f32,
    pub drift_threshold_cos: f32,
    /// Allow webhook/Slack alert targets that resolve to private, loopback, or
    /// link-local addresses. Defaults to `false` (SSRF-safe). Enable only for
    /// trusted internal receivers.
    #[serde(default)]
    pub allow_private_webhook_targets: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub openai: OpenAiConfig,
    #[serde(default)]
    pub anthropic: AnthropicConfig,
    #[serde(default)]
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_openai_model")]
    pub model: String,
    #[serde(default = "default_openai_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_openai_embedding_dim")]
    pub embedding_dim: usize,
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            model: default_openai_model(),
            embedding_model: default_openai_embedding_model(),
            embedding_dim: default_openai_embedding_dim(),
            base_url: default_openai_base_url(),
            max_tokens: default_max_tokens(),
        }
    }
}

fn default_openai_model() -> String {
    "gpt-5.4".to_string()
}
fn default_openai_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}
fn default_openai_embedding_dim() -> usize {
    1536
}
fn default_openai_base_url() -> String {
    "https://api.openai.com".to_string()
}
fn default_max_tokens() -> u32 {
    1024
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_anthropic_model")]
    pub model: String,
    #[serde(default = "default_anthropic_base_url")]
    pub base_url: String,
    #[serde(default = "default_anthropic_api_version")]
    pub api_version: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            model: default_anthropic_model(),
            base_url: default_anthropic_base_url(),
            api_version: default_anthropic_api_version(),
            max_tokens: default_max_tokens(),
        }
    }
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-6".to_string()
}
fn default_anthropic_base_url() -> String {
    "https://api.anthropic.com".to_string()
}
fn default_anthropic_api_version() -> String {
    "2023-06-01".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_model")]
    pub model: String,
    #[serde(default = "default_ollama_base_url")]
    pub base_url: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            model: default_ollama_model(),
            base_url: default_ollama_base_url(),
        }
    }
}

fn default_ollama_model() -> String {
    "llama3".to_string()
}
fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

/// Optional API-key authentication for the daemon HTTP API.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    /// When `true`, all `/api/` endpoints require a valid bearer token or
    /// `X-Api-Key` header matching one of `api_keys`.
    #[serde(default)]
    pub enabled: bool,
    /// Accepted API keys. At least one must be present when `enabled = true`.
    #[serde(default)]
    pub api_keys: Vec<String>,
}

impl AppConfig {
    /// Load from a TOML file path.
    ///
    /// # Errors
    ///
    /// Returns `ModelSentryError::Config` if the file cannot be read, parsed, or fails validation.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| ModelSentryError::Config {
            message: format!("failed to read config file '{}': {e}", path.display()),
        })?;
        let config: Self = toml::from_str(&content).map_err(|e| ModelSentryError::Config {
            message: format!("failed to parse config: {e}"),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Validate all fields after deserializing.
    ///
    /// # Errors
    ///
    /// Returns `ModelSentryError::Config` if any field has an invalid value.
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            return Err(ModelSentryError::Config {
                message: "server.port must not be 0".to_string(),
            });
        }
        if self.alerts.drift_threshold_kl < 0.0 {
            return Err(ModelSentryError::Config {
                message: "alerts.drift_threshold_kl must be non-negative".to_string(),
            });
        }
        if self.alerts.drift_threshold_cos < 0.0 {
            return Err(ModelSentryError::Config {
                message: "alerts.drift_threshold_cos must be non-negative".to_string(),
            });
        }
        if self.auth.enabled && self.auth.api_keys.is_empty() {
            return Err(ModelSentryError::Config {
                message: "auth.api_keys must contain at least one key when auth is enabled"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Insecure-but-valid configuration choices worth warning about at startup.
    ///
    /// These never fail validation (the daemon must still run), but the
    /// operator should see them — e.g. an unauthenticated API, an
    /// unauthenticated API on a non-loopback bind, or fully permissive CORS.
    #[must_use]
    pub fn security_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let loopback = matches!(self.server.host.as_str(), "127.0.0.1" | "localhost" | "::1");

        if !self.auth.enabled {
            if loopback {
                warnings.push(
                    "API authentication is disabled ([auth] enabled = false) — anything that \
                     can reach the port has full access."
                        .to_string(),
                );
            } else {
                warnings.push(format!(
                    "API authentication is disabled AND the server binds a non-loopback address \
                     ({}) — the API is exposed to the network with no auth. Enable [auth] or bind \
                     to 127.0.0.1.",
                    self.server.host
                ));
            }
        }
        if self.server.cors_origin == "*" {
            warnings.push(
                "CORS is fully permissive ([server] cors_origin = \"*\") — any website can call \
                 this API from a browser. Set a specific origin for production."
                    .to_string(),
            );
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_toml_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("config")
            .join("default.toml")
    }

    /// Build a valid test config with all default values.
    fn test_config() -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 7740,
                timeout_secs: 30,
                cors_origin: "http://localhost:5173".to_string(),
            },
            vault: VaultConfig {
                path: PathBuf::from(".vault"),
            },
            database: DatabaseConfig {
                path: PathBuf::from(".db"),
            },
            scheduler: SchedulerConfig {
                default_interval_minutes: 60,
            },
            alerts: AlertsConfig {
                drift_threshold_kl: 0.1,
                drift_threshold_cos: 0.1,
                allow_private_webhook_targets: false,
            },
            providers: ProvidersConfig::default(),
            auth: AuthConfig::default(),
        }
    }

    #[test]
    fn config_loads_from_default_toml() {
        let path = default_toml_path();
        let cfg = AppConfig::load(&path).expect("default.toml should load successfully");
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert!(cfg.server.port > 0);
    }

    #[test]
    fn config_validate_rejects_port_zero() {
        let mut cfg = test_config();
        cfg.server.port = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn config_validate_rejects_negative_threshold() {
        let mut cfg = test_config();
        cfg.alerts.drift_threshold_kl = -1.0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("kl"));
    }

    #[test]
    fn config_validate_rejects_negative_cos_threshold() {
        let mut cfg = test_config();
        cfg.alerts.drift_threshold_cos = -0.5;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("cos"));
    }

    #[test]
    fn config_validate_rejects_auth_enabled_without_keys() {
        let mut cfg = test_config();
        cfg.auth.enabled = true;
        cfg.auth.api_keys = vec![];
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("api_keys"));
    }

    #[test]
    fn config_validate_accepts_auth_enabled_with_keys() {
        let mut cfg = test_config();
        cfg.auth.enabled = true;
        cfg.auth.api_keys = vec!["secret-key".to_string()];
        cfg.validate().expect("should be valid");
    }

    #[test]
    fn security_warnings_flags_disabled_auth_on_loopback() {
        let mut cfg = test_config();
        cfg.auth.enabled = false;
        cfg.server.host = "127.0.0.1".to_string();
        cfg.server.cors_origin = "http://localhost:5173".to_string();
        let w = cfg.security_warnings();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("authentication is disabled"));
    }

    #[test]
    fn security_warnings_escalates_disabled_auth_on_public_bind() {
        let mut cfg = test_config();
        cfg.auth.enabled = false;
        cfg.server.host = "0.0.0.0".to_string();
        let w = cfg.security_warnings();
        assert!(
            w.iter().any(|m| m.contains("non-loopback")),
            "expected non-loopback escalation, got {w:?}"
        );
    }

    #[test]
    fn security_warnings_flags_permissive_cors() {
        let mut cfg = test_config();
        cfg.server.cors_origin = "*".to_string();
        let w = cfg.security_warnings();
        assert!(
            w.iter().any(|m| m.contains("CORS is fully permissive")),
            "{w:?}"
        );
    }

    #[test]
    fn security_warnings_silent_for_secure_config() {
        let mut cfg = test_config();
        cfg.auth.enabled = true;
        cfg.auth.api_keys = vec!["secret-key".to_string()];
        cfg.server.host = "127.0.0.1".to_string();
        cfg.server.cors_origin = "http://localhost:5173".to_string();
        assert!(cfg.security_warnings().is_empty());
    }

    #[test]
    fn missing_required_field_returns_config_error() {
        // [server] block without port — TOML parse will fail on deserialization
        let toml_str = r#"
[server]
host = "127.0.0.1"
"#;
        let result: std::result::Result<AppConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }
}
