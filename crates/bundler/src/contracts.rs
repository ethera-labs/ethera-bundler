//! Solidity bindings for ERC-4337 v0.7 `EntryPoint` and `EntryPointSimulations`.
//!
//! See [eth-infinitism/account-abstraction v0.7.0][src] for canonical sources.
//!
//! [src]: https://github.com/eth-infinitism/account-abstraction/tree/v0.7.0/contracts/core

use alloy::sol;

sol! {
    /// Packed onchain encoding of a v0.7 `UserOperation`. The JSON-RPC wire format
    /// is *unpacked*; see [`crate::packing::pack`] for the conversion.
    #[derive(Debug)]
    struct PackedUserOperation {
        address sender;
        uint256 nonce;
        bytes initCode;
        bytes callData;
        bytes32 accountGasLimits;
        uint256 preVerificationGas;
        bytes32 gasFees;
        bytes paymasterAndData;
        bytes signature;
    }

    #[derive(Debug)]
    struct ReturnInfo {
        uint256 preOpGas;
        uint256 prefund;
        uint256 accountValidationData;
        uint256 paymasterValidationData;
        bytes paymasterContext;
    }

    #[derive(Debug)]
    struct StakeInfo {
        uint256 stake;
        uint256 unstakeDelay;
    }

    #[derive(Debug)]
    struct AggregatorStakeInfo {
        address aggregator;
        StakeInfo stakeInfo;
    }

    #[derive(Debug)]
    struct ValidationResult {
        ReturnInfo returnInfo;
        StakeInfo senderInfo;
        StakeInfo factoryInfo;
        StakeInfo paymasterInfo;
        AggregatorStakeInfo aggregatorInfo;
    }

    #[sol(rpc)]
    interface IEntryPoint {
        function balanceOf(address account) external view returns (uint256);
        function getUserOpHash(PackedUserOperation calldata userOp) external view returns (bytes32);
        function handleOps(PackedUserOperation[] calldata ops, address payable beneficiary) external;

        error FailedOp(uint256 opIndex, string reason);
        error FailedOpWithRevert(uint256 opIndex, string reason, bytes inner);
        error SignatureValidationFailed(address aggregator);
        error PostOpReverted(bytes returnData);
    }

    #[sol(rpc)]
    interface IEntryPointSimulations {
        function simulateValidation(PackedUserOperation calldata userOp) external returns (ValidationResult memory);
    }
}
