//! Per-op canonical ERC-4337 v0.7 validation.
//!
//! Calls `IEntryPointSimulations.simulateValidation` via state-override
//! `eth_call`, then parses `ValidationResult.returnInfo` to surface the
//! account/paymaster validation data, prefund, and time range.

use alloy::primitives::{Address, U256};

use crate::contracts::{PackedUserOperation, ValidationResult};
use crate::provider::{EthProvider, ProviderError};

/// `address(1)` - the `EntryPoint` constant indicating signature validation failed.
/// `address(0)` is the "no aggregator, success" sentinel.
const SIG_VALIDATION_FAILED: Address = Address::with_last_byte(1);

/// `type(uint48).max` - `EntryPoint` treats `validUntil == 0` as "no expiry"
/// by substituting this value.
const UINT48_MAX: u64 = (1u64 << 48) - 1;

#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub pre_op_gas: U256,
    pub prefund: U256,
    pub account: ValidationWindow,
    pub paymaster: ValidationWindow,
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationWindow {
    pub aggregator: Address,
    pub valid_after: u64,
    pub valid_until: u64,
}

impl ValidationWindow {
    pub fn signature_failed(&self) -> bool {
        self.aggregator == SIG_VALIDATION_FAILED
    }

    pub fn requires_aggregator(&self) -> bool {
        !self.aggregator.is_zero() && !self.signature_failed()
    }

    pub fn is_out_of_time_range(&self, now: u64) -> bool {
        now > self.valid_until || now < self.valid_after
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("account signature validation failed")]
    AccountSignatureFailed,
    #[error("paymaster signature validation failed")]
    PaymasterSignatureFailed,
    #[error("userop out of time range (now={now}, account=[{a_after}..{a_until}], paymaster=[{p_after}..{p_until}])")]
    OutOfTimeRange {
        now: u64,
        a_after: u64,
        a_until: u64,
        p_after: u64,
        p_until: u64,
    },
    #[error("aggregator-signed userops are not supported")]
    UnsupportedAggregator,
}

/// Simulate `op` against the given `EntryPoint`, returning a parsed report.
/// Hard-fails on signature / time-range / aggregator errors; surfaces the
/// numeric fields (`pre_op_gas`, `prefund`) for callers that want to cross-check.
pub async fn simulate<P: EthProvider + ?Sized>(
    provider: &P,
    entrypoint: Address,
    simulations_code: alloy::primitives::Bytes,
    op: PackedUserOperation,
    now_unix: u64,
) -> Result<ValidationReport, ValidationError> {
    let raw: ValidationResult = provider
        .simulate_validation(entrypoint, simulations_code, op)
        .await?;

    let account = parse_window(raw.returnInfo.accountValidationData);
    let paymaster = parse_window(raw.returnInfo.paymasterValidationData);

    if account.signature_failed() {
        return Err(ValidationError::AccountSignatureFailed);
    }
    if paymaster.signature_failed() {
        return Err(ValidationError::PaymasterSignatureFailed);
    }
    if account.requires_aggregator() || paymaster.requires_aggregator() {
        return Err(ValidationError::UnsupportedAggregator);
    }
    if account.is_out_of_time_range(now_unix) || paymaster.is_out_of_time_range(now_unix) {
        return Err(ValidationError::OutOfTimeRange {
            now: now_unix,
            a_after: account.valid_after,
            a_until: account.valid_until,
            p_after: paymaster.valid_after,
            p_until: paymaster.valid_until,
        });
    }

    Ok(ValidationReport {
        pre_op_gas: raw.returnInfo.preOpGas,
        prefund: raw.returnInfo.prefund,
        account,
        paymaster,
    })
}

/// Layout per ERC-4337 v0.7 `_parseValidationData`:
/// `aggregator | (validUntil << 160) | (validAfter << (160+48))`.
fn parse_window(data: U256) -> ValidationWindow {
    let mask_160 = (U256::from(1u64) << 160) - U256::from(1u64);
    let mask_48: U256 = U256::from(UINT48_MAX);

    let aggregator_bits: U256 = data & mask_160;
    let aggregator = Address::from_slice(&aggregator_bits.to_be_bytes::<32>()[12..]);

    let valid_until_raw = u64::try_from((data >> 160) & mask_48).unwrap_or(UINT48_MAX);
    let valid_until = if valid_until_raw == 0 {
        UINT48_MAX
    } else {
        valid_until_raw
    };

    let valid_after = u64::try_from((data >> (160 + 48)) & mask_48).unwrap_or(0);

    ValidationWindow {
        aggregator,
        valid_after,
        valid_until,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn make(aggregator: u64, valid_until: u64, valid_after: u64) -> U256 {
        let mut v = U256::from(aggregator);
        v |= U256::from(valid_until) << 160;
        v |= U256::from(valid_after) << (160 + 48);
        v
    }

    #[test]
    fn parse_window_success() {
        let w = parse_window(make(0, 1000, 100));
        assert!(w.aggregator.is_zero());
        assert!(!w.signature_failed());
        assert_eq!(w.valid_after, 100);
        assert_eq!(w.valid_until, 1000);
        assert!(w.is_out_of_time_range(50));
        assert!(!w.is_out_of_time_range(500));
        assert!(w.is_out_of_time_range(2000));
    }

    #[test]
    fn parse_window_signature_failed() {
        let w = parse_window(make(1, 0, 0));
        assert!(w.signature_failed());
        assert!(!w.requires_aggregator());
    }

    #[test]
    fn parse_window_zero_is_success_no_expiry() {
        let w = parse_window(U256::ZERO);
        assert!(w.aggregator.is_zero());
        assert!(!w.signature_failed());
        assert_eq!(w.valid_after, 0);
        assert_eq!(w.valid_until, UINT48_MAX);
        // `validUntil == 0` saturates to UINT48_MAX, so any timestamp up to
        // that bound is in range and anything beyond it is out.
        assert!(!w.is_out_of_time_range(UINT48_MAX));
        assert!(w.is_out_of_time_range(UINT48_MAX + 1));
    }

    #[test]
    fn parse_window_aggregator() {
        let agg = address!("00000000000000000000000000000000000000aa");
        let mut data = U256::ZERO;
        data |= U256::from_be_bytes::<32>({
            let mut buf = [0u8; 32];
            buf[12..].copy_from_slice(agg.as_slice());
            buf
        });
        let w = parse_window(data);
        assert_eq!(w.aggregator, agg);
        assert!(w.requires_aggregator());
    }
}
