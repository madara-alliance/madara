// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {Factory} from "src/factory/Factory.sol";
import "src/factory/libraries/DataTypes.sol";

contract FactoryTest is Test {
  Factory public factory;

  function setUp() public {
    ImplementationContracts
      memory implementationContracts = ImplementationContracts({
        coreContract: address(0x009c65f85425c619a6cb6d29ff8d57ef696323d188),
        manager: address(0x007cf4be31f546c04787886358b9486ca3d62b9acf),
        registry: address(0x0033e45b187da34826abceda1039231be46f1b05af),
        multiBridge: address(0x000c626fc4a447b01554518550e30600136864640b),
        ethBridge: address(0x00a21ddc1f17df41589bc6a5209292aed2df61cc94),
        ethBridgeEIC: address(0x002a590c461db46bca129e8dbe5c3998a8ff402e76)
      });
    factory = new Factory(address(this), implementationContracts);
  }

  function test_setup() public {
    CoreContractInitData memory coreContractInitData = CoreContractInitData({
      programHash: 1,
      aggregatorProgramHash: 0,
      verifier: address(0),
      configHash: 0,
      state: State({globalRoot: 0, blockNumber: 0, blockHash: 0})
    });

    factory.setup(coreContractInitData, address(1), address(2));
  }
}
