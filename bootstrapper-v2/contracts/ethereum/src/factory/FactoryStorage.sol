// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "./libraries/DataTypes.sol";

contract FactoryStorage {
  // Stores all the implementations Contracts to setup the contracts
  ImplementationContracts public implementationContracts;
}
