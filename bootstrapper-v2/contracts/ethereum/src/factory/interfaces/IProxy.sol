// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IProxy {
  function safeAddImplementation(
    address newImplementation,
    bytes calldata data
  ) external;

  function upgradeTo(address newImplementation, bytes calldata data) external;
}
