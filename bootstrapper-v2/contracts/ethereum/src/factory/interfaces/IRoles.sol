// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IRoles {
  function registerGovernanceAdmin(address account) external;

  function registerUpgradeGovernor(address account) external;

  function registerAppRoleAdmin(address account) external;

  function registerAppGovernor(address account) external;

  function registerTokenAdmin(address account) external;

  function registerSecurityAdmin(address account) external;

  function registerSecurityAgent(address account) external;

  function registerOperator(address account) external;

  function revokeAppGovernor(address account) external;

  function revokeOperator(address account) external;

  function renounceRole(bytes32 role, address account) external;
}
