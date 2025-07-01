// SPDX-License-Identifier: Apache-2.0.
pragma solidity 0.8.20;

import "src/starkware/solidity/components/GovernanceStub.sol";
import "src/starkware/solidity/interfaces/ProxySupport.sol";

/**
  Implements the ProxySupport required code for the trivial case
  of a proxied contract that has no initialization and no sub-contracts.
  It can also be used for non-trivial cases simply by overriding what's needed.
*/
abstract contract ProxySupportImpl is ProxySupport, GovernanceStub {
    function validateInitData(bytes calldata data) internal view virtual override {
        require(data.length == 0, "ILLEGAL_DATA_SIZE");
    }

    function initializeContractState(bytes calldata data) internal virtual override {
        require(data.length == 0, "UNEXPECTED_DATA");
    }

    function isInitialized() internal view virtual override returns (bool) {
        return true;
    }

    function processSubContractAddresses(bytes calldata subContractAddresses)
        internal
        virtual
        override
    {
        require(subContractAddresses.length == 0, "UNEXPECTED_DATA");
    }

    function numOfSubContracts() internal pure virtual override returns (uint256) {
        return 0;
    }
}
