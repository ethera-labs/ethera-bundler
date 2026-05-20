# ethera-bundler

[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](./COPYING)
[![Rust 1.91](https://img.shields.io/badge/rust-1.91-orange.svg)](./rust-toolchain.toml)

An ERC-4337 v0.7 bundler for Ethera rollups. Accepts user-signed `UserOperation`s
over JSON-RPC, validates them against a live execution-layer node, and returns a
sequencer-signed EIP-1559 transaction calling `EntryPoint.handleOps` - ready for
`eth_sendRawTransaction`.

The sequencer EOA is both signer and `handleOps` beneficiary: it fronts the gas
on chain and gets reimbursed from each `UserOperation`'s EntryPoint deposit
inside the same transaction.

## Quick start

```sh
# Build
make build

# Configure
cp .env.example .env
$EDITOR .env                       # set CHAIN_ID, EXEC_RPC_URL, SEQUENCER_KEY, ENTRYPOINT_SIMULATIONS_CODE

# Run
set -a; source .env; set +a
./target/release/ethera-bundler
```

Or with Docker:

```sh
docker build -t ethera-bundler .
docker run --rm -p 8082:8082 --env-file .env ethera-bundler
```

## JSON-RPC

| Method                        | Params                                  | Returns        |
|-------------------------------|-----------------------------------------|----------------|
| `ethera_buildSignedUserOpsTx` | `[UserOperation[], { "chainId": u64 }]` | `SignedTxResp` |

`UserOperation` is the unpacked ERC-4337 v0.7 wire format (`sender`, `nonce`,
`callData`, `callGasLimit`, …). Either the packed form (`initCode`,
`paymasterAndData`) or the unpacked form (`factory`, `factoryData`,
`paymaster`, `paymasterVerificationGasLimit`, …) is accepted.

`SignedTxResp`:

```json
{
  "raw": "0x02f8…",
  "hash": "0x…",
  "to": "0x0000000071727De22E5E9d8BAf0edAc6f37da032",
  "chainId": 31337,
  "gas": "0x4c4b40",
  "maxFeePerGas": "0x12a05f200",
  "maxPriorityFeePerGas": "0x3b9aca00",
  "userOpHashes": [
    "0x…"
  ]
}
```

Errors follow the [ERC-4337 Bundler RPC spec][rpc-spec]:

| Code     | Meaning                                                      |
|----------|--------------------------------------------------------------|
| `-32602` | Invalid params (chain ID, batch size, fee policy, encoding)  |
| `-32500` | Rejected by `EntryPoint` validation (e.g. `AA21`, `AA24`)    |
| `-32501` | Rejected by paymaster (`AA34`)                               |
| `-32503` | `UserOperation` outside its `validAfter`/`validUntil` window |
| `-32521` | Reverted inside account validation                           |
| `-32603` | Internal error                                               |

[rpc-spec]: https://eips.ethereum.org/EIPS/eip-4337#bundler-rpc-api

## Configuration

All settings are exposed as both CLI flags and `ETHERA_BUNDLER_*` env vars. See
[`.env.example`](./.env.example) for the full set.

| Variable                                     | Default        | Description                                    |
|----------------------------------------------|----------------|------------------------------------------------|
| `ETHERA_BUNDLER_LISTEN_ADDR`                 | `0.0.0.0:8082` | JSON-RPC server bind address                   |
| `ETHERA_BUNDLER_CHAIN_ID`                    | *required*     | Chain ID the bundler signs for                 |
| `ETHERA_BUNDLER_EXEC_RPC_URL`                | *required*     | Execution-layer JSON-RPC endpoint              |
| `ETHERA_BUNDLER_ENTRYPOINT_ADDRESS`          | canonical v0.7 | `EntryPoint` contract address                  |
| `ETHERA_BUNDLER_ENTRYPOINT_SIMULATIONS_CODE` | *required*     | `EntryPointSimulations` runtime bytecode (hex) |
| `ETHERA_BUNDLER_SEQUENCER_KEY`               | *required*     | Sequencer EOA private key (32-byte hex)        |
| `ETHERA_BUNDLER_MAX_BATCH_SIZE`              | `10`           | Maximum `UserOperation`s per request           |
| `ETHERA_BUNDLER_MIN_PRIORITY_FEE_WEI`        | `1000000000`   | Minimum accepted `maxPriorityFeePerGas` (wei)  |
| `ETHERA_BUNDLER_GAS_MARGIN_PCT`              | `10`           | Safety margin over `eth_estimateGas` (percent) |

Run `ethera-bundler --help` for the full CLI surface.

## Pipeline

For each request:

1. **Sanity** - chain ID match, batch size ≤ `MAX_BATCH_SIZE`, per-op fee policy
   (`maxPriorityFeePerGas ≥ MIN_PRIORITY_FEE_WEI`, `maxFeePerGas ≥ baseFee + tip`).
2. **Pack** - wire-format → onchain `PackedUserOperation`.
3. **Simulate** - `IEntryPointSimulations.simulateValidation` via `eth_call`
   with a state override; checks signature, time range, and aggregator.
4. **Prefund** - `(verGas + callGas + pmVerGas + pmPostOpGas + preVerGas) × maxFeePerGas`
   against `EntryPoint.balanceOf(paymaster ?? sender)`.
5. **Hash** - `EntryPoint.getUserOpHash` per op for the response.
6. **Build** - ABI-encode `handleOps(ops, beneficiary = sequencer)`, estimate
   gas + margin.
7. **Choose outer fees** - `tip = min(max(MIN_PRIORITY_FEE_WEI, suggested), min(op.maxPriorityFeePerGas))`,
   `feeCap = min(op.maxFeePerGas)` across the batch.
8. **Sign** - EIP-1559 transaction, sequencer key, return raw + hash.

## Development

```sh
make build           # cargo build --workspace
make test            # cargo test --workspace --all-targets
make fmt             # cargo fmt --all
make lint            # cargo clippy --workspace --all-targets -- -D warnings
make pr              # fmt-check + lint + test (run before opening a PR)
```

Toolchain pinned to Rust 1.91 via [`rust-toolchain.toml`](./rust-toolchain.toml).
Workspace-wide lints (including `clippy::pedantic`-style rules) are configured
in the root `Cargo.toml`.

## License

[GNU General Public License v3.0](./COPYING).
