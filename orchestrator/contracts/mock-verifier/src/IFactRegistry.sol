// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @notice Minimal IFactRegistry interface consumed by Starkware's Starknet core contract.
/// @dev Mirrors the shape at:
///   cairo-lang/src/starkware/solidity/interfaces/IFactRegistry.sol
interface IFactRegistry {
    /// @notice Returns true if the given fact was previously registered.
    function isValid(bytes32 fact) external view returns (bool);
}
