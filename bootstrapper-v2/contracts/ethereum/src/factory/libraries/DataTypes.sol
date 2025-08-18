// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

struct ImplementationContracts {
  address coreContract;
  address manager;
  address registry;
  address multiBridge;
  address ethBridge;
  address ethBridgeEIC;
}

struct BaseLayerContracts {
  address coreContract;
  address manager;
  address registry;
  address tokenBridge;
  address ethTokenBridge;
}

struct State {
  uint256 globalRoot;
  int256 blockNumber;
  uint256 blockHash;
}

struct CoreContractInitData {
  uint256 programHash;
  uint256 aggregatorProgramHash;
  address verifier;
  uint256 configHash;
  State state;
}
