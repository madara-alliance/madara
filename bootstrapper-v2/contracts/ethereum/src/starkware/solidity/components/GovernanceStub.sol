// SPDX-License-Identifier: Apache-2.0.
pragma solidity ^0.8.0;

import "src/starkware/solidity/interfaces/MGovernance.sol";

/*
  Implements Governance stub to inherit correctly from MGovernance for contracts that
  do not implement Governance. (e.g. CallProxy).
*/
abstract contract GovernanceStub is MGovernance {
    function initGovernance() internal override {}

    function _isGovernor(
        address /*user*/
    ) internal pure override returns (bool) {
        return false;
    }
}
