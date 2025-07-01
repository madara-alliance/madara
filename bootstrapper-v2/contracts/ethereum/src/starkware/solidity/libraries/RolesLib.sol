// SPDX-License-Identifier: Apache-2.0.
pragma solidity ^0.8.0;

import "src/starkware/solidity/libraries/AccessControl.sol";

// int.from_bytes(Web3.keccak(text="ROLE_APP_GOVERNOR"), "big") & MASK_250 .
bytes32 constant APP_GOVERNOR = bytes32(
    uint256(0xd2ead78c620e94b02d0a996e99298c59ddccfa1d8a0149080ac3a20de06068)
);

// int.from_bytes(Web3.keccak(text="ROLE_APP_ROLE_ADMIN"), "big") & MASK_250 .
bytes32 constant APP_ROLE_ADMIN = bytes32(
    uint256(0x03e615638e0b79444a70f8c695bf8f2a47033bf1cf95691ec3130f64939cee99)
);

// int.from_bytes(Web3.keccak(text="ROLE_GOVERNANCE_ADMIN"), "big") & MASK_250 .
bytes32 constant GOVERNANCE_ADMIN = bytes32(
    uint256(0x03711c9d994faf6055172091cb841fd4831aa743e6f3315163b06a122c841846)
);

// int.from_bytes(Web3.keccak(text="ROLE_OPERATOR"), "big") & MASK_250 .
bytes32 constant OPERATOR = bytes32(
    uint256(0x023edb77f7c8cc9e38e8afe78954f703aeeda7fffe014eeb6e56ea84e62f6da7)
);

// int.from_bytes(Web3.keccak(text="ROLE_SECURITY_ADMIN"), "big") & MASK_250 .
bytes32 constant SECURITY_ADMIN = bytes32(
    uint256(0x026bd110619d11cfdfc28e281df893bc24828e89177318e9dbd860cdaedeb6b3)
);

// int.from_bytes(Web3.keccak(text="ROLE_SECURITY_AGENT"), "big") & MASK_250 .
bytes32 constant SECURITY_AGENT = bytes32(
    uint256(0x037693ba312785932d430dccf0f56ffedd0aa7c0f8b6da2cc4530c2717689b96)
);

// int.from_bytes(Web3.keccak(text="ROLE_TOKEN_ADMIN"), "big") & MASK_250 .
bytes32 constant TOKEN_ADMIN = bytes32(
    uint256(0x0128d63adbf6b09002c26caf55c47e2f26635807e3ef1b027218aa74c8d61a3e)
);

// int.from_bytes(Web3.keccak(text="ROLE_UPGRADE_GOVERNOR"), "big") & MASK_250 .
bytes32 constant UPGRADE_GOVERNOR = bytes32(
    uint256(0x0251e864ca2a080f55bce5da2452e8cfcafdbc951a3e7fff5023d558452ec228)
);

/*
  Role                |   Role Admin
  ----------------------------------------
  GOVERNANCE_ADMIN    |   GOVERNANCE_ADMIN
  UPGRADE_GOVERNOR    |   GOVERNANCE_ADMIN
  APP_ROLE_ADMIN      |   GOVERNANCE_ADMIN
  APP_GOVERNOR        |   APP_ROLE_ADMIN
  OPERATOR            |   APP_ROLE_ADMIN
  TOKEN_ADMIN         |   APP_ROLE_ADMIN
  SECURITY_ADMIN      |   SECURITY_ADMIN
  SECURITY_AGENT      |   SECURITY_ADMIN .
*/
library RolesLib {
    // INITIALIZERS.
    function governanceRolesInitialized() internal view returns (bool) {
        return AccessControl.getRoleAdmin(GOVERNANCE_ADMIN) != bytes32(0x00);
    }

    function securityRolesInitialized() internal view returns (bool) {
        return AccessControl.getRoleAdmin(SECURITY_ADMIN) != bytes32(0x00);
    }

    function initialize() internal {
        address provisional = AccessControl._msgSender();
        initialize(provisional, provisional);
    }

    function initialize(address provisionalGovernor, address provisionalSecAdmin) internal {
        initialize(provisionalGovernor, provisionalSecAdmin, false);
    }

    function initialize(
        address provisionalGovernor,
        address provisionalSecAdmin,
        bool subGovernors
    ) internal {
        if (governanceRolesInitialized()) {
            // Support Proxied contract initialization.
            // In case the Proxy already initialized the roles,
            // init will succeed IFF the provisionalGovernor is already `GovernanceAdmin`.
            require(
                AccessControl.hasRole(GOVERNANCE_ADMIN, provisionalGovernor),
                "ROLES_ALREADY_INITIALIZED"
            );
        } else {
            initGovernanceRoles(provisionalGovernor, subGovernors);
        }

        if (securityRolesInitialized()) {
            // If SecurityAdmin initialized,
            // then provisionalSecAdmin must already be a `SecurityAdmin`.
            // If it's not initilized - initialize it.
            require(
                AccessControl.hasRole(SECURITY_ADMIN, provisionalSecAdmin),
                "SECURITY_ROLES_ALREADY_INITIALIZED"
            );
        } else {
            initSecurityRoles(provisionalSecAdmin);
        }
    }

    function initSecurityRoles(address provisionalSecAdmin) private {
        AccessControl._setRoleAdmin(SECURITY_ADMIN, SECURITY_ADMIN);
        AccessControl._setRoleAdmin(SECURITY_AGENT, SECURITY_ADMIN);
        AccessControl._grantRole(SECURITY_ADMIN, provisionalSecAdmin);
    }

    function initGovernanceRoles(address provisionalGovernor, bool subGovernors) private {
        AccessControl._grantRole(GOVERNANCE_ADMIN, provisionalGovernor);
        AccessControl._setRoleAdmin(APP_GOVERNOR, APP_ROLE_ADMIN);
        AccessControl._setRoleAdmin(APP_ROLE_ADMIN, GOVERNANCE_ADMIN);
        AccessControl._setRoleAdmin(GOVERNANCE_ADMIN, GOVERNANCE_ADMIN);
        AccessControl._setRoleAdmin(OPERATOR, APP_ROLE_ADMIN);
        AccessControl._setRoleAdmin(TOKEN_ADMIN, APP_ROLE_ADMIN);
        AccessControl._setRoleAdmin(UPGRADE_GOVERNOR, GOVERNANCE_ADMIN);
        if (subGovernors) {
            AccessControl._grantRole(APP_ROLE_ADMIN, provisionalGovernor);
            AccessControl._grantRole(GOVERNANCE_ADMIN, provisionalGovernor);
            AccessControl._grantRole(UPGRADE_GOVERNOR, provisionalGovernor);
        }
    }
}
