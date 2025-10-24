// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IProxyRoles {
  function registerGovernanceAdmin(address account) external;

  function registerAppRoleAdmin(address account) external;

  function revokeAppGovernor(address account) external;
}
