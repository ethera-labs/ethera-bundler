//! Thin wrapper around an alloy EL provider for the calls the bundler needs.
//!
//! Centralizing them here keeps the bundler core mockable: tests inject a
//! [`MockEthProvider`]-style impl without touching real RPC.

use std::time::{Duration, Instant};

use alloy::eips::BlockNumberOrTag;
use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, Bytes, B256, U256};
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::state::{AccountOverride, StateOverride};
use alloy::rpc::types::TransactionRequest;
use async_trait::async_trait;

use crate::contracts::{
    IEntryPoint, IEntryPointSimulations, PackedUserOperation, ValidationResult,
};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("rpc transport error: {0}")]
    Transport(String),
    #[error("eth_call returned no data")]
    EmptyCall,
    #[error("abi decode failed: {0}")]
    Decode(String),
}

/// Outcome of awaiting a submitted transaction's receipt.
#[derive(Debug, Clone)]
pub struct ReceiptInfo {
    pub tx_hash: B256,
    pub block_number: u64,
    pub success: bool,
}

/// All EL interactions the bundler needs. Mock for tests, real impl for prod.
#[async_trait]
pub trait EthProvider: Send + Sync + 'static {
    async fn chain_id(&self) -> Result<u64, ProviderError>;
    async fn base_fee(&self) -> Result<U256, ProviderError>;
    async fn max_priority_fee(&self) -> Result<U256, ProviderError>;
    async fn pending_nonce(&self, addr: Address) -> Result<u64, ProviderError>;

    /// EOA balance at the latest block. Used by the startup probe to surface
    /// an unfunded sequencer key before serving any traffic.
    async fn balance(&self, addr: Address) -> Result<U256, ProviderError>;

    /// Deployed code at `addr` at the latest block. An empty return means the
    /// address holds no code (EOA or non-existent contract).
    async fn get_code(&self, addr: Address) -> Result<Bytes, ProviderError>;

    async fn balance_of(
        &self,
        entrypoint: Address,
        account: Address,
    ) -> Result<U256, ProviderError>;

    async fn user_op_hash(
        &self,
        entrypoint: Address,
        op: PackedUserOperation,
    ) -> Result<B256, ProviderError>;

    /// Estimate gas for a sequencer-signed `handleOps` call.
    async fn estimate_handle_ops_gas(
        &self,
        from: Address,
        entrypoint: Address,
        call_data: Bytes,
    ) -> Result<u64, ProviderError>;

    /// Broadcast an EIP-2718-encoded signed transaction and return the
    /// resulting tx hash once the node has accepted it into the mempool.
    async fn send_raw_transaction(&self, raw: Bytes) -> Result<B256, ProviderError>;

    /// Poll for the receipt of a submitted transaction until it lands or the
    /// timeout elapses. Returns `None` on timeout so the caller can surface
    /// a domain-specific error instead of a transport error.
    async fn wait_for_receipt(
        &self,
        hash: B256,
        timeout: Duration,
    ) -> Result<Option<ReceiptInfo>, ProviderError>;

    /// Replay an `eth_call` against the latest block, returning the revert
    /// bytes when the call reverts. Used after a failed receipt to recover
    /// the `FailedOp*` reason that the node already knew at mining time.
    async fn try_call_for_revert(
        &self,
        from: Address,
        to: Address,
        call_data: Bytes,
    ) -> Result<Option<Bytes>, ProviderError>;

    /// Simulate `IEntryPointSimulations.simulateValidation(op)` via
    /// `eth_call` with a state-override that swaps `EntryPoint`'s runtime code
    /// for the simulations bytecode. v0.7 moved this method out of the main
    /// contract; this trick is the canonical workaround.
    async fn simulate_validation(
        &self,
        entrypoint: Address,
        simulations_code: Bytes,
        op: PackedUserOperation,
    ) -> Result<ValidationResult, ProviderError>;
}

pub struct AlloyProvider {
    inner: RootProvider,
}

impl std::fmt::Debug for AlloyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlloyProvider").finish_non_exhaustive()
    }
}

impl AlloyProvider {
    pub fn new(rpc_url: &str) -> Result<Self, ProviderError> {
        let url = rpc_url
            .parse()
            .map_err(|e: url::ParseError| ProviderError::Transport(e.to_string()))?;
        Ok(Self {
            inner: RootProvider::new_http(url),
        })
    }
}

#[async_trait]
impl EthProvider for AlloyProvider {
    async fn chain_id(&self) -> Result<u64, ProviderError> {
        self.inner
            .get_chain_id()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn base_fee(&self) -> Result<U256, ProviderError> {
        let block = self
            .inner
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?
            .ok_or_else(|| ProviderError::Transport("latest block missing".into()))?;
        Ok(U256::from(
            block.header.base_fee_per_gas.unwrap_or_default(),
        ))
    }

    async fn max_priority_fee(&self) -> Result<U256, ProviderError> {
        let tip = self
            .inner
            .get_max_priority_fee_per_gas()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(U256::from(tip))
    }

    async fn pending_nonce(&self, addr: Address) -> Result<u64, ProviderError> {
        self.inner
            .get_transaction_count(addr)
            .pending()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn balance(&self, addr: Address) -> Result<U256, ProviderError> {
        self.inner
            .get_balance(addr)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn get_code(&self, addr: Address) -> Result<Bytes, ProviderError> {
        self.inner
            .get_code_at(addr)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn balance_of(
        &self,
        entrypoint: Address,
        account: Address,
    ) -> Result<U256, ProviderError> {
        let ep = IEntryPoint::new(entrypoint, &self.inner);
        ep.balanceOf(account)
            .call()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn user_op_hash(
        &self,
        entrypoint: Address,
        op: PackedUserOperation,
    ) -> Result<B256, ProviderError> {
        let ep = IEntryPoint::new(entrypoint, &self.inner);
        ep.getUserOpHash(op)
            .call()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn estimate_handle_ops_gas(
        &self,
        from: Address,
        entrypoint: Address,
        call_data: Bytes,
    ) -> Result<u64, ProviderError> {
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(entrypoint)
            .with_input(call_data);
        self.inner
            .estimate_gas(tx)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    async fn send_raw_transaction(&self, raw: Bytes) -> Result<B256, ProviderError> {
        let pending = self
            .inner
            .send_raw_transaction(&raw)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(*pending.tx_hash())
    }

    async fn wait_for_receipt(
        &self,
        hash: B256,
        timeout: Duration,
    ) -> Result<Option<ReceiptInfo>, ProviderError> {
        let deadline = Instant::now() + timeout;
        let mut backoff = Duration::from_millis(200);
        loop {
            if let Some(r) = self
                .inner
                .get_transaction_receipt(hash)
                .await
                .map_err(|e| ProviderError::Transport(e.to_string()))?
            {
                return Ok(Some(ReceiptInfo {
                    tx_hash: hash,
                    block_number: r.block_number.unwrap_or_default(),
                    success: r.status(),
                }));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let sleep = backoff.min(deadline - now);
            tokio::time::sleep(sleep).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }

    async fn try_call_for_revert(
        &self,
        from: Address,
        to: Address,
        call_data: Bytes,
    ) -> Result<Option<Bytes>, ProviderError> {
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_input(call_data);
        match self.inner.call(tx).await {
            Ok(_) => Ok(None),
            Err(err) => Ok(err
                .as_error_resp()
                .and_then(|payload| payload.as_revert_data())),
        }
    }

    async fn simulate_validation(
        &self,
        entrypoint: Address,
        simulations_code: Bytes,
        op: PackedUserOperation,
    ) -> Result<ValidationResult, ProviderError> {
        use alloy::sol_types::SolCall;

        let call = IEntryPointSimulations::simulateValidationCall { userOp: op };
        let data = Bytes::from(call.abi_encode());

        let mut overrides = StateOverride::default();
        overrides.insert(
            entrypoint,
            AccountOverride {
                code: Some(simulations_code),
                ..AccountOverride::default()
            },
        );

        let tx = TransactionRequest::default()
            .with_to(entrypoint)
            .with_input(data);
        let result = self
            .inner
            .call(tx)
            .overrides(overrides)
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;

        if result.is_empty() {
            return Err(ProviderError::EmptyCall);
        }
        IEntryPointSimulations::simulateValidationCall::abi_decode_returns(&result)
            .map_err(|e| ProviderError::Decode(e.to_string()))
    }
}
