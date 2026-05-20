//! Thin wrapper around an alloy EL provider for the calls the bundler needs.
//!
//! Centralizing them here keeps the bundler core mockable: tests inject a
//! [`MockEthProvider`]-style impl without touching real RPC.

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

/// All EL interactions the bundler needs. Mock for tests, real impl for prod.
#[async_trait]
pub trait EthProvider: Send + Sync + 'static {
    async fn chain_id(&self) -> Result<u64, ProviderError>;
    async fn base_fee(&self) -> Result<U256, ProviderError>;
    async fn max_priority_fee(&self) -> Result<U256, ProviderError>;
    async fn pending_nonce(&self, addr: Address) -> Result<u64, ProviderError>;

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
