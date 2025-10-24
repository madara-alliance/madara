// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {Factory} from "src/factory/Factory.sol";
import "src/factory/libraries/DataTypes.sol";
import {ConfigureSingleBridgeEIC} from "src/eic/ConfigureSingleBridgeEIC.sol";

// Currently to use this test, 
// 1. Run anvil. 
// 2. Run the setup-base layer command from the rust cli 
// 3. Run the test by using forge test (the hardcoded addresses are what will be deployed on anvil). 
// Ideally it would have been better that we could do implementation contract deployment too in foundry test, 
// but anyways we are not writing the test for the implementation contract but for the factory, so this works somehow.
// 
// One otherway to skip running anvil and setup-base command is to test using sepolia fork too, 
// where we can have the implementations pre-deployed
contract FactoryTest is Test {
  Factory public factory;

  function setUp() public {
    ConfigureSingleBridgeEIC ethBridgeEIC = new ConfigureSingleBridgeEIC();
    ImplementationContracts
      memory implementationContracts = ImplementationContracts({
        ethBridgeEIC: address(ethBridgeEIC),
        registry: address(0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0),
        multiBridge: address(0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9),
        coreContract: address(0x5FbDB2315678afecb367f032d93F642f64180aa3),
        manager: address(0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512),
        ethBridge: address(0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9)
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

    BaseLayerContracts memory baselayer = factory.setup(coreContractInitData, address(1), address(2));
    factory.setL2Bridge(1, 2, baselayer.ethTokenBridge, baselayer.tokenBridge);
  }
}
