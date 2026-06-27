//! Daemon-local runtime tuning constants.
//!
//! Compile-time parameters for the HTTP server, scheduler, and per-run
//! execution live here rather than inline in `server.rs` / `main.rs` /
//! `scheduler.rs`, so every tunable is named and reviewable in one place.
//! Operator-facing settings (intervals, FPR, permissions) belong in
//! `config/default.toml`; these are implementation constants.

use std::time::Duration;

/// HTTP server limits.
pub mod server {
    /// Maximum accepted request body. Probe/alert payloads are small; anything
    /// larger is rejected with `413 Payload Too Large` to bound memory use.
    pub const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MiB

    /// Per-client rate limit (keyed by peer IP): the bucket holds up to
    /// `RATE_LIMIT_BURST` requests and refills one token every
    /// `RATE_LIMIT_REPLENISH_SECS` seconds, throttling credential brute-forcing
    /// while allowing a steady stream.
    pub const RATE_LIMIT_BURST: u32 = 100;
    /// Token refill period (seconds) for the per-client rate limiter.
    pub const RATE_LIMIT_REPLENISH_SECS: u64 = 1;
}

/// Outbound HTTP (alert/webhook delivery).
pub mod alert {
    /// HTTP timeout for the alert/webhook client (seconds).
    pub const HTTP_TIMEOUT_SECS: u64 = 10;
}

/// Scheduler / probe-execution tuning.
pub mod runtime {
    use super::Duration;

    /// How often the scheduler re-reads the probe set from the store to pick up
    /// probes added, edited, or deleted via the API/CLI after startup.
    pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

    /// Maximum number of prompts a single probe run executes concurrently.
    pub const PROBE_CONCURRENCY: usize = 4;
}
