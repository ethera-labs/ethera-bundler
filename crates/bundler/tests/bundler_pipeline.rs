//! End-to-end pipeline tests against a mocked `EthProvider`. Exercise the fee
//! policy, prefund formula, simulation, signing, and EIP-2718 encoding.

use std::sync::Arc;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use alloy::consensus::TxEnvelope;
use alloy::eips::eip2718::Decodable2718;
use alloy::primitives::{address, b256, bytes, Address, Bytes, B256, U256};
use alloy::sol_types::SolCall;
use async_trait::async_trait;
use ethera_bundler_core::bundler::Bundler;
use ethera_bundler_core::config::{BundlerConfig, ENTRYPOINT_V07};
use ethera_bundler_core::contracts::{
    AggregatorStakeInfo, IEntryPoint, PackedUserOperation, ReturnInfo, StakeInfo, ValidationResult,
};
use ethera_bundler_core::provider::{EthProvider, ProviderError, ReceiptInfo};
use ethera_bundler_core::signer::LocalSigner;
use ethera_bundler_core::types::{BuildOpts, UserOpV07};

const TEST_CHAIN_ID: u64 = 31337;
const TEST_KEY: B256 = b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
const TEST_BASE_FEE: u64 = 1_000_000_000; // 1 gwei
const TEST_TIP: u64 = 1_000_000_000; // 1 gwei
const TEST_NONCE: u64 = 7;
const TEST_GAS_ESTIMATE: u64 = 250_000;
const TEST_DEPOSIT: u128 = 10u128.pow(18); // 1 ETH
const TEST_USER_OP_HASH: B256 =
    b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

#[derive(Debug, Clone)]
struct MockProvider {
    deposit: U256,
    submissions: Arc<std::sync::Mutex<Vec<Bytes>>>,
    /// The execution layer's pending nonce. Tests mutate it to model a nonce
    /// being reserved (advance) or released after an abort (drop back down).
    pending_nonce: Arc<AtomicU64>,
    /// When `true`, `send_raw_transaction` fails without recording the
    /// submission.
    fail_send: Arc<AtomicBool>,
    /// When `true`, `wait_for_receipt` returns `Ok(None)` to simulate a timeout.
    timeout_receipt: Arc<AtomicBool>,
    /// When `true`, a successful `send_raw_transaction` bumps `pending_nonce`,
    /// modelling the execution layer accepting the tx.
    advance_pending_on_send: Arc<AtomicBool>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self {
            deposit: U256::from(TEST_DEPOSIT),
            submissions: Arc::new(std::sync::Mutex::new(Vec::new())),
            pending_nonce: Arc::new(AtomicU64::new(TEST_NONCE)),
            fail_send: Arc::new(AtomicBool::new(false)),
            timeout_receipt: Arc::new(AtomicBool::new(false)),
            advance_pending_on_send: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl EthProvider for MockProvider {
    async fn chain_id(&self) -> Result<u64, ProviderError> {
        Ok(TEST_CHAIN_ID)
    }
    async fn base_fee(&self) -> Result<U256, ProviderError> {
        Ok(U256::from(TEST_BASE_FEE))
    }
    async fn max_priority_fee(&self) -> Result<U256, ProviderError> {
        Ok(U256::from(TEST_TIP))
    }
    async fn pending_nonce(&self, _addr: Address) -> Result<u64, ProviderError> {
        Ok(self.pending_nonce.load(Ordering::Relaxed))
    }
    async fn balance(&self, _addr: Address) -> Result<U256, ProviderError> {
        Ok(U256::from(TEST_DEPOSIT))
    }
    async fn get_code(&self, _addr: Address) -> Result<Bytes, ProviderError> {
        Ok(Bytes::from_static(b"\x00\x01\x02"))
    }
    async fn balance_of(&self, _ep: Address, _account: Address) -> Result<U256, ProviderError> {
        Ok(self.deposit)
    }
    async fn user_op_hash(
        &self,
        _ep: Address,
        _op: PackedUserOperation,
    ) -> Result<B256, ProviderError> {
        Ok(TEST_USER_OP_HASH)
    }
    async fn send_raw_transaction(&self, raw: Bytes) -> Result<B256, ProviderError> {
        if self.fail_send.load(Ordering::Relaxed) {
            return Err(ProviderError::Transport("nonce too low".into()));
        }
        let envelope = TxEnvelope::decode_2718(&mut raw.as_ref())
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let hash = *envelope.tx_hash();
        self.submissions.lock().unwrap().push(raw);
        if self.advance_pending_on_send.load(Ordering::Relaxed) {
            self.pending_nonce.fetch_add(1, Ordering::Relaxed);
        }
        Ok(hash)
    }
    async fn wait_for_receipt(
        &self,
        hash: B256,
        _timeout: Duration,
    ) -> Result<Option<ReceiptInfo>, ProviderError> {
        if self.timeout_receipt.load(Ordering::Relaxed) {
            return Ok(None);
        }
        Ok(Some(ReceiptInfo {
            tx_hash: hash,
            block_number: 1,
            success: true,
        }))
    }
    async fn try_call_for_revert(
        &self,
        _from: Address,
        _to: Address,
        _data: Bytes,
    ) -> Result<Option<Bytes>, ProviderError> {
        Ok(None)
    }
    async fn estimate_handle_ops_gas(
        &self,
        _from: Address,
        _ep: Address,
        _data: Bytes,
    ) -> Result<u64, ProviderError> {
        Ok(TEST_GAS_ESTIMATE)
    }
    async fn simulate_validation(
        &self,
        _ep: Address,
        _code: Bytes,
        _op: PackedUserOperation,
    ) -> Result<ValidationResult, ProviderError> {
        Ok(ValidationResult {
            returnInfo: ReturnInfo {
                preOpGas: U256::from(100_000),
                prefund: U256::from(50_000),
                accountValidationData: U256::ZERO, // success, no expiry
                paymasterValidationData: U256::ZERO,
                paymasterContext: Bytes::new(),
            },
            senderInfo: StakeInfo {
                stake: U256::ZERO,
                unstakeDelay: U256::ZERO,
            },
            factoryInfo: StakeInfo {
                stake: U256::ZERO,
                unstakeDelay: U256::ZERO,
            },
            paymasterInfo: StakeInfo {
                stake: U256::ZERO,
                unstakeDelay: U256::ZERO,
            },
            aggregatorInfo: AggregatorStakeInfo {
                aggregator: Address::ZERO,
                stakeInfo: StakeInfo {
                    stake: U256::ZERO,
                    unstakeDelay: U256::ZERO,
                },
            },
        })
    }
}

fn test_config() -> BundlerConfig {
    BundlerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        chain_id: TEST_CHAIN_ID,
        exec_rpc_url: "http://localhost:0/unused".into(),
        entrypoint_address: ENTRYPOINT_V07,
        sequencer_key: TEST_KEY,
        max_batch_size: 10,
        min_priority_fee_wei: U256::from(1_000_000_000u64), // 1 gwei
        gas_margin_pct: 10,
    }
}

fn happy_op() -> UserOpV07 {
    UserOpV07 {
        sender: address!("1111111111111111111111111111111111111111"),
        nonce: U256::from(1),
        init_code: Bytes::new(),
        factory: None,
        factory_data: Bytes::new(),
        call_data: bytes!("b61d27f6"), // execute() selector - irrelevant for the mock
        call_gas_limit: U256::from(100_000),
        verification_gas_limit: U256::from(100_000),
        pre_verification_gas: U256::from(50_000),
        max_fee_per_gas: U256::from(5_000_000_000u64), // 5 gwei
        max_priority_fee_per_gas: U256::from(2_000_000_000u64), // 2 gwei
        paymaster: None,
        paymaster_verification_gas_limit: U256::ZERO,
        paymaster_post_op_gas_limit: U256::ZERO,
        paymaster_data: Bytes::new(),
        paymaster_and_data: Bytes::new(),
        signature: bytes!("00"),
    }
}

async fn run(
    provider: MockProvider,
    ops: Vec<UserOpV07>,
) -> ethera_bundler_core::types::SignedTxResp {
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();
    bundler
        .build_signed_user_ops_tx(
            ops,
            BuildOpts {
                chain_id: TEST_CHAIN_ID,
                submit: true,
            },
        )
        .await
        .expect("bundler pipeline should succeed for the happy path")
}

#[tokio::test]
async fn happy_path_single_op_returns_signed_tx() {
    let resp = run(MockProvider::default(), vec![happy_op()]).await;

    assert_eq!(resp.chain_id, TEST_CHAIN_ID);
    assert_eq!(resp.to, ENTRYPOINT_V07);
    assert_eq!(resp.user_op_hashes, vec![TEST_USER_OP_HASH]);

    // gas = estimate + 10%
    assert_eq!(
        resp.gas,
        U256::from(TEST_GAS_ESTIMATE + TEST_GAS_ESTIMATE / 10)
    );

    // outer tip = max(bundler_min, suggested) = max(1g, 1g) = 1g
    assert_eq!(resp.max_priority_fee_per_gas, U256::from(1_000_000_000u64));
    // outer feeCap = min(op.maxFeePerGas) = 5g
    assert_eq!(resp.max_fee_per_gas, U256::from(5_000_000_000u64));

    // The raw tx must round-trip through the EIP-2718 decoder and match `resp.hash`.
    let envelope =
        TxEnvelope::decode_2718(&mut resp.raw.as_ref()).expect("raw must be EIP-2718-encoded");
    assert_eq!(*envelope.tx_hash(), resp.hash);

    let TxEnvelope::Eip1559(signed) = envelope else {
        panic!("expected EIP-1559 envelope, got {envelope:?}");
    };
    let tx = signed.tx();
    assert_eq!(tx.chain_id, TEST_CHAIN_ID);
    assert_eq!(tx.nonce, TEST_NONCE);
    assert_eq!(tx.to.to().copied(), Some(ENTRYPOINT_V07));

    // Calldata decodes back into `handleOps(ops, beneficiary)` with our op.
    let decoded = IEntryPoint::handleOpsCall::abi_decode(&tx.input).expect("calldata is handleOps");
    assert_eq!(decoded.ops.len(), 1);
    assert_eq!(decoded.ops[0].sender, happy_op().sender);
    assert_eq!(
        decoded.beneficiary,
        LocalSigner::from_key(TEST_KEY).unwrap().address_alias()
    );
}

#[tokio::test]
async fn rejects_wrong_chain_id() {
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(MockProvider::default()), signer).unwrap();

    let err = bundler
        .build_signed_user_ops_tx(
            vec![happy_op()],
            BuildOpts {
                chain_id: 999,
                submit: true,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("wrong chainId"), "got: {err}");
}

#[tokio::test]
async fn rejects_priority_fee_below_minimum() {
    let mut op = happy_op();
    op.max_priority_fee_per_gas = U256::from(1u64); // 1 wei - well below 1 gwei

    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(MockProvider::default()), signer).unwrap();

    let err = bundler
        .build_signed_user_ops_tx(
            vec![op],
            BuildOpts {
                chain_id: TEST_CHAIN_ID,
                submit: true,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("priority fee"), "got: {err}");
}

#[tokio::test]
async fn rejects_insufficient_deposit() {
    let provider = MockProvider {
        deposit: U256::from(1u64),
        ..MockProvider::default()
    };
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();

    let err = bundler
        .build_signed_user_ops_tx(
            vec![happy_op()],
            BuildOpts {
                chain_id: TEST_CHAIN_ID,
                submit: true,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("insufficient deposit"), "got: {err}");
}

/// Each `build_signed_user_ops_tx` call must broadcast the signed tx via
/// `send_raw_transaction` before returning. The bundler owns submission so
/// the chain - not an internal counter - is the source of truth for the
/// next nonce.
#[tokio::test]
async fn build_submits_raw_transaction_to_provider() {
    let provider = MockProvider::default();
    let submissions = provider.submissions.clone();
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();
    let opts = BuildOpts {
        chain_id: TEST_CHAIN_ID,
        submit: true,
    };

    let resp = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts)
        .await
        .unwrap();

    let captured = submissions.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0], resp.raw);
}

/// With `submit: false` the bundler signs the envelope and returns it
/// without calling `send_raw_transaction`.
#[tokio::test]
async fn sign_only_does_not_broadcast() {
    let provider = MockProvider::default();
    let submissions = provider.submissions.clone();
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();

    let resp = bundler
        .build_signed_user_ops_tx(
            vec![happy_op()],
            BuildOpts {
                chain_id: TEST_CHAIN_ID,
                submit: false,
            },
        )
        .await
        .unwrap();

    assert!(
        submissions.lock().unwrap().is_empty(),
        "sign-only path must not call send_raw_transaction"
    );
    // The response is still complete: raw is what the caller forwards onward,
    // hash is the eventual on-chain hash they can poll for.
    assert_eq!(resp.chain_id, TEST_CHAIN_ID);
    assert!(!resp.raw.is_empty());
    let envelope =
        TxEnvelope::decode_2718(&mut resp.raw.as_ref()).expect("raw must be EIP-2718-encoded");
    assert_eq!(*envelope.tx_hash(), resp.hash);
}

/// Sign-only builds always sign at the execution layer's pending nonce. When a
/// reserved nonce is released after an abort, the pending nonce drops back and
/// the next build reuses it, so an aborted cross-chain leg never wedges the
/// signer behind a permanent gap.
#[tokio::test]
async fn sign_only_follows_builder_pending() {
    let provider = MockProvider::default();
    let pending = provider.pending_nonce.clone();
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();
    let opts = BuildOpts {
        chain_id: TEST_CHAIN_ID,
        submit: false,
    };

    let first = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts.clone())
        .await
        .unwrap();
    assert_eq!(decode_eip1559_nonce(&first.raw), TEST_NONCE);

    // The leg is reserved: pending advances.
    pending.store(TEST_NONCE + 1, Ordering::Relaxed);
    let second = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts.clone())
        .await
        .unwrap();
    assert_eq!(decode_eip1559_nonce(&second.raw), TEST_NONCE + 1);

    // The cross-chain xT aborts: the reservation is freed and pending drops.
    pending.store(TEST_NONCE, Ordering::Relaxed);
    let third = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts)
        .await
        .unwrap();
    assert_eq!(decode_eip1559_nonce(&third.raw), TEST_NONCE);
}

/// A failed broadcast surfaces the error and records no submission. The next
/// build reads the pending nonce afresh, so a transient failure never leaves
/// the bundler stuck on a stale value.
#[tokio::test]
async fn send_failure_propagates_and_refetches_pending() {
    let provider = MockProvider::default();
    let fail = provider.fail_send.clone();
    let pending = provider.pending_nonce.clone();
    let submissions = provider.submissions.clone();
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();
    let opts = BuildOpts {
        chain_id: TEST_CHAIN_ID,
        submit: true,
    };

    fail.store(true, Ordering::Relaxed);
    bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts.clone())
        .await
        .expect_err("send failure should propagate");
    assert!(submissions.lock().unwrap().is_empty());

    fail.store(false, Ordering::Relaxed);
    pending.store(TEST_NONCE + 3, Ordering::Relaxed);
    let resp = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts)
        .await
        .unwrap();
    assert_eq!(decode_eip1559_nonce(&resp.raw), TEST_NONCE + 3);
}

/// A receipt timeout surfaces as an error. The next build reads the pending
/// nonce afresh, with no carried-over state from the timed-out attempt.
#[tokio::test]
async fn receipt_timeout_propagates_and_refetches_pending() {
    let provider = MockProvider::default();
    let timeout = provider.timeout_receipt.clone();
    let pending = provider.pending_nonce.clone();
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();
    let opts = BuildOpts {
        chain_id: TEST_CHAIN_ID,
        submit: true,
    };

    timeout.store(true, Ordering::Relaxed);
    let err = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts.clone())
        .await
        .expect_err("receipt timeout must surface as an error");
    assert!(
        err.to_string().contains("not found within timeout"),
        "got: {err}"
    );

    timeout.store(false, Ordering::Relaxed);
    pending.store(TEST_NONCE + 4, Ordering::Relaxed);
    let resp = bundler
        .build_signed_user_ops_tx(vec![happy_op()], opts)
        .await
        .unwrap();
    assert_eq!(decode_eip1559_nonce(&resp.raw), TEST_NONCE + 4);
}

/// `submit_lock` serializes concurrent submit-mode builds: each one reads the
/// pending nonce and broadcasts atomically, so racing calls draw distinct,
/// contiguous nonces rather than colliding on the same one.
#[tokio::test]
async fn submit_mode_serializes_concurrent_nonces() {
    const CONCURRENCY: u64 = 8;

    let provider = MockProvider::default();
    provider
        .advance_pending_on_send
        .store(true, Ordering::Relaxed);
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Arc::new(Bundler::new(cfg, Arc::new(provider), signer).unwrap());

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let bundler = bundler.clone();
            tokio::spawn(async move {
                bundler
                    .build_signed_user_ops_tx(
                        vec![happy_op()],
                        BuildOpts {
                            chain_id: TEST_CHAIN_ID,
                            submit: true,
                        },
                    )
                    .await
                    .unwrap()
            })
        })
        .collect();

    let mut nonces = Vec::with_capacity(CONCURRENCY as usize);
    for handle in handles {
        nonces.push(decode_eip1559_nonce(&handle.await.unwrap().raw));
    }
    nonces.sort_unstable();

    let expected: Vec<u64> = (TEST_NONCE..TEST_NONCE + CONCURRENCY).collect();
    assert_eq!(nonces, expected, "nonces must be distinct and contiguous");
}

fn decode_eip1559_nonce(raw: &Bytes) -> u64 {
    let envelope =
        TxEnvelope::decode_2718(&mut raw.as_ref()).expect("raw must be EIP-2718-encoded");
    match envelope {
        TxEnvelope::Eip1559(signed) => signed.tx().nonce,
        other => panic!("expected EIP-1559 envelope, got {other:?}"),
    }
}

// Convenience for the test: expose the signer's address without dragging in
// the public Signer trait import in every test body.
trait LocalSignerExt {
    fn address_alias(&self) -> Address;
}
impl LocalSignerExt for LocalSigner {
    fn address_alias(&self) -> Address {
        use ethera_bundler_core::signer::Signer;
        Signer::address(self)
    }
}
