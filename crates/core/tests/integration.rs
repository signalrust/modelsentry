// Integration tests for the ModelSentry core crate.
//
// #[path] is required because integration test binaries resolve `mod`
// declarations relative to the tests/ directory, not tests/integration/.
#[path = "integration/probe_lifecycle.rs"]
mod probe_lifecycle;
#[path = "integration/drift_detection.rs"]
mod drift_detection;
#[path = "integration/alert_fire.rs"]
mod alert_fire;
