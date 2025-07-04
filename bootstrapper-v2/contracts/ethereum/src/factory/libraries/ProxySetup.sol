// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {IProxy} from "../interfaces/IProxy.sol";

library ProxySetup {
  function addImplementationAndUpgrade(
    address proxy,
    address implementation,
    bytes calldata initData
  ) public {
    IProxy(proxy).safeAddImplementation(implementation, initData);
    IProxy(proxy).upgradeTo(implementation, initData);
  }
}
