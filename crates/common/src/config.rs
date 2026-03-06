use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ModelSentryError, Result};

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub vault: VaultConfig,
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub alerts: AlertsConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct VaultConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct SchedulerConfig {
    pub default_interval_minutes: u32,
}

#[derive(Debug, Deserialize)]
pub struct AlertsConfig {
    pub drift_threshold_kl: f32,
    pub drift_threshold_cos: f32,
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
        Ok(())
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

    #[test]
    fn config_loads_from_default_toml() {
        let path = default_toml_path();
        let cfg = AppConfig::load(&path).expect("default.toml should load successfully");
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert!(cfg.server.port > 0);
    }

    #[test]
    fn config_validate_rejects_port_zero() {
        let cfg = AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
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
            },
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn config_validate_rejects_negative_threshold() {
        let cfg = AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 7740,
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
                drift_threshold_kl: -1.0,
                drift_threshold_cos: 0.1,
            },
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("kl"));
    }

    #[test]
    fn config_validate_rejects_negative_cos_threshold() {
        let cfg = AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 7740,
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
                drift_threshold_cos: -0.5,
            },
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("cos"));
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
