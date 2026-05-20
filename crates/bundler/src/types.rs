//! JSON-RPC IO types for `ethera_buildSignedUserOpsTx`.

use alloy::primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

fn bytes_is_empty(b: &Bytes) -> bool {
    b.is_empty()
}

/// Unpacked v0.7 `UserOperation` as it appears on the JSON-RPC wire.
///
/// Either the packed form (`initCode`, `paymasterAndData`) or the unpacked
/// form (`factory`/`factoryData`, `paymaster`/`paymasterVerificationGasLimit`/
/// `paymasterPostOpGasLimit`/`paymasterData`) may be provided.
/// [`crate::packing::pack`] handles both shapes.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOpV07 {
    pub sender: Address,
    pub nonce: U256,

    #[serde(default, skip_serializing_if = "bytes_is_empty")]
    pub init_code: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory: Option<Address>,
    #[serde(default, skip_serializing_if = "bytes_is_empty")]
    pub factory_data: Bytes,

    pub call_data: Bytes,

    pub call_gas_limit: U256,
    pub verification_gas_limit: U256,
    pub pre_verification_gas: U256,

    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster: Option<Address>,
    #[serde(default)]
    pub paymaster_verification_gas_limit: U256,
    #[serde(default)]
    pub paymaster_post_op_gas_limit: U256,
    #[serde(default, skip_serializing_if = "bytes_is_empty")]
    pub paymaster_data: Bytes,
    #[serde(default, skip_serializing_if = "bytes_is_empty")]
    pub paymaster_and_data: Bytes,

    #[serde(default)]
    pub signature: Bytes,
}

/// Second positional parameter of `ethera_buildSignedUserOpsTx`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOpts {
    pub chain_id: u64,
}

/// Successful response: a fully signed type-2 tx ready for `eth_sendRawTransaction`,
/// plus the per-op userOp hashes the caller can use to watch receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedTxResp {
    pub raw: Bytes,
    pub hash: B256,
    pub to: Address,
    pub chain_id: u64,
    pub gas: U256,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,
    pub user_op_hashes: Vec<B256>,
}
