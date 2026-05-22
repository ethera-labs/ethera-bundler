//! End-to-end pipeline tests against a mocked `EthProvider`. Exercise the fee
//! policy, prefund formula, simulation, signing, and EIP-2718 encoding.

use std::sync::Arc;

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
use ethera_bundler_core::provider::{EthProvider, ProviderError};
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
}

impl Default for MockProvider {
    fn default() -> Self {
        Self {
            deposit: U256::from(TEST_DEPOSIT),
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
        Ok(TEST_NONCE)
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
        .build_signed_user_ops_tx(vec![happy_op()], BuildOpts { chain_id: 999 })
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
    };
    let cfg = test_config();
    let signer = Arc::new(LocalSigner::from_key(TEST_KEY).unwrap());
    let bundler = Bundler::new(cfg, Arc::new(provider), signer).unwrap();

    let err = bundler
        .build_signed_user_ops_tx(
            vec![happy_op()],
            BuildOpts {
                chain_id: TEST_CHAIN_ID,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("insufficient deposit"), "got: {err}");
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
