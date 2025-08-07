// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Proxy} from "src/starkware/solidity/upgrade/Proxy.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import "./libraries/DataTypes.sol";
import {ProxySetup} from "./libraries/ProxySetup.sol";
import {Implementations} from "./Implementations.sol";

import {IOperator} from "./interfaces/IOperator.sol";
import {IBridge} from "./interfaces/IBridge.sol";
import {IRoles} from "./interfaces/IRoles.sol";
import {IStarknetGovernor} from "./interfaces/IStarknetGovernor.sol";
import {IProxyRoles} from "./interfaces/IProxyRoles.sol";

contract Factory is Ownable, Pausable, Implementations {
  constructor(
    address owner,
    ImplementationContracts memory _implementationContracts
  ) Ownable(owner) {
    implementationContracts = _implementationContracts;
  }

  function setup(
    CoreContractInitData calldata coreContractInitData,
    address operator,
    address governor
  ) public returns (BaseLayerContracts memory baseLayerContracts) {
    _requireNotPaused();
    baseLayerContracts.coreContract = setupCoreContract(
      implementationContracts.coreContract,
      coreContractInitData,
      operator,
      governor
    );

    // Deploying both Manager and Registry together
    // as there cyclic dependency in their initalisers
    // so we need the address of both before before initialising them
    (
      baseLayerContracts.manager,
      baseLayerContracts.registry
    ) = deployManagerAndRegistry();

    // Deploy and setup MuiltiBridge
    address multiBridgeProxy = setupMultiBridge(
      implementationContracts.multipBridge,
      address(baseLayerContracts.manager),
      address(baseLayerContracts.coreContract),
      governor
    );

    baseLayerContracts.ethTokenBridge = setupEthBridge(
      implementationContracts.ethBridge,
      address(baseLayerContracts.manager),
      address(baseLayerContracts.coreContract),
      implementationContracts.ethBridgeEIC,
      governor
    );

    // Setting up the Registry
    setupRegistry(
      baseLayerContracts.registry,
      implementationContracts.registry,
      address(baseLayerContracts.manager),
      governor
    );

    setupManager(
      baseLayerContracts.manager,
      implementationContracts.manager,
      multiBridgeProxy,
      address(baseLayerContracts.registry),
      governor
    );

    return baseLayerContracts;
  }

  function setupCoreContract(
    address coreContractImplementation,
    CoreContractInitData calldata coreContractInitData,
    address operator,
    address governor
  ) public returns (address) {
    _requireNotPaused();
    // Deploying proxy with 0 upgradeActivationDelay
    Proxy coreContractProxy = new Proxy(0);
    // [sub_contracts_addresses[], eic address, initData].
    // In case of Starknet.sol the initData looks like
    //
    //``` (
    //    uint256 programHash_,
    //    uint256 aggregatorProgramHash_,
    //    address verifier_,
    //    uint256 configHash_,
    //    StarknetState.State memory initialState
    // ) = abi.decode(data, (uint256, uint256, address, uint256, StarknetState.State)); ```
    // The sub_contracts_addresses[] is an array of addresses of the sub_contracts
    // The above address is empty, while the eic address is 0 adderess.
    bytes memory upgradeData = abi.encode(address(0), coreContractInitData);
    ProxySetup.addImplementationAndUpgrade(
      address(coreContractProxy),
      coreContractImplementation,
      upgradeData
    );

    IOperator(address(coreContractProxy)).registerOperator(operator);
    IStarknetGovernor(address(coreContractProxy)).starknetNominateNewGovernor(governor);
    IProxyRoles(address(coreContractProxy)).registerGovernanceAdmin(governor);

    return address(coreContractProxy);
  }

  function deployManagerAndRegistry() public returns (address, address) {
    _requireNotPaused();
    Proxy managerProxy = new Proxy(0);
    Proxy registryProxy = new Proxy(0);

    return (address(managerProxy), address(registryProxy));
  }

  function setupMultiBridge(
    address multiBridgeImplementation,
    address managerProxy,
    address messagingContract, // coreContract
    address governor
  ) public returns (address) {
    _requireNotPaused();
    Proxy multiBridgeProxy = new Proxy(0);
    bytes memory initData = abi.encode(
      address(0),
      managerProxy,
      messagingContract
    );
    ProxySetup.addImplementationAndUpgrade(
      address(multiBridgeProxy),
      multiBridgeImplementation,
      initData
    );

    registerAdmins(address(multiBridgeProxy), governor);

    return address(multiBridgeProxy);
  }

  function setupEthBridge(
    address ethBridgeImplementation,
    address manager,
    address messagingContract, // coreContractProxy
    address eicContract,
    address governor
  ) public returns (address) {
    _requireNotPaused();
    Proxy ethBridgePxoxy = new Proxy(0);

    bytes memory initData = abi.encode(eicContract, manager, messagingContract);
    ProxySetup.addImplementationAndUpgrade(
      address(ethBridgePxoxy),
      ethBridgeImplementation,
      initData
    );

    registerAdmins(address(ethBridgePxoxy), governor);
    return address(ethBridgePxoxy);
  }

  function setupRegistry(
    address registryProxy,
    address registryImplementation,
    address managerProxy,
    address governor
  ) public whenNotPaused {
    bytes memory initData = abi.encode(address(0), managerProxy);
    ProxySetup.addImplementationAndUpgrade(
      address(registryProxy),
      registryImplementation,
      initData
    );

    registerAdmins(registryProxy, governor);
  }

  function setupManager(
    address managerProxy,
    address managerImplementation,
    address bridgeProxy,
    address registryProxy,
    address governor
  ) public whenNotPaused {
    bytes memory upgradeData = abi.encode(address(0), registryProxy, bridgeProxy);
    ProxySetup.addImplementationAndUpgrade(
      managerProxy,
      managerImplementation,
      upgradeData
    );
    registerAdmins(managerProxy, governor);
  }

  // Ensure to remove the governance admin role post setup,
  // As this function can be rerun 
  function set_l2_bridge(
    uint256 l2_eth_bridge_address,
    uint256 l2_erc20_bridge_address,
    address ethTokenBridge,
    address tokenBridge
  ) onlyOwner whenNotPaused public {
    IRoles(ethTokenBridge).registerAppRoleAdmin(address(this));
    IRoles(ethTokenBridge).registerAppGovernor(address(this));
    IBridge(ethTokenBridge).setL2TokenBridge(l2_eth_bridge_address);
    IRoles(ethTokenBridge).revokeAppGovernor(address(this));
    IRoles(ethTokenBridge).revokeAppRoleAdmin(address(this));

    IRoles(tokenBridge).registerAppRoleAdmin(address(this));
    IRoles(tokenBridge).registerAppGovernor(address(this));
    IBridge(tokenBridge).setL2TokenBridge(l2_erc20_bridge_address);
    IRoles(tokenBridge).revokeAppGovernor(address(this));
    IRoles(tokenBridge).revokeAppRoleAdmin(address(this));
  }

  function registerAdmins(
    address proxyContract,
    address governanceAdmin
  ) private {
    IRoles(proxyContract).registerGovernanceAdmin(governanceAdmin);
    IRoles(proxyContract).registerSecurityAdmin(governanceAdmin);
    IRoles(proxyContract).registerUpgradeGovernor(governanceAdmin);
    IRoles(proxyContract).registerAppRoleAdmin(governanceAdmin);
    IRoles(proxyContract).registerAppGovernor(governanceAdmin);
    IRoles(proxyContract).registerUpgradeGovernor(governanceAdmin);
  }

  function pause() external onlyOwner {
    _pause();
  }

  function unpasuse() external onlyOwner {
    _unpause();
  }
}
