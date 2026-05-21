# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

ERC-4337 v0.7 sequencer-sponsored bundler for Ethera rollups. The JSON-RPC method `ethera_buildSignedUserOpsTx` takes a
batch of unpacked `UserOperation`s and returns a sequencer-signed EIP-1559 transaction wrapping `EntryPoint.handleOps` -
the caller is responsible for `eth_sendRawTransaction`. The sequencer EOA is both signer and `handleOps` beneficiary (it
fronts gas and gets reimbursed from each op's EntryPoint deposit in the same tx).

## Commands

Toolchain pinned to Rust 1.91 via `rust-toolchain.toml`. Use the Makefile targets:

- `make build` / `make build-debug` - workspace build
- `make test` - `cargo test --workspace --all-targets`
- `make fmt` / `make fmt-check` - `cargo fmt --all`
- `make lint` - `cargo clippy --workspace --all-targets -- -D warnings`
- `make pr` - full pre-PR gate (fmt-check + lint + test)
- `make run` - runs the bundler with `.env` auto-loaded
- `make deny` / `make machete` - supply-chain + unused-dep checks (also enforced by pre-commit)

Single test: `cargo test -p ethera-bundler-core <test_name>` (e.g., the integration tests live in
`crates/bundler/tests/bundler_pipeline.rs`).

First-time setup: `make install-tools && make install-hooks` to wire `cargo-deny`, `cargo-machete`, and `pre-commit`.

## Workspace layout

Two-crate Cargo workspace:

- `bin/ethera-bundler` - thin binary: clap config → `AlloyProvider` + `LocalSigner` → `jsonrpsee` server with a
  permissive CORS layer.
- `crates/bundler` (`ethera-bundler-core`) - all the logic. Modules: `bundler` (orchestration), `validator` (
  simulateValidation parsing), `packing` (unpacked↔packed UserOp), `contracts` (alloy `sol!` bindings + embedded
  simulations runtime), `provider` (EL trait + alloy impl), `signer` (EIP-1559 signing trait), `rpc` (`jsonrpsee` server
  module), `config`, `types`, `errors`.

## Architecture worth knowing before editing

**The request pipeline lives in `Bundler::build_signed_user_ops_tx`** (`crates/bundler/src/bundler.rs`). Steps run
per-op in this order, and the ordering matters:

1. Sanity (chainId match, batch size, per-op fee policy).
2. `packing::pack` - unpacked wire form → onchain `PackedUserOperation`.
3. `check_prefund` - full v0.7 formula `(verGas + callGas + pmVerGas + pmPostOpGas + preVerGas) × maxFeePerGas` vs.
   `EntryPoint.balanceOf(paymaster || sender)`. Paymaster is parsed out of `paymasterAndData` (first 20 bytes) - see
   `packing::parse_paymaster_and_data`.
4. `validator::simulate` - `IEntryPointSimulations.simulateValidation` via `eth_call` with a **state-override that swaps
   the EntryPoint runtime for the embedded simulations bytecode**. v0.7 moved this method out of the main contract; the
   override is the canonical workaround. The simulations bytecode is included from
   `crates/bundler/assets/entrypoint_simulations_v07.bin` via `ENTRYPOINT_SIMULATIONS_RUNTIME_V07` in `contracts.rs`.
   Regenerate with `forge inspect contracts/core/EntryPointSimulations.sol:EntryPointSimulations deployedBytecode` when
   bumping ERC-4337.
5. `getUserOpHash` per op.

Then once per batch:

6. ABI-encode `handleOps(ops, beneficiary = sequencer)`, `eth_estimateGas`, add `gas_margin_pct` safety margin.
7. **Outer-tx fee policy** (`choose_outer_fees`):
   `tip = min(max(min_priority_fee_wei, suggested), min(op.maxPriorityFeePerGas))`, `feeCap = min(op.maxFeePerGas)`
   across the batch. Rejected if `feeCap < baseFee + tip` (transient: network moved during the call).
8. Sign EIP-1559 with `Signer::sign_eip1559`, return raw + hash.

**`validator::parse_window`** decodes the v0.7 `_parseValidationData` layout:
`aggregator | (validUntil << 160) | (validAfter << 208)`. Special cases: `aggregator == address(1)` ⇒ signature failed;
`validUntil == 0` ⇒ saturate to `type(uint48).max`.

**Error mapping is centralized in `errors::classify`** (`crates/bundler/src/errors.rs`). It maps `BundlerError` →
`(code, data)` per the ERC-4337 RPC spec: `-32602` (invalid params / fee policy), `-32500` (`AA2x`), `-32501` (`AA3x`
paymaster), `-32503` (out-of-time-range), `-32521` (account revert / unsupported aggregator), `-32603` (
transport/sign/decode). When adding a new failure mode, extend `BundlerError` *and* `classify` together - the RPC layer
relies on this mapping.

**The `EthProvider` trait** (`crates/bundler/src/provider.rs`) is the seam for tests. Integration tests in
`crates/bundler/tests/bundler_pipeline.rs` inject a `MockProvider`; the real impl is `AlloyProvider`. Keep new RPC
interactions behind this trait so the pipeline stays mockable.

**Aggregator-signed userops are explicitly unsupported** (`ValidationError::UnsupportedAggregator`). Don't add
aggregator handling without coordinating on the broader policy.

## Conventions

- Workspace-wide lints in root `Cargo.toml` include `clippy::all` + a curated set of pedantic-ish lints (`use_self`,
  `uninlined_format_args`, `doc_markdown`, `semicolon_if_nothing_returned`, etc.). `make lint` enforces `-D warnings`.
- `rustfmt.toml` and `deny.toml` are the source of truth for formatting and supply-chain policy.
- All config flags double as `ETHERA_BUNDLER_*` env vars via clap `env =` (see `config.rs` and `.env.example`).
- Canonical EntryPoint v0.7 address is `ENTRYPOINT_V07` in `config.rs` - reuse rather than hard-coding.
