//! Copyright 2020-2022 Cognite AS
//!
//! A default configured proxy that should work for many common deployments.
//!
//! To configure a custom proxy, including custom feature flags, tracing etc, see `unleash_proxy_rust::main`.
#![warn(clippy::all)]

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Deployment specific:
    // TODO: Add tracing
    // TODO: Add prometheus
    // TODO: Add healthz route control
    env_logger::init();
    unleash_proxy::main().await
}
