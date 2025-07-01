// SPDX-License-Identifier: Apache-2.0.
pragma solidity ^0.8.0;

import "src/starkware/solidity/libraries/RolesLib.sol";

struct GovernanceInfoStruct {
    mapping(address => bool) effectiveGovernors;
    address candidateGovernor;
    bool initialized;
}

// PROXY_GOVERNANCE_TAG = "StarkEx.Proxy.2019.GovernorsInformation"
// LEGACY_PROXY_GOVERNOR_SLOT = Web3.solidityKeccak(["string", "uint256"], [PROXY_GOVERNANCE_TAG, 0]) .
bytes32 constant LEGACY_PROXY_GOVERNOR_SLOT = 0x45f38e273862f8834bd2fe7a449988f63de55a7a5b685dea46ccedeb69cf0e26;

/**
  This contract allows the governance admin (which is the top of the `Roles` heirarchy),
  to override the proxy governance.
*/
abstract contract OverrideLegacyProxyGovernance {
    event LogNewGovernorAccepted(address acceptedGovernor);
    event LogRemovedGovernor(address removedGovernor);

    modifier GovernanceAdminOnly() {
        require(
            AccessControl.hasRole(GOVERNANCE_ADMIN, AccessControl._msgSender()),
            "GOVERNANCE_ADMIN_ONLY"
        );
        _;
    }

    function legacyProxyGovInfo() private pure returns (GovernanceInfoStruct storage gov) {
        bytes32 location = LEGACY_PROXY_GOVERNOR_SLOT;
        assembly {
            gov.slot := location
        }
    }

    /*
      Assigns `account` as proxy governor and clears pending govneror candidate.
    */
    function assignLegacyProxyGovernor(address account) external GovernanceAdminOnly {
        GovernanceInfoStruct storage legacyProxyGov = legacyProxyGovInfo();
        legacyProxyGov.effectiveGovernors[account] = true;
        delete legacyProxyGov.candidateGovernor;
        emit LogNewGovernorAccepted(account);
    }

    /*
      Removes `account` from proxy governor role and clears pending govneror candidate.
    */
    function removeLegacyProxyGovernor(address account) external GovernanceAdminOnly {
        GovernanceInfoStruct storage legacyProxyGov = legacyProxyGovInfo();
        legacyProxyGov.effectiveGovernors[account] = false;
        delete legacyProxyGov.candidateGovernor;
        emit LogRemovedGovernor(account);
    }
}
