//! `ethera-bundler` binary entrypoint: clap config → provider + signer → JSON-RPC server.

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use ethera_bundler_core::bundler::Bundler;
use ethera_bundler_core::config::BundlerConfig;
use ethera_bundler_core::provider::AlloyProvider;
use ethera_bundler_core::rpc::{BundlerRpc, EtheraBundlerApiServer};
use ethera_bundler_core::signer::{LocalSigner, Signer};
use jsonrpsee::server::Server;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = BundlerConfig::parse();
    tracing::info!(
        chain_id = cfg.chain_id,
        entrypoint = %cfg.entrypoint_address,
        rpc = %cfg.exec_rpc_url,
        listen = %cfg.listen_addr,
        "starting ethera-bundler",
    );

    let provider = Arc::new(
        AlloyProvider::new(&cfg.exec_rpc_url)
            .context("failed to construct execution-layer RPC provider")?,
    );
    let signer =
        Arc::new(LocalSigner::from_key(cfg.sequencer_key).context("failed to load sequencer key")?);
    tracing::info!(sequencer = %signer.address(), "sequencer signer loaded");

    let bundler = Arc::new(
        Bundler::new(cfg.clone(), provider, signer).context("failed to construct bundler")?,
    );
    let listen_addr = cfg.listen_addr;
    let rpc = BundlerRpc::new(bundler);

    // Permissive CORS so browser-based clients
    // can call the bundler from a different origin.
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any);
    let middleware = ServiceBuilder::new().layer(cors);

    let server = Server::builder()
        .set_http_middleware(middleware)
        .build(listen_addr)
        .await
        .with_context(|| format!("failed to bind JSON-RPC server on {listen_addr}"))?;
    let handle = server.start(rpc.into_rpc());
    tracing::info!(%listen_addr, "JSON-RPC server listening");

    tokio::signal::ctrl_c()
        .await
        .context("ctrl_c handler failed")?;
    tracing::info!("ctrl-c received; shutting down");
    handle.stop().ok();
    handle.stopped().await;
    Ok(())
}
