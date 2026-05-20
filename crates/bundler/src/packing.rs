//! v0.7 `UserOperation` packing and `paymasterAndData` parsing.
//!
//! v0.7 splits the wire format into 17 fields but the onchain struct packs
//! several `uint128` pairs into single `bytes32` words. This module is the
//! only place that knows the layout.

use alloy::primitives::{Address, Bytes, B256, U256};

use crate::contracts::PackedUserOperation;
use crate::types::UserOpV07;

/// Error returned by [`pack`] / [`pack_pair_to_bytes32`] when a value exceeds
/// the `uint128` range packed words allow.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("value exceeds uint128 bounds in {field}")]
    Overflow { field: &'static str },
    #[error("paymasterAndData too short: {0} bytes (need at least 52)")]
    PaymasterAndDataTooShort(usize),
}

/// Layout of the paymaster gas-limit prefix inside `paymasterAndData`.
///
/// `[0..20]` paymaster address · `[20..36]` `paymasterVerificationGasLimit`
/// (uint128 BE) · `[36..52]` `paymasterPostOpGasLimit` (uint128 BE) ·
/// `[52..]` paymaster-specific data.
pub const PAYMASTER_HEADER_LEN: usize = 52;
const ADDRESS_LEN: usize = 20;
const U128_LEN: usize = 16;

/// Pack two `uint128`s into a `bytes32`: `(hi << 128) | lo`.
pub fn pack_pair_to_bytes32(hi: U256, lo: U256, field: &'static str) -> Result<B256, PackError> {
    if hi.bit_len() > 128 || lo.bit_len() > 128 {
        return Err(PackError::Overflow { field });
    }
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&hi.to_be_bytes::<32>()[16..]);
    out[16..].copy_from_slice(&lo.to_be_bytes::<32>()[16..]);
    Ok(B256::from(out))
}

/// Convert an unpacked wire-format [`UserOpV07`] to the onchain [`PackedUserOperation`].
///
/// Accepts either the v0.7 unpacked fields (`factory`/`factoryData`,
/// `paymaster`/`paymasterVerificationGasLimit`/...) or the pre-packed
/// `initCode` / `paymasterAndData` blobs. Pre-packed blobs take precedence
/// when both are present, matching viem's behavior.
pub fn pack(op: &UserOpV07) -> Result<PackedUserOperation, PackError> {
    let init_code = if !op.init_code.is_empty() {
        op.init_code.clone()
    } else {
        match op.factory {
            Some(f) if !f.is_zero() => {
                let mut buf = Vec::with_capacity(ADDRESS_LEN + op.factory_data.len());
                buf.extend_from_slice(f.as_slice());
                buf.extend_from_slice(&op.factory_data);
                Bytes::from(buf)
            }
            _ => Bytes::new(),
        }
    };

    let paymaster_and_data = if !op.paymaster_and_data.is_empty() {
        op.paymaster_and_data.clone()
    } else {
        match op.paymaster {
            Some(pm) if !pm.is_zero() => {
                if op.paymaster_verification_gas_limit.bit_len() > 128 {
                    return Err(PackError::Overflow {
                        field: "paymasterVerificationGasLimit",
                    });
                }
                if op.paymaster_post_op_gas_limit.bit_len() > 128 {
                    return Err(PackError::Overflow {
                        field: "paymasterPostOpGasLimit",
                    });
                }
                let mut buf = Vec::with_capacity(PAYMASTER_HEADER_LEN + op.paymaster_data.len());
                buf.extend_from_slice(pm.as_slice());
                buf.extend_from_slice(
                    &op.paymaster_verification_gas_limit.to_be_bytes::<32>()[16..],
                );
                buf.extend_from_slice(&op.paymaster_post_op_gas_limit.to_be_bytes::<32>()[16..]);
                buf.extend_from_slice(&op.paymaster_data);
                Bytes::from(buf)
            }
            _ => Bytes::new(),
        }
    };

    let account_gas_limits = pack_pair_to_bytes32(
        op.verification_gas_limit,
        op.call_gas_limit,
        "accountGasLimits",
    )?;
    let gas_fees =
        pack_pair_to_bytes32(op.max_priority_fee_per_gas, op.max_fee_per_gas, "gasFees")?;

    Ok(PackedUserOperation {
        sender: op.sender,
        nonce: op.nonce,
        initCode: init_code,
        callData: op.call_data.clone(),
        accountGasLimits: account_gas_limits,
        preVerificationGas: op.pre_verification_gas,
        gasFees: gas_fees,
        paymasterAndData: paymaster_and_data,
        signature: op.signature.clone(),
    })
}

/// Extract `(paymaster, verificationGasLimit, postOpGasLimit)` from a
/// `paymasterAndData` blob. Returns `Ok(None)` for an empty blob (no paymaster).
pub fn parse_paymaster_and_data(blob: &[u8]) -> Result<Option<(Address, U256, U256)>, PackError> {
    if blob.is_empty() {
        return Ok(None);
    }
    if blob.len() < PAYMASTER_HEADER_LEN {
        return Err(PackError::PaymasterAndDataTooShort(blob.len()));
    }
    let paymaster = Address::from_slice(&blob[..ADDRESS_LEN]);
    let ver = U256::from_be_slice(&blob[ADDRESS_LEN..ADDRESS_LEN + U128_LEN]);
    let post = U256::from_be_slice(&blob[ADDRESS_LEN + U128_LEN..PAYMASTER_HEADER_LEN]);
    Ok(Some((paymaster, ver, post)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, b256, bytes};

    #[test]
    fn pack_pair_zero() {
        let out = pack_pair_to_bytes32(U256::ZERO, U256::ZERO, "x").unwrap();
        assert_eq!(out, B256::ZERO);
    }

    #[test]
    fn pack_pair_canonical() {
        // hi = 0x1, lo = 0x2 → 0x0000…00010000…0002
        let out = pack_pair_to_bytes32(U256::from(1), U256::from(2), "x").unwrap();
        assert_eq!(
            out,
            b256!("0000000000000000000000000000000100000000000000000000000000000002")
        );
    }

    #[test]
    fn pack_pair_max_uint128() {
        let max = (U256::from(1) << 128) - U256::from(1);
        let out = pack_pair_to_bytes32(max, max, "x").unwrap();
        assert_eq!(
            out,
            b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
        );
    }

    #[test]
    fn pack_pair_overflow() {
        let overflow = U256::from(1) << 128;
        let err = pack_pair_to_bytes32(overflow, U256::ZERO, "gasFees").unwrap_err();
        assert!(matches!(err, PackError::Overflow { field: "gasFees" }));
    }

    fn base_op() -> UserOpV07 {
        UserOpV07 {
            sender: address!("0101010101010101010101010101010101010101"),
            nonce: U256::from(7),
            init_code: Bytes::new(),
            factory: None,
            factory_data: Bytes::new(),
            call_data: bytes!("aabb"),
            call_gas_limit: U256::from(100_000),
            verification_gas_limit: U256::from(200_000),
            pre_verification_gas: U256::from(30_000),
            max_fee_per_gas: U256::from(2_000_000_000u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000u64),
            paymaster: None,
            paymaster_verification_gas_limit: U256::ZERO,
            paymaster_post_op_gas_limit: U256::ZERO,
            paymaster_data: Bytes::new(),
            paymaster_and_data: Bytes::new(),
            signature: bytes!("cc"),
        }
    }

    #[test]
    fn pack_no_factory_no_paymaster() {
        let p = pack(&base_op()).unwrap();
        assert!(p.initCode.is_empty());
        assert!(p.paymasterAndData.is_empty());
        assert_eq!(p.callData, bytes!("aabb"));
        assert_eq!(p.signature, bytes!("cc"));
    }

    #[test]
    fn pack_with_factory_and_data() {
        let mut op = base_op();
        op.factory = Some(address!("0202020202020202020202020202020202020202"));
        op.factory_data = bytes!("deadbeef");
        let p = pack(&op).unwrap();
        assert_eq!(
            p.initCode,
            bytes!("0202020202020202020202020202020202020202deadbeef")
        );
    }

    #[test]
    fn pack_prefers_explicit_init_code() {
        let mut op = base_op();
        op.factory = Some(address!("0202020202020202020202020202020202020202"));
        op.factory_data = bytes!("deadbeef");
        op.init_code = bytes!("cafe");
        let p = pack(&op).unwrap();
        assert_eq!(p.initCode, bytes!("cafe"));
    }

    #[test]
    fn pack_with_paymaster_unpacked() {
        let mut op = base_op();
        op.paymaster = Some(address!("0303030303030303030303030303030303030303"));
        op.paymaster_verification_gas_limit = U256::from(50_000);
        op.paymaster_post_op_gas_limit = U256::from(20_000);
        op.paymaster_data = bytes!("f00d");
        let p = pack(&op).unwrap();
        assert_eq!(p.paymasterAndData.len(), PAYMASTER_HEADER_LEN + 2);
        let parsed = parse_paymaster_and_data(&p.paymasterAndData)
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed.0,
            address!("0303030303030303030303030303030303030303")
        );
        assert_eq!(parsed.1, U256::from(50_000));
        assert_eq!(parsed.2, U256::from(20_000));
        assert_eq!(&p.paymasterAndData[PAYMASTER_HEADER_LEN..], &[0xf0, 0x0d]);
    }

    #[test]
    fn parse_paymaster_empty_is_none() {
        assert!(parse_paymaster_and_data(&[]).unwrap().is_none());
    }

    #[test]
    fn parse_paymaster_too_short_errors() {
        let err = parse_paymaster_and_data(&[0u8; 30]).unwrap_err();
        assert!(matches!(err, PackError::PaymasterAndDataTooShort(30)));
    }

    #[test]
    fn pack_gas_fees_layout() {
        // v0.7 `gasFees` unpacks via
        // `unpackUints(packed) -> (maxPriorityFeePerGas, maxFeePerGas)`,
        // so the tip occupies the high 128 bits and the fee cap the low 128.
        let mut op = base_op();
        op.max_priority_fee_per_gas = U256::from(0xAAu64);
        op.max_fee_per_gas = U256::from(0xBBu64);
        let p = pack(&op).unwrap();
        assert_eq!(
            p.gasFees,
            b256!("000000000000000000000000000000aa000000000000000000000000000000bb")
        );
    }
}
