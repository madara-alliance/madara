// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Proxy} from "src/starkware/solidity/upgrade/Proxy.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import "./libraries/DataTypes.sol";
import {ProxySetup} from "./libraries/ProxySetup.sol";
import {FactoryStorage} from "./FactoryStorage.sol";

import {IOperator} from "./interfaces/IOperator.sol";
import {IGovernor} from "./interfaces/IGovernor.sol";
import {IProxyRoles} from "./interfaces/IProxyRoles.sol";

// Avoiding having anything in storage to make it cheaper
contract Factory is Ownable, Pausable, FactoryStorage {
  constructor(
    address owner,
    ImplementationContracts memory _implementationContracts
  ) Ownable(owner) {
    implementationContracts = _implementationContracts;
  }

  event ImplementationContractsUpdated(
    ImplementationContracts implementationContracts
  );

  function updateImplementations(
    ImplementationContracts calldata _implementationContracts
  ) public onlyOwner whenNotPaused {
    implementationContracts = _implementationContracts;

    emit ImplementationContractsUpdated(implementationContracts);
  }

  function getImplementations()
    public
    view
    returns (ImplementationContracts memory)
  {
    return implementationContracts;
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
    ) = deployManagerRegistry();

    // Deploy and setup MuiltiBridge
    address multiBridgeProxy = setupMultiBridge(
      implementationContracts.multipBridge,
      address(baseLayerContracts.manager),
      address(baseLayerContracts.coreContract)
    );

    baseLayerContracts.ethTokenBridge = setupEthBridge(
      implementationContracts.ethBridge,
      address(baseLayerContracts.manager),
      address(baseLayerContracts.coreContract),
      implementationContracts.ethBridgeEIC
    );

    // Setting up the Registry
    setupRegistry(
      baseLayerContracts.registry,
      implementationContracts.registry,
      address(baseLayerContracts.manager)
    );

    setupManager(
      baseLayerContracts.manager,
      implementationContracts.manager,
      multiBridgeProxy,
      address(baseLayerContracts.registry)
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
    // [sub_contracts addresses, eic address, initData].
    // In case of Starknet.sol the initData looks like
    //
    // (
    //    uint256 programHash_,
    //    uint256 aggregatorProgramHash_,
    //    address verifier_,
    //    uint256 configHash_,
    //    StarknetState.State memory initialState
    // ) = abi.decode(data, (uint256, uint256, address, uint256, StarknetState.State));
    bytes memory initData = abi.encode(address(0), coreContractInitData);
    ProxySetup.addImplementationAndUpgrade(
      address(coreContractProxy),
      coreContractImplementation,
      initData
    );

    IOperator(address(coreContractProxy)).registerOperator(operator);
    IGovernor(address(coreContractProxy)).starknetNominateNewGovernor(governor);
    IProxyRoles(address(coreContractProxy)).registerGovernanceAdmin(governor);

    return address(coreContractProxy);
  }

  function deployManagerRegistry() public returns (address, address) {
    _requireNotPaused();
    Proxy managerProxy = new Proxy(0);
    Proxy registryProxy = new Proxy(0);

    return (address(managerProxy), address(registryProxy));
  }

  function setupMultiBridge(
    address multiBridgeImplementation,
    address managerProxy,
    address messagingContract // coreContract
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

    return address(multiBridgeProxy);
  }

  function setupEthBridge(
    address ethBridgeImplementation,
    address manager,
    address messagingContract, // coreContractProxy
    address eicContract
  ) public returns (address) {
    _requireNotPaused();
    Proxy ethBridgePxoxy = new Proxy(0);

    bytes memory initData = abi.encode(eicContract, manager, messagingContract);
    ProxySetup.addImplementationAndUpgrade(
      address(ethBridgePxoxy),
      ethBridgeImplementation,
      initData
    );
    return address(ethBridgePxoxy);
  }

  function setupRegistry(
    address registryProxy,
    address registryImplementation,
    address managerProxy
  ) public {
    _requireNotPaused();
    bytes memory initData = abi.encode(address(0), managerProxy);
    ProxySetup.addImplementationAndUpgrade(
      address(registryProxy),
      registryImplementation,
      initData
    );
  }

  function setupManager(
    address managerProxy,
    address managerImplementation,
    address bridgeProxy,
    address registryProxy
  ) public {
    _requireNotPaused();
    bytes memory initData = abi.encode(address(0), bridgeProxy, registryProxy);
    ProxySetup.addImplementationAndUpgrade(
      managerProxy,
      managerImplementation,
      initData
    );
  }

  function pause() external onlyOwner {
    _pause();
  }

  function unpasuse() external onlyOwner {
    _unpause();
  }
}
