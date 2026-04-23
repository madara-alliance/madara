// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {IFactRegistry} from "./IFactRegistry.sol";

/// @title MockGpsVerifier
/// @notice A trust-free fact registry for mocknet / dev use.
///         Exposes the IFactRegistry interface so the Starknet core contract
///         can point at it as its verifier. Anyone can call `registerFact`;
///         we do NOT verify a proof alongside the fact. Use only in
///         environments where proof validity does not matter.
/// @dev DO NOT DEPLOY TO ENVIRONMENTS HANDLING REAL VALUE.
contract MockGpsVerifier is IFactRegistry {
    mapping(bytes32 => bool) private _facts;

    event FactRegistered(bytes32 indexed fact);

    /// @notice Mark a fact as valid. Idempotent.
    /// @param fact The keccak256 fact hash to register.
    function registerFact(bytes32 fact) external {
        if (!_facts[fact]) {
            _facts[fact] = true;
            emit FactRegistered(fact);
        }
    }

    /// @inheritdoc IFactRegistry
    function isValid(bytes32 fact) external view override returns (bool) {
        return _facts[fact];
    }
}
