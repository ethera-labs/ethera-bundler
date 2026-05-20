//! `jsonrpsee` RPC server module exposing `ethera_buildSignedUserOpsTx`.

use std::sync::Arc;

use jsonrpsee::core::async_trait;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::ErrorObjectOwned;

use crate::bundler::Bundler;
use crate::errors::classify;
use crate::provider::EthProvider;
use crate::signer::Signer;
use crate::types::{BuildOpts, SignedTxResp, UserOpV07};

#[rpc(server, namespace = "ethera")]
pub trait EtheraBundlerApi {
    #[method(name = "buildSignedUserOpsTx")]
    async fn build_signed_user_ops_tx(
        &self,
        user_ops: Vec<UserOpV07>,
        opts: BuildOpts,
    ) -> Result<SignedTxResp, ErrorObjectOwned>;
}

pub struct BundlerRpc<P: EthProvider + ?Sized, S: Signer + ?Sized> {
    inner: Arc<Bundler<P, S>>,
}

impl<P: EthProvider + ?Sized, S: Signer + ?Sized> BundlerRpc<P, S> {
    pub fn new(inner: Arc<Bundler<P, S>>) -> Self {
        Self { inner }
    }
}

impl<P: EthProvider + ?Sized, S: Signer + ?Sized> std::fmt::Debug for BundlerRpc<P, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BundlerRpc").finish_non_exhaustive()
    }
}

#[async_trait]
impl<P: EthProvider + ?Sized, S: Signer + ?Sized> EtheraBundlerApiServer for BundlerRpc<P, S> {
    async fn build_signed_user_ops_tx(
        &self,
        user_ops: Vec<UserOpV07>,
        opts: BuildOpts,
    ) -> Result<SignedTxResp, ErrorObjectOwned> {
        self.inner
            .build_signed_user_ops_tx(user_ops, opts)
            .await
            .map_err(|e| {
                let (code, data) = classify(&e);
                ErrorObjectOwned::owned(code, e.to_string(), Some(data))
            })
    }
}
