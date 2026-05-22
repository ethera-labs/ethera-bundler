//! Bundler error type and its mapping to ERC-4337 JSON-RPC error codes.
//!
//! Reference: [ERC-4337 Bundler RPC error codes][spec].
//!
//! [spec]: https://eips.ethereum.org/EIPS/eip-4337#error-codes

use serde_json::{json, Value};

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
