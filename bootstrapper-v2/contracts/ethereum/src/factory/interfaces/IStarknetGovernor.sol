// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IStarknetGovernor {
  function starknetNominateNewGovernor(address newGovernor) external;
}
