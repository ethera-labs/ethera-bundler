//! Sequencer-tx signing. Trait-based so a KMS / `Web3Signer` backend can replace
//! the local key without touching the bundler core.

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::network::TxSignerSync;
use alloy::primitives::{Address, B256};
use alloy::signers::local::PrivateKeySigner;
use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("invalid sequencer key: {0}")]
    InvalidKey(String),
    #[error("signing failed: {0}")]
    Signing(String),
}

/// Signs the outer EIP-1559 transaction that wraps `EntryPoint.handleOps`.
#[async_trait]
pub trait Signer: Send + Sync + 'static {
    fn address(&self) -> Address;
    async fn sign_eip1559(&self, tx: TxEip1559) -> Result<TxEnvelope, SignerError>;
}

#[derive(Debug, Clone)]
pub struct LocalSigner {
    inner: PrivateKeySigner,
}

impl LocalSigner {
    pub fn from_key(key: B256) -> Result<Self, SignerError> {
        let inner = PrivateKeySigner::from_bytes(&key)
            .map_err(|e| SignerError::InvalidKey(e.to_string()))?;
        Ok(Self { inner })
    }
}

#[async_trait]
impl Signer for LocalSigner {
    fn address(&self) -> Address {
        self.inner.address()
    }

    async fn sign_eip1559(&self, mut tx: TxEip1559) -> Result<TxEnvelope, SignerError> {
        let signature = self
            .inner
            .sign_transaction_sync(&mut tx)
            .map_err(|e| SignerError::Signing(e.to_string()))?;
        Ok(TxEnvelope::Eip1559(tx.into_signed(signature)))
    }
}
