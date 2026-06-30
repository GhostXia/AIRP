//! Tracing/logging setup. Honors `RUST_LOG`, defaulting to `info`.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialize the global tracing subscriber. Safe to call once at startup.
pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,airp_gateway=debug"));
    fmt().with_env_filter(filter).with_target(false).init();
}
