//! Read-only discovery shims used by wallet SDKs and tooling to identify
//! the bundler before sending a `UserOp`:
//!
//! * `eth_chainId` ([EIP-695]) - confirm the bundler is on the expected chain.
//! * `eth_supportedEntryPoints` ([ERC-4337]) - discover the `EntryPoint` to
//!   target when building `UserOps`.
//! * `web3_clientVersion` - human-readable build identifier for logs and
//!   bug reports.
//!
//! This is intentionally a partial surface: the full ERC-4337 RPC
//! (`eth_sendUserOperation`, `eth_estimateUserOperationGas`, ...) is not
//! exposed because this bundler does not run a public mempool - `UserOps`
//! arrive pre-batched through [`crate::rpc`].
//!
//! [EIP-695]: https://eips.ethereum.org/EIPS/eip-695
//! [ERC-4337]: https://eips.ethereum.org/EIPS/eip-4337#rpc-methods-eth-namespace

use alloy::primitives::Address;
use jsonrpsee::core::async_trait;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;

#[rpc(server, namespace = "eth")]
pub trait EthDiscoveryApi {
    /// EIP-695: hex-encoded chain id with `0x` prefix and no leading zeros.
    #[method(name = "chainId")]
    async fn chain_id(&self) -> Result<String, ErrorObjectOwned>;

    /// ERC-4337: `EntryPoint` contracts this bundler will build transactions
    /// against. Today always a single-element vector matching
    /// `BundlerConfig::entrypoint_address`.
    #[method(name = "supportedEntryPoints")]
    async fn supported_entry_points(&self) -> Result<Vec<Address>, ErrorObjectOwned>;
}

#[rpc(server, namespace = "web3")]
pub trait Web3DiscoveryApi {
    /// Identifier of the form `ethera-bundler/v<semver>`. Stable across the
    /// lifetime of a binary; safe to log and to include in error reports.
    #[method(name = "clientVersion")]
    async fn client_version(&self) -> Result<String, ErrorObjectOwned>;
}

#[derive(Debug, Clone)]
pub struct DiscoveryRpc {
    chain_id: u64,
    entrypoint: Address,
    client_version: String,
}

impl DiscoveryRpc {
    pub fn new(chain_id: u64, entrypoint: Address) -> Self {
        Self {
            chain_id,
            entrypoint,
            client_version: format!("ethera-bundler/v{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[async_trait]
impl EthDiscoveryApiServer for DiscoveryRpc {
    async fn chain_id(&self) -> Result<String, ErrorObjectOwned> {
        Ok(format!("0x{:x}", self.chain_id))
    }

    async fn supported_entry_points(&self) -> Result<Vec<Address>, ErrorObjectOwned> {
        Ok(vec![self.entrypoint])
    }
}

#[async_trait]
impl Web3DiscoveryApiServer for DiscoveryRpc {
    async fn client_version(&self) -> Result<String, ErrorObjectOwned> {
        Ok(self.client_version.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const EP_V07: Address = address!("0000000071727De22E5E9d8BAf0edAc6f37da032");

    fn fixture() -> DiscoveryRpc {
        DiscoveryRpc::new(100_003, EP_V07)
    }

    #[tokio::test]
    async fn chain_id_is_hex_lowercase_no_leading_zeros() {
        // EIP-695 forbids leading zeros: `100_003 == 0x186a3`, not `0x0186a3`.
        assert_eq!(fixture().chain_id().await.unwrap(), "0x186a3");
    }

    #[tokio::test]
    async fn supported_entry_points_returns_configured() {
        assert_eq!(
            fixture().supported_entry_points().await.unwrap(),
            vec![EP_V07]
        );
    }

    #[tokio::test]
    async fn client_version_includes_package_version() {
        assert!(fixture()
            .client_version()
            .await
            .unwrap()
            .starts_with("ethera-bundler/v"));
    }
}
