pub mod alert;
pub mod baseline;
pub mod probe;

use anyhow::Result;

/// Build a shared `reqwest::Client` with a descriptive user-agent.
pub(crate) fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("modelsentry-cli/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
