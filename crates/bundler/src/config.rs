//! Bundler configuration. Parsed from CLI flags or `ETHERA_BUNDLER_*` env vars.

use std::net::SocketAddr;

use alloy::primitives::{Address, B256, U256};
use clap::Parser;

/// Canonical ERC-4337 v0.7 `EntryPoint` address ([eth-infinitism/account-abstraction v0.7.0]).
///
/// [eth-infinitism/account-abstraction v0.7.0]:
///     https://github.com/eth-infinitism/account-abstraction/releases/tag/v0.7.0
pub const ENTRYPOINT_V07: Address = Address::new([
    0x00, 0x00, 0x00, 0x00, 0x71, 0x72, 0x7d, 0xe2, 0x2e, 0x5e, 0x9d, 0x8b, 0xaf, 0x0e, 0xda, 0xc6,
    0xf3, 0x7d, 0xa0, 0x32,
]);

#[derive(Debug, Clone, Parser)]
#[command(
    name = "ethera-bundler",
    version,
    about = "Ethera ERC-4337 v0.7 bundler"
)]
pub struct BundlerConfig {
    /// JSON-RPC server listen address.
    #[arg(
        long,
        env = "ETHERA_BUNDLER_LISTEN_ADDR",
        default_value = "0.0.0.0:8082"
    )]
    pub listen_addr: SocketAddr,

    /// Chain ID the bundler signs for. Must match `EXEC_RPC_URL`.
    #[arg(long, env = "ETHERA_BUNDLER_CHAIN_ID")]
    pub chain_id: u64,

    /// Execution-layer JSON-RPC URL (op-rbuilder or any compatible client).
    #[arg(long, env = "ETHERA_BUNDLER_EXEC_RPC_URL")]
    pub exec_rpc_url: String,

    /// `EntryPoint` contract address. Defaults to canonical v0.7.
    #[arg(long, env = "ETHERA_BUNDLER_ENTRYPOINT_ADDRESS", default_value_t = ENTRYPOINT_V07)]
    pub entrypoint_address: Address,

    /// `EntryPointSimulations` deployed bytecode, hex-encoded.
    ///
    /// Used as the `code` field of an `eth_call` state override so that
    /// `simulateValidation` can be invoked against the live `EntryPoint`
    /// address. v0.7 defines this method on a separate contract.
    #[arg(long, env = "ETHERA_BUNDLER_ENTRYPOINT_SIMULATIONS_CODE")]
    pub entrypoint_simulations_code: String,

    /// Sequencer signing key (32-byte hex, with or without `0x` prefix).
    #[arg(long, env = "ETHERA_BUNDLER_SEQUENCER_KEY")]
    pub sequencer_key: B256,

    /// Maximum number of `UserOperation`s per `ethera_buildSignedUserOpsTx` call.
    #[arg(long, env = "ETHERA_BUNDLER_MAX_BATCH_SIZE", default_value_t = 10)]
    pub max_batch_size: usize,

    /// Minimum priority fee the bundler accepts on a userop, in wei.
    ///
    /// Userops with `maxPriorityFeePerGas` below this are rejected with `-32602`.
    /// Default: 1 gwei.
    #[arg(long, env = "ETHERA_BUNDLER_MIN_PRIORITY_FEE_WEI", default_value_t = U256::from_limbs([1_000_000_000, 0, 0, 0]))]
    pub min_priority_fee_wei: U256,

    /// Safety margin added on top of `eth_estimateGas` for `handleOps`, in percent.
    #[arg(long, env = "ETHERA_BUNDLER_GAS_MARGIN_PCT", default_value_t = 10)]
    pub gas_margin_pct: u32,
}
