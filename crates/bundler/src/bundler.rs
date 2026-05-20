//! Core bundler orchestration.
//!
//! Input: a batch of unpacked v0.7 `UserOperation`s plus chainId.
//! Output: a sequencer-signed type-2 transaction wrapping `EntryPoint.handleOps`,
//! ready for `eth_sendRawTransaction`.
//!
//! Pipeline (per call):
//! 1. Sanity: chainId, batch size, per-op fee policy.
//! 2. Pack each op (`crate::packing::pack`).
//! 3. `simulateValidation` per op via [`crate::validator`].
//! 4. Prefund check: full v0.7 formula including paymaster gas limits.
//! 5. `getUserOpHash` per op for the response.
//! 6. ABI-encode `handleOps`, `eth_estimateGas` + margin.
//! 7. Choose outer-tx fees: bundler minimum priority fee, capped by the
//!    smallest user-supplied cap across the batch.
//! 8. Build & sign type-2 tx.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::consensus::TxEip1559;
use alloy::eips::eip2718::Encodable2718;
use alloy::primitives::{Bytes, TxKind, B256, U256};
use alloy::sol_types::SolCall;

use crate::config::BundlerConfig;
use crate::contracts::{IEntryPoint, PackedUserOperation};
use crate::errors::BundlerError;
use crate::packing::{self, parse_paymaster_and_data};
use crate::provider::EthProvider;
use crate::signer::Signer;
use crate::types::{BuildOpts, SignedTxResp, UserOpV07};
use crate::validator;

pub struct Bundler<P: EthProvider + ?Sized, S: Signer + ?Sized> {
    cfg: BundlerConfig,
    provider: Arc<P>,
    signer: Arc<S>,
    simulations_code: Bytes,
}

impl<P: EthProvider + ?Sized, S: Signer + ?Sized> std::fmt::Debug for Bundler<P, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundler")
            .field("chain_id", &self.cfg.chain_id)
            .field("entrypoint", &self.cfg.entrypoint_address)
            .field("signer", &self.signer.address())
            .finish()
    }
}

impl<P: EthProvider + ?Sized, S: Signer + ?Sized> Bundler<P, S> {
    pub fn new(cfg: BundlerConfig, provider: Arc<P>, signer: Arc<S>) -> Result<Self, BundlerError> {
        let code =
            hex::decode(cfg.entrypoint_simulations_code.trim_start_matches("0x")).map_err(|e| {
                BundlerError::InvalidBatch(Box::leak(
                    format!("invalid ENTRYPOINT_SIMULATIONS_CODE hex: {e}").into_boxed_str(),
                ))
            })?;
        Ok(Self {
            cfg,
            provider,
            signer,
            simulations_code: Bytes::from(code),
        })
    }

    pub async fn build_signed_user_ops_tx(
        &self,
        ops: Vec<UserOpV07>,
        opts: BuildOpts,
    ) -> Result<SignedTxResp, BundlerError> {
        if opts.chain_id != self.cfg.chain_id {
            return Err(BundlerError::WrongChainId {
                expected: self.cfg.chain_id,
                got: opts.chain_id,
            });
        }
        if ops.is_empty() {
            return Err(BundlerError::InvalidBatch("empty userOps"));
        }
        if ops.len() > self.cfg.max_batch_size {
            return Err(BundlerError::BatchTooLarge {
                got: ops.len(),
                max: self.cfg.max_batch_size,
            });
        }

        let base_fee = self.provider.base_fee().await?;
        let suggested_tip = self.provider.max_priority_fee().await?;
        let now_unix = unix_now();

        let mut packed_ops: Vec<PackedUserOperation> = Vec::with_capacity(ops.len());
        let mut user_op_hashes: Vec<B256> = Vec::with_capacity(ops.len());
        let mut min_user_tip: Option<U256> = None;
        let mut min_user_fee_cap: Option<U256> = None;

        for (i, op) in ops.iter().enumerate() {
            self.check_per_op_fees(i, op, base_fee)?;
            let packed = packing::pack(op)?;
            self.check_prefund(i, op, &packed).await?;

            validator::simulate(
                self.provider.as_ref(),
                self.cfg.entrypoint_address,
                self.simulations_code.clone(),
                packed.clone(),
                now_unix,
            )
            .await?;

            let hash = self
                .provider
                .user_op_hash(self.cfg.entrypoint_address, packed.clone())
                .await?;
            user_op_hashes.push(hash);
            packed_ops.push(packed);

            update_min(&mut min_user_tip, op.max_priority_fee_per_gas);
            update_min(&mut min_user_fee_cap, op.max_fee_per_gas);
        }

        let beneficiary = self.signer.address();
        let call_data = IEntryPoint::handleOpsCall {
            ops: packed_ops,
            beneficiary,
        }
        .abi_encode();
        let call_data = Bytes::from(call_data);

        let estimated = self
            .provider
            .estimate_handle_ops_gas(beneficiary, self.cfg.entrypoint_address, call_data.clone())
            .await?;
        let gas = estimated.saturating_add(estimated * u64::from(self.cfg.gas_margin_pct) / 100);

        let (outer_tip, outer_fee_cap) = self.choose_outer_fees(
            base_fee,
            suggested_tip,
            min_user_tip.expect("non-empty batch"),
            min_user_fee_cap.expect("non-empty batch"),
        )?;

        let nonce = self.provider.pending_nonce(beneficiary).await?;

        let tx = TxEip1559 {
            chain_id: self.cfg.chain_id,
            nonce,
            gas_limit: gas,
            max_fee_per_gas: u128_from_u256(outer_fee_cap, "outer maxFeePerGas")?,
            max_priority_fee_per_gas: u128_from_u256(outer_tip, "outer maxPriorityFeePerGas")?,
            to: TxKind::Call(self.cfg.entrypoint_address),
            value: U256::ZERO,
            access_list: Default::default(),
            input: call_data,
        };

        let envelope = self.signer.sign_eip1559(tx).await?;
        let raw = Bytes::from(envelope.encoded_2718());
        let hash = *envelope.tx_hash();

        Ok(SignedTxResp {
            raw,
            hash,
            to: self.cfg.entrypoint_address,
            chain_id: self.cfg.chain_id,
            gas: U256::from(gas),
            max_fee_per_gas: outer_fee_cap,
            max_priority_fee_per_gas: outer_tip,
            user_op_hashes,
        })
    }

    fn check_per_op_fees(
        &self,
        i: usize,
        op: &UserOpV07,
        base_fee: U256,
    ) -> Result<(), BundlerError> {
        if op.max_priority_fee_per_gas > op.max_fee_per_gas {
            return Err(BundlerError::InvalidUserOp {
                op_index: i,
                reason: "maxPriorityFeePerGas > maxFeePerGas".into(),
            });
        }
        if op.max_priority_fee_per_gas < self.cfg.min_priority_fee_wei {
            return Err(BundlerError::PriorityFeeTooLow {
                op_index: i,
                got: op.max_priority_fee_per_gas.to_string(),
                min: self.cfg.min_priority_fee_wei.to_string(),
            });
        }
        let required = base_fee.saturating_add(self.cfg.min_priority_fee_wei);
        if op.max_fee_per_gas < required {
            return Err(BundlerError::FeeCapTooLow {
                op_index: i,
                fee_cap: op.max_fee_per_gas.to_string(),
                required: required.to_string(),
            });
        }
        Ok(())
    }

    /// Canonical v0.7 prefund:
    /// `(verGas + callGas + pmVerGas + pmPostOpGas + preVerGas) * maxFeePerGas`
    /// vs. `EntryPoint.balanceOf(paymaster || sender)`.
    async fn check_prefund(
        &self,
        i: usize,
        op: &UserOpV07,
        packed: &PackedUserOperation,
    ) -> Result<(), BundlerError> {
        let (sponsor, pm_ver, pm_post) = match parse_paymaster_and_data(&packed.paymasterAndData)? {
            Some((pm, ver, post)) if !pm.is_zero() => (pm, ver, post),
            _ => (op.sender, U256::ZERO, U256::ZERO),
        };

        let total_gas = op
            .verification_gas_limit
            .saturating_add(op.call_gas_limit)
            .saturating_add(pm_ver)
            .saturating_add(pm_post)
            .saturating_add(op.pre_verification_gas);
        let required = total_gas.saturating_mul(op.max_fee_per_gas);

        let deposit = self
            .provider
            .balance_of(self.cfg.entrypoint_address, sponsor)
            .await?;
        if deposit < required {
            return Err(BundlerError::InsufficientDeposit {
                op_index: i,
                sponsor: format!("{sponsor:?}"),
                required: required.to_string(),
                deposit: deposit.to_string(),
            });
        }
        Ok(())
    }

    /// Outer-tx fee policy:
    /// * `tip` = `min(max(bundler.min_priority_fee, suggested_tip), min(op.maxPriorityFeePerGas))`
    /// * `feeCap` = `min(op.maxFeePerGas)` across batch
    /// * Reject if `feeCap < baseFee + tip` (transient: network moved during the call).
    fn choose_outer_fees(
        &self,
        base_fee: U256,
        suggested_tip: U256,
        min_user_tip: U256,
        min_user_fee_cap: U256,
    ) -> Result<(U256, U256), BundlerError> {
        let desired_tip = max_u256(self.cfg.min_priority_fee_wei, suggested_tip);
        let tip = min_u256(desired_tip, min_user_tip);
        let fee_cap = min_user_fee_cap;

        let required = base_fee.saturating_add(tip);
        if fee_cap < required {
            return Err(BundlerError::FeeCapTooLow {
                op_index: usize::MAX,
                fee_cap: fee_cap.to_string(),
                required: required.to_string(),
            });
        }
        Ok((tip, fee_cap))
    }
}

fn update_min(slot: &mut Option<U256>, candidate: U256) {
    match slot {
        Some(current) if *current <= candidate => {}
        _ => *slot = Some(candidate),
    }
}

fn min_u256(a: U256, b: U256) -> U256 {
    if a <= b {
        a
    } else {
        b
    }
}

fn max_u256(a: U256, b: U256) -> U256 {
    if a >= b {
        a
    } else {
        b
    }
}

fn u128_from_u256(v: U256, field: &'static str) -> Result<u128, BundlerError> {
    u128::try_from(v).map_err(|_| BundlerError::InvalidUserOp {
        op_index: usize::MAX,
        reason: format!("{field} exceeds u128"),
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
