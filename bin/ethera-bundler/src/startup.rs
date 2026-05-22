//! Pre-flight probes against the configured execution layer.
//!
//! Catches the failure modes that would otherwise only surface on the first
//! `ethera_buildSignedUserOpsTx` call (and produce confusing errors there):
//!
//! * RPC reachable and on the expected `chain_id` (mismatched chain = every
//!   signed tx invalid, no client-visible signal until first call).
//! * `EntryPoint` contract is actually deployed at the configured address.
//! * Sequencer EOA exists and has enough balance to pay the outer tx — logged
//!   as a warning rather than a hard failure, since funding can lag deploy.
//!
//! Failures here abort `main` before the JSON-RPC server binds; the operator
//! sees a single context-rich error instead of every `UserOp` returning `-32603`.
//!
//! Run on a generic `EthProvider` so the binary stays mockable for tests in
//! the future.

use alloy::primitives::{Address, U256};
use anyhow::{bail, Context, Result};
use ethera_bundler_core::provider::EthProvider;

const ONE_ETH_WEI: u128 = 1_000_000_000_000_000_000;

pub(crate) async fn probe(
    provider: &dyn EthProvider,
    chain_id: u64,
    entrypoint: Address,
    sequencer: Address,
) -> Result<()> {
    let actual_chain_id = provider
        .chain_id()
        .await
        .context("eth_chainId probe failed")?;
    if actual_chain_id != chain_id {
        bail!(
            "chain id mismatch: config={chain_id}, rpc={actual_chain_id} — bundler would sign \
             txs the chain rejects"
        );
    }
    tracing::info!(chain_id, "chain id probe ok");

    let code = provider
        .get_code(entrypoint)
        .await
        .context("eth_getCode(entrypoint) probe failed")?;
    if code.is_empty() {
        bail!("EntryPoint {entrypoint} has no code on chain {chain_id}");
    }
    tracing::info!(
        %entrypoint,
        code_bytes = code.len(),
        "EntryPoint code probe ok",
    );

    let balance = provider
        .balance(sequencer)
        .await
        .context("eth_getBalance(sequencer) probe failed")?;
    let floor = U256::from(ONE_ETH_WEI);
    if balance < floor {
        tracing::warn!(
            %sequencer,
            balance_wei = %balance,
            "sequencer balance below 1 ETH equivalent — handleOps may run out of gas funds",
        );
    } else {
        tracing::info!(%sequencer, balance_wei = %balance, "sequencer balance ok");
    }

    Ok(())
}
