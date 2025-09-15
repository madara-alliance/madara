// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "./libraries/DataTypes.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";

abstract contract Implementations is Ownable, Pausable {
    // Stores all the implementations Contracts to setup the contracts
    ImplementationContracts public implementationContracts;

    event ImplementationContractsUpdated(ImplementationContracts implementationContracts);

    function updateImplementations(ImplementationContracts calldata _implementationContracts)
        public
        onlyOwner
        whenNotPaused
    {
        implementationContracts = _implementationContracts;

        emit ImplementationContractsUpdated(implementationContracts);
    }

    function getImplementations() public view returns (ImplementationContracts memory) {
        return implementationContracts;
    }
}
