// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IRoles {
  function registerGovernanceAdmin(address account) external;

  function registerSecurityAdmin(address account) external;
}
