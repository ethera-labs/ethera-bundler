//! Bundler error type and its mapping to ERC-4337 JSON-RPC error codes.
//!
//! Reference: [ERC-4337 Bundler RPC error codes][spec].
//!
//! [spec]: https://eips.ethereum.org/EIPS/eip-4337#error-codes

use alloy::primitives::Bytes;
use alloy::sol_types::{SolError, SolValue};
use serde_json::{json, Value};

use crate::contracts::IEntryPoint;
use crate::packing::PackError;
use crate::provider::ProviderError;
use crate::signer::SignerError;
use crate::validator::ValidationError;

/// Standard ERC-4337 bundler RPC error codes (subset we can produce).
pub mod codes {
    /// Standard JSON-RPC: invalid params / pre-validation rejection.
    pub const INVALID_PARAMS: i32 = -32602;
    /// Standard JSON-RPC: internal error.
    pub const INTERNAL_ERROR: i32 = -32603;
    /// Transaction rejected by `EntryPoint`'s `simulateValidation` (`AA2x` family).
    pub const SIMULATE_VALIDATION_REJECTED: i32 = -32500;
    /// Transaction rejected by paymaster's `validatePaymasterUserOp` (`AA3x` family).
    pub const PAYMASTER_REJECTED: i32 = -32501;
    /// `UserOperation` out of valid time range (`validAfter` / `validUntil`).
    pub const OUT_OF_TIME_RANGE: i32 = -32503;
    /// Transaction reverted in account validation (account threw, not returned a sentinel).
    pub const ACCOUNT_VALIDATION_REVERTED: i32 = -32521;
}

#[derive(Debug, thiserror::Error)]
pub enum BundlerError {
    #[error("wrong chainId: expected {expected}, got {got}")]
    WrongChainId { expected: u64, got: u64 },

    #[error("invalid userOps batch: {0}")]
    InvalidBatch(&'static str),

    #[error("batch too large: {got} > {max}")]
    BatchTooLarge { got: usize, max: usize },

    #[error("op {op_index}: {reason}")]
    InvalidUserOp { op_index: usize, reason: String },

    #[error("op {op_index}: priority fee {got} below bundler minimum {min}")]
    PriorityFeeTooLow {
        op_index: usize,
        got: String,
        min: String,
    },

    #[error("op {op_index}: maxFeePerGas {fee_cap} below baseFee+tip {required}")]
    FeeCapTooLow {
        op_index: usize,
        fee_cap: String,
        required: String,
    },

    #[error(
        "op {op_index}: insufficient deposit on {sponsor}: required {required}, deposit {deposit}"
    )]
    InsufficientDeposit {
        op_index: usize,
        sponsor: String,
        required: String,
        deposit: String,
    },

    #[error("bundler is shutting down")]
    Cancelled,

    #[error("receipt for {tx_hash:?} not found within timeout")]
    ReceiptTimeout { tx_hash: alloy::primitives::B256 },

    #[error("handleOps reverted on-chain: {reason}")]
    HandleOpsReverted {
        tx_hash: alloy::primitives::B256,
        block_number: u64,
        reason: HandleOpsRevertReason,
    },

    #[error(transparent)]
    Pack(#[from] PackError),

    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Signer(#[from] SignerError),
}

/// JSON-RPC error code + `data` payload for [`BundlerError`].
///
/// Mapping mirrors the [ERC-4337 spec][spec]:
/// * `-32602` for any pre-EntryPoint validation failure (bad input, fee policy, gas overflow).
/// * `-32500` for `EntryPoint` / account simulation rejections (`AA21 didn't pay prefund`, etc.).
/// * `-32501` for paymaster signature failures.
/// * `-32503` for out-of-time-range.
/// * `-32521` reserved for account-revert; we surface it via aggregator-unsupported for now.
/// * `-32603` for everything else (transport, signing, ABI decode).
///
/// [spec]: https://eips.ethereum.org/EIPS/eip-4337#error-codes
#[must_use]
pub fn classify(err: &BundlerError) -> (i32, Value) {
    match err {
        BundlerError::WrongChainId { expected, got } => (
            codes::INVALID_PARAMS,
            json!({ "reason": "wrongChainId", "expected": expected, "got": got }),
        ),
        BundlerError::InvalidBatch(reason) => (codes::INVALID_PARAMS, json!({ "reason": reason })),
        BundlerError::BatchTooLarge { got, max } => (
            codes::INVALID_PARAMS,
            json!({ "reason": "batchTooLarge", "got": got, "max": max }),
        ),
        BundlerError::InvalidUserOp { op_index, reason } => (
            codes::INVALID_PARAMS,
            json!({ "opIndex": op_index, "reason": reason }),
        ),
        BundlerError::PriorityFeeTooLow { op_index, got, min } => (
            codes::INVALID_PARAMS,
            json!({ "opIndex": op_index, "reason": "priorityFeeTooLow", "got": got, "min": min }),
        ),
        BundlerError::FeeCapTooLow {
            op_index,
            fee_cap,
            required,
        } => (
            codes::INVALID_PARAMS,
            json!({
                "opIndex": op_index,
                "reason": "maxFeePerGasBelowBaseFee",
                "maxFeePerGas": fee_cap,
                "required": required,
            }),
        ),
        BundlerError::InsufficientDeposit {
            op_index,
            sponsor,
            required,
            deposit,
        } => (
            codes::SIMULATE_VALIDATION_REJECTED,
            json!({
                "opIndex": op_index,
                "reason": "AA21 didn't pay prefund",
                "sponsor": sponsor,
                "required": required,
                "deposit": deposit,
            }),
        ),
        BundlerError::Cancelled => (
            codes::INTERNAL_ERROR,
            json!({ "reason": "shutdown in progress" }),
        ),
        BundlerError::ReceiptTimeout { tx_hash } => (
            codes::INTERNAL_ERROR,
            json!({ "reason": "receiptTimeout", "txHash": format!("{tx_hash:?}") }),
        ),
        BundlerError::HandleOpsReverted {
            tx_hash,
            block_number,
            reason,
        } => {
            let (code, payload) = reason.classify();
            let mut payload = payload;
            payload["txHash"] = json!(format!("{tx_hash:?}"));
            payload["blockNumber"] = json!(block_number);
            (code, payload)
        }
        BundlerError::Pack(e) => (codes::INVALID_PARAMS, json!({ "reason": e.to_string() })),
        BundlerError::Validation(e) => match e {
            ValidationError::AccountSignatureFailed => (
                codes::SIMULATE_VALIDATION_REJECTED,
                json!({ "reason": "AA24 signature error" }),
            ),
            ValidationError::PaymasterSignatureFailed => (
                codes::PAYMASTER_REJECTED,
                json!({ "reason": "AA34 signature error" }),
            ),
            ValidationError::OutOfTimeRange {
                now,
                a_after,
                a_until,
                p_after,
                p_until,
            } => (
                codes::OUT_OF_TIME_RANGE,
                json!({
                    "reason": "AA22 expired or not due",
                    "now": now,
                    "accountValidAfter": a_after,
                    "accountValidUntil": a_until,
                    "paymasterValidAfter": p_after,
                    "paymasterValidUntil": p_until,
                }),
            ),
            ValidationError::UnsupportedAggregator => (
                codes::ACCOUNT_VALIDATION_REVERTED,
                json!({ "reason": "aggregator not supported" }),
            ),
            ValidationError::Provider(p) => {
                (codes::INTERNAL_ERROR, json!({ "reason": p.to_string() }))
            }
        },
        BundlerError::Provider(e) => (codes::INTERNAL_ERROR, json!({ "reason": e.to_string() })),
        BundlerError::Signer(e) => (codes::INTERNAL_ERROR, json!({ "reason": e.to_string() })),
    }
}

/// Decoded `handleOps` revert. Variants mirror the `error` declarations on
/// the v0.7 `EntryPoint` contract (`FailedOp`, `FailedOpWithRevert`,
/// `SignatureValidationFailed`, `PostOpReverted`) plus the two standard
/// Solidity revert encodings (`Error(string)`, `Panic(uint256)`). Unknown
/// selectors fall through to [`Self::Unknown`] with the raw bytes preserved.
#[derive(Debug, Clone)]
pub enum HandleOpsRevertReason {
    FailedOp {
        op_index: u64,
        reason: String,
    },
    FailedOpWithRevert {
        op_index: u64,
        reason: String,
        inner: Bytes,
    },
    SignatureValidationFailed {
        aggregator: alloy::primitives::Address,
    },
    PostOpReverted {
        return_data: Bytes,
    },
    StandardError(String),
    Panic(u64),
    /// Selector matched none of the known shapes; `data` carries the raw
    /// revert bytes so operators can post-mortem from the logs.
    Unknown(Bytes),
}

impl std::fmt::Display for HandleOpsRevertReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FailedOp { op_index, reason } => write!(f, "FailedOp[{op_index}]: {reason}"),
            Self::FailedOpWithRevert {
                op_index, reason, ..
            } => write!(f, "FailedOpWithRevert[{op_index}]: {reason}"),
            Self::SignatureValidationFailed { aggregator } => {
                write!(f, "SignatureValidationFailed: aggregator={aggregator:?}")
            }
            Self::PostOpReverted { .. } => write!(f, "PostOpReverted"),
            Self::StandardError(m) => write!(f, "Error({m})"),
            Self::Panic(c) => write!(f, "Panic(0x{c:x})"),
            Self::Unknown(b) => write!(f, "unknown revert: 0x{}", hex::encode(b)),
        }
    }
}

impl HandleOpsRevertReason {
    /// JSON-RPC error code + payload for this revert. AA-prefixed reasons
    /// hint at the failing layer: `AA3*` is paymaster, anything else is
    /// account/EntryPoint.
    pub fn classify(&self) -> (i32, Value) {
        match self {
            Self::FailedOp { op_index, reason }
            | Self::FailedOpWithRevert {
                op_index, reason, ..
            } => {
                let code = if reason.starts_with("AA3") {
                    codes::PAYMASTER_REJECTED
                } else {
                    codes::SIMULATE_VALIDATION_REJECTED
                };
                (code, json!({ "reason": reason, "opIndex": op_index }))
            }
            Self::SignatureValidationFailed { aggregator } => (
                codes::SIMULATE_VALIDATION_REJECTED,
                json!({
                    "reason": "SignatureValidationFailed",
                    "aggregator": format!("{aggregator:?}"),
                }),
            ),
            Self::PostOpReverted { return_data } => (
                codes::PAYMASTER_REJECTED,
                json!({ "reason": "PostOpReverted", "returnData": format!("0x{}", hex::encode(return_data)) }),
            ),
            Self::StandardError(message) => (
                codes::SIMULATE_VALIDATION_REJECTED,
                json!({ "reason": "Error(string)", "message": message }),
            ),
            Self::Panic(code) => (
                codes::SIMULATE_VALIDATION_REJECTED,
                json!({ "reason": "Panic(uint256)", "code": format!("0x{code:x}") }),
            ),
            Self::Unknown(raw) => (
                codes::INTERNAL_ERROR,
                json!({ "reason": "undecodedRevert", "data": format!("0x{}", hex::encode(raw)) }),
            ),
        }
    }
}

const ERROR_STRING_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
const PANIC_UINT_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Match the first 4 bytes of `data` against the `EntryPoint` custom errors
/// and the two Solidity standard revert encodings. Returns
/// [`HandleOpsRevertReason::Unknown`] on shape mismatch so the caller still
/// gets the raw bytes.
pub fn decode_handle_ops_revert(data: &[u8]) -> HandleOpsRevertReason {
    let Some(selector) = data.get(0..4) else {
        return HandleOpsRevertReason::Unknown(Bytes::copy_from_slice(data));
    };
    let body = &data[4..];

    if selector == IEntryPoint::FailedOp::SELECTOR.as_slice() {
        if let Ok(decoded) = IEntryPoint::FailedOp::abi_decode_raw(body) {
            return HandleOpsRevertReason::FailedOp {
                op_index: clamp_u64(decoded.opIndex),
                reason: decoded.reason,
            };
        }
    } else if selector == IEntryPoint::FailedOpWithRevert::SELECTOR.as_slice() {
        if let Ok(decoded) = IEntryPoint::FailedOpWithRevert::abi_decode_raw(body) {
            return HandleOpsRevertReason::FailedOpWithRevert {
                op_index: clamp_u64(decoded.opIndex),
                reason: decoded.reason,
                inner: decoded.inner,
            };
        }
    } else if selector == IEntryPoint::SignatureValidationFailed::SELECTOR.as_slice() {
        if let Ok(decoded) = IEntryPoint::SignatureValidationFailed::abi_decode_raw(body) {
            return HandleOpsRevertReason::SignatureValidationFailed {
                aggregator: decoded.aggregator,
            };
        }
    } else if selector == IEntryPoint::PostOpReverted::SELECTOR.as_slice() {
        if let Ok(decoded) = IEntryPoint::PostOpReverted::abi_decode_raw(body) {
            return HandleOpsRevertReason::PostOpReverted {
                return_data: decoded.returnData,
            };
        }
    } else if selector == ERROR_STRING_SELECTOR {
        if let Ok(s) = String::abi_decode(body) {
            return HandleOpsRevertReason::StandardError(s);
        }
    } else if selector == PANIC_UINT_SELECTOR {
        if let Ok(code) = alloy::primitives::U256::abi_decode(body) {
            return HandleOpsRevertReason::Panic(clamp_u64(code));
        }
    }

    HandleOpsRevertReason::Unknown(Bytes::copy_from_slice(data))
}

fn clamp_u64(v: alloy::primitives::U256) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, U256};
    use alloy::sol_types::SolError;

    #[test]
    fn decodes_failed_op() {
        let encoded = IEntryPoint::FailedOp {
            opIndex: U256::from(2),
            reason: "AA21 didn't pay prefund".into(),
        }
        .abi_encode();
        let decoded = decode_handle_ops_revert(&encoded);
        let HandleOpsRevertReason::FailedOp { op_index, reason } = decoded else {
            panic!("expected FailedOp, got {decoded:?}");
        };
        assert_eq!(op_index, 2);
        assert_eq!(reason, "AA21 didn't pay prefund");
    }

    #[test]
    fn decodes_signature_validation_failed() {
        let agg = address!("0000000000000000000000000000000000000aaa");
        let encoded = IEntryPoint::SignatureValidationFailed { aggregator: agg }.abi_encode();
        let decoded = decode_handle_ops_revert(&encoded);
        assert!(matches!(
            decoded,
            HandleOpsRevertReason::SignatureValidationFailed { aggregator } if aggregator == agg
        ));
    }

    #[test]
    fn decodes_standard_error_string() {
        let mut encoded = ERROR_STRING_SELECTOR.to_vec();
        encoded.extend_from_slice(&"boom".to_string().abi_encode());
        let decoded = decode_handle_ops_revert(&encoded);
        assert!(matches!(decoded, HandleOpsRevertReason::StandardError(s) if s == "boom"));
    }

    #[test]
    fn paymaster_aa3_routes_to_paymaster_code() {
        let r = HandleOpsRevertReason::FailedOp {
            op_index: 0,
            reason: "AA31 paymaster deposit too low".into(),
        };
        let (code, _) = r.classify();
        assert_eq!(code, codes::PAYMASTER_REJECTED);
    }

    #[test]
    fn unknown_selector_falls_through() {
        let raw = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02];
        let decoded = decode_handle_ops_revert(&raw);
        assert!(matches!(decoded, HandleOpsRevertReason::Unknown(b) if b == raw));
    }
}
