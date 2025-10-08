// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Proxy} from "src/starkware/solidity/upgrade/Proxy.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import "./libraries/DataTypes.sol";
import {Implementations} from "./Implementations.sol";
import {IProxy} from "./interfaces/IProxy.sol";

import {IOperator} from "./interfaces/IOperator.sol";
import {IBridge} from "./interfaces/IBridge.sol";
import {IRoles} from "./interfaces/IRoles.sol";
import {IStarknetGovernor} from "./interfaces/IStarknetGovernor.sol";
import {IProxyRoles} from "./interfaces/IProxyRoles.sol";

// int.from_bytes(Web3.keccak(text="ROLE_APP_ROLE_ADMIN"), "big") & MASK_250 .
bytes32 constant APP_ROLE_ADMIN = bytes32(
    uint256(0x03e615638e0b79444a70f8c695bf8f2a47033bf1cf95691ec3130f64939cee99)
);
contract Factory is Ownable, Pausable, Implementations {
  event BaseLayerContractsDeployed(BaseLayerContracts _baseLayerContracts);

  constructor(
    address owner,
    ImplementationContracts memory _implementationContracts
  ) Ownable(owner) {
    implementationContracts = _implementationContracts;
  }

  function addImplementationAndUpgrade(
    address proxy,
    address implementation,
    bytes memory initData
  ) public {
    IRoles(proxy).registerUpgradeGovernor(address(this));
    IProxy(proxy).addImplementation(implementation, initData, false);
    IProxy(proxy).upgradeTo(implementation, initData, false);
    IRoles(proxy).revokeUpgradeGovernor(address(this));
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
    // as there is a cyclic dependency in their initalisers
    // so we need the address of both before before initialising them
    (
      baseLayerContracts.manager,
      baseLayerContracts.registry
    ) = deployManagerAndRegistry();

    // Deploy and setup MuiltiBridge
    baseLayerContracts.tokenBridge = setupMultiBridge(
      implementationContracts.multiBridge,
      address(baseLayerContracts.manager),
      address(baseLayerContracts.coreContract),
      governor
    );

    baseLayerContracts.ethTokenBridge = setupEthBridge(
      implementationContracts.ethBridge,
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
      baseLayerContracts.tokenBridge,
      address(baseLayerContracts.registry),
      governor
    );

    emit BaseLayerContractsDeployed(baseLayerContracts);
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

    addImplementationAndUpgrade(
      address(coreContractProxy),
      coreContractImplementation,
      upgradeData
    );

    IStarknetGovernor(address(coreContractProxy)).starknetNominateNewGovernor(
      governor
    );
    IProxyRoles(address(coreContractProxy)).registerGovernanceAdmin(governor);
    IProxyRoles(address(coreContractProxy)).registerAppRoleAdmin(address(this));
    IOperator(address(coreContractProxy)).registerOperator(operator);

    return address(coreContractProxy);
  }

  function deployManagerAndRegistry() public returns (address, address) {
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

    addImplementationAndUpgrade(
      address(multiBridgeProxy),
      multiBridgeImplementation,
      initData
    );

    registerAdmins(address(multiBridgeProxy), governor);

    return address(multiBridgeProxy);
  }

  function setupEthBridge(
    address ethBridgeImplementation,
    address messagingContract, // coreContractProxy
    address eicContract,
    address governor
  ) public returns (address) {
    _requireNotPaused();
    Proxy ethBridgePxoxy = new Proxy(0);
    // 'eth' is 0x657468
    bytes memory initData = abi.encode(
      eicContract,
      messagingContract,
      0x657468,
      address(0)
    );

    addImplementationAndUpgrade(
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
  ) public {
    _requireNotPaused();
    bytes memory initData = abi.encode(address(0), managerProxy);
    addImplementationAndUpgrade(
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
  ) private {
    bytes memory upgradeData = abi.encode(
      address(0),
      registryProxy,
      bridgeProxy
    );
    addImplementationAndUpgrade(
      managerProxy,
      managerImplementation,
      upgradeData
    );
    registerAdmins(managerProxy, governor);
  }

  // Ensure to remove the governance admin role post setup,
  // As this function can be rerun
  function setL2Bridge(
    uint256 l2EthBridgeAddress,
    uint256 l2Erc20BridgeAddress,
    address ethTokenBridge,
    address tokenBridge
  ) public onlyOwner {
    _requireNotPaused();
    IRoles(ethTokenBridge).registerAppRoleAdmin(address(this));
    IRoles(ethTokenBridge).registerAppGovernor(address(this));
    IBridge(ethTokenBridge).setL2TokenBridge(l2EthBridgeAddress);
    IRoles(ethTokenBridge).revokeAppGovernor(address(this));
    IRoles(ethTokenBridge).renounceRole(APP_ROLE_ADMIN, address(this));

    IRoles(tokenBridge).registerAppRoleAdmin(address(this));
    IRoles(tokenBridge).registerAppGovernor(address(this));
    IBridge(tokenBridge).setL2TokenBridge(l2Erc20BridgeAddress);
    IRoles(tokenBridge).revokeAppGovernor(address(this));
    IRoles(tokenBridge).renounceRole(APP_ROLE_ADMIN, address(this));
  }

  function registerAdmins(
    address proxyContract,
    address governanceAdmin
  ) private {
    IRoles(proxyContract).registerGovernanceAdmin(governanceAdmin);
    IRoles(proxyContract).registerSecurityAdmin(governanceAdmin);
    IRoles(proxyContract).registerUpgradeGovernor(governanceAdmin);
    IRoles(proxyContract).registerAppRoleAdmin(governanceAdmin);
    IRoles(proxyContract).registerAppRoleAdmin(address(this));
    IRoles(proxyContract).registerAppGovernor(governanceAdmin);
  }
}
