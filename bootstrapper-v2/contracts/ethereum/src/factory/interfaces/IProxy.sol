// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IProxy {
  function addImplementation(
    address newImplementation,
    bytes calldata data,
    bool finalize
  ) external;

  function upgradeTo(
    address newImplementation,
    bytes calldata data,
    bool finalize
  ) external;
}
