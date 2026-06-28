//! SMTP email delivery for the email alert channel.
//!
//! [`EmailMailer`] wraps a configured async SMTP transport plus the `From`
//! mailbox, so the [`crate::alert::AlertEngine`] can send a notification without
//! knowing anything about SMTP. The transport is built **once** at startup from
//! `[alerts.smtp]` config and the vault-held password, then shared (it is cheap
//! to clone and pools connections internally).

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use modelsentry_common::config::{SmtpConfig, SmtpSecurity};
use modelsentry_common::error::{ModelSentryError, Result};
use modelsentry_common::types::ApiKey;

/// A ready-to-use SMTP mailer: a configured transport plus the `From` mailbox.
#[derive(Clone)]
pub struct EmailMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl std::fmt::Debug for EmailMailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The transport holds credentials; never render it.
        f.debug_struct("EmailMailer")
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl EmailMailer {
    /// Build a mailer from `[alerts.smtp]` config and the vault-held SMTP
    /// `password` (the value stored under
    /// [`modelsentry_common::constants::credential::SMTP_PASSWORD`]).
    ///
    /// The password is required only when `cfg.username` is set; otherwise the
    /// relay is used unauthenticated.
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Email`] if the `from` address is unparseable, the
    /// transport cannot be constructed (e.g. TLS setup), or a username is
    /// configured without a vault password.
    pub fn from_config(cfg: &SmtpConfig, password: Option<&ApiKey>) -> Result<Self> {
        let from: Mailbox = cfg.from.parse().map_err(|e| {
            email_err(format!(
                "invalid [alerts.smtp] from address '{}': {e}",
                cfg.from
            ))
        })?;

        let builder = match cfg.security {
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                    .map_err(|e| email_err(format!("STARTTLS relay setup failed: {e}")))?
            }
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
                .map_err(|e| email_err(format!("TLS relay setup failed: {e}")))?,
            // Plaintext: localhost / testing only (documented in config).
            SmtpSecurity::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            }
        };

        let mut builder = builder.port(cfg.port);
        if let Some(username) = &cfg.username {
            let password = password.ok_or_else(|| {
                email_err(
                    "[alerts.smtp] username is set but no SMTP password is in the vault \
                     (set it under the 'smtp' key)"
                        .to_string(),
                )
            })?;
            builder = builder.credentials(Credentials::new(
                username.clone(),
                password.expose().to_string(),
            ));
        }

        Ok(Self {
            transport: builder.build(),
            from,
        })
    }

    /// Send a plain-text email. Logs nothing; the caller decides how to report
    /// failures (the alert engine logs at `WARN` and continues).
    ///
    /// # Errors
    ///
    /// [`ModelSentryError::Email`] if `to` is unparseable, the message cannot be
    /// built, or the SMTP transport rejects/​fails to deliver it.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let to: Mailbox = to
            .parse()
            .map_err(|e| email_err(format!("invalid recipient '{to}': {e}")))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| email_err(format!("could not build email message: {e}")))?;
        self.transport
            .send(message)
            .await
            .map_err(|e| email_err(format!("SMTP delivery failed: {e}")))?;
        Ok(())
    }
}

/// Construct an [`ModelSentryError::Email`] from a message.
fn email_err(message: String) -> ModelSentryError {
    ModelSentryError::Email { message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelsentry_common::config::SmtpConfig;

    fn cfg(security: SmtpSecurity) -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            from: "ModelSentry <alerts@example.com>".to_string(),
            username: None,
            security,
        }
    }

    #[test]
    fn builds_mailer_for_each_security_mode_without_credentials() {
        for security in [
            SmtpSecurity::StartTls,
            SmtpSecurity::Tls,
            SmtpSecurity::None,
        ] {
            let mailer = EmailMailer::from_config(&cfg(security), None);
            assert!(mailer.is_ok(), "{security:?} should build: {mailer:?}");
        }
    }

    #[test]
    fn rejects_unparseable_from_address() {
        let mut c = cfg(SmtpSecurity::StartTls);
        c.from = "not a valid mailbox".to_string();
        let err = EmailMailer::from_config(&c, None).unwrap_err();
        assert!(matches!(err, ModelSentryError::Email { .. }), "{err:?}");
        assert!(err.to_string().contains("from address"), "{err}");
    }

    #[test]
    fn username_without_password_is_an_error() {
        let mut c = cfg(SmtpSecurity::StartTls);
        c.username = Some("mailer@example.com".to_string());
        let err = EmailMailer::from_config(&c, None).unwrap_err();
        assert!(err.to_string().contains("vault"), "{err}");
    }

    #[test]
    fn username_with_password_builds() {
        let mut c = cfg(SmtpSecurity::StartTls);
        c.username = Some("mailer@example.com".to_string());
        let pass = ApiKey::new("hunter2".to_string());
        assert!(EmailMailer::from_config(&c, Some(&pass)).is_ok());
    }
}
