// SPDX-License-Identifier: MIT
// Based on OpenZeppelin Contract (access/AccessControl.sol)
// StarkWare modification (storage slot, change to library).

pragma solidity ^0.8.0;

import "src/third_party/open_zeppelin/utils/Strings.sol";

/*
  Library module that allows using contracts to implement role-based access
  control mechanisms. This is a lightweight version that doesn't allow enumerating role
  members except through off-chain means by accessing the contract event logs. Some
  applications may benefit from on-chain enumerability, for those cases see
  {AccessControlEnumerable}.
 
  Roles are referred to by their `bytes32` identifier. These should be exposed
  in the external API and be unique. The best way to achieve this is by
  using `public constant` hash digests:
 
  ```
  bytes32 public constant MY_ROLE = keccak256("MY_ROLE");
  ```
 
  Roles can be used to represent a set of permissions. To restrict access to a
  function call, use {hasRole}:
 
  ```
  function foo() public {
      require(hasRole(MY_ROLE, msg.sender));
      ...
  }
  ```
 
  Roles can be granted and revoked dynamically via the {grantRole} and
  {revokeRole} functions. Each role has an associated admin role, and only
  accounts that have a role's admin role can call {grantRole} and {revokeRole}.
 
  By default, the admin role for all roles is `DEFAULT_ADMIN_ROLE`, which means
  that only accounts with this role will be able to grant or revoke other
  roles. More complex role relationships can be created by using
  {_setRoleAdmin}.
 
  WARNING: The `DEFAULT_ADMIN_ROLE` is also its own admin: it has permission to
  grant and revoke this role. Extra precautions should be taken to secure
  accounts that have been granted it.
 
  OpenZeppelin implementation changed as following:
  1. Converted to library.
  2. Storage valiable {_roles} moved outside of linear storage,
     to avoid potential storage conflicts or corruption.
  3. Removed ERC165 support.
*/
library AccessControl {
    /*
      Emitted when `newAdminRole` is set as ``role``'s admin role, replacing `previousAdminRole`
     
      `DEFAULT_ADMIN_ROLE` is the starting admin for all roles, despite
      {RoleAdminChanged} not being emitted signaling this.
     
      Available since v3.1.
    */
    event RoleAdminChanged(
        bytes32 indexed role,
        bytes32 indexed previousAdminRole,
        bytes32 indexed newAdminRole
    );

    /*
      Emitted when `account` is granted `role`.
     
      `sender` is the account that originated the contract call, an admin role
      bearer except when using {AccessControl-_setupRole}.
    */
    event RoleGranted(bytes32 indexed role, address indexed account, address indexed sender);

    /*
      Emitted when `account` is revoked `role`.
     
      `sender` is the account that originated the contract call:
        - if using `revokeRole`, it is the admin role bearer
        - if using `renounceRole`, it is the role bearer (i.e. `account`).
    */
    event RoleRevoked(bytes32 indexed role, address indexed account, address indexed sender);

    struct RoleData {
        mapping(address => bool) members;
        bytes32 adminRole;
    }

    // Context interface functions.
    function _msgSender() internal view returns (address) {
        return msg.sender;
    }

    function _msgData() internal pure returns (bytes calldata) {
        return msg.data;
    }

    // The storage variable `_roles` is located away from the contract linear area (low storage addresses)
    // to prevent potential collision/corruption in upgrade scenario.
    // Slot = Web3.keccak(text="AccesControl_Storage_Slot").
    bytes32 constant rolesSlot = 0x53e43b954ba190a7e49386f1f78b01dcd9f628db23f432fa029a7dfd6d98e8fb;

    function _roles() private pure returns (mapping(bytes32 => RoleData) storage roles) {
        assembly {
            roles.slot := rolesSlot
        }
    }

    bytes32 constant DEFAULT_ADMIN_ROLE = 0x00;

    /*
      Modifier that checks that an account has a specific role. Reverts
      with a standardized message including the required role.
      
      The format of the revert reason is given by the following regular expression:
      
      /^AccessControl: account (0x[0-9a-f]{40}) is missing role (0x[0-9a-f]{64})$/
      
      Available since v4.1.
    */
    modifier onlyRole(bytes32 role) {
        _checkRole(role);
        _;
    }

    /*
      Returns `true` if `account` has been granted `role`.
    */
    function hasRole(bytes32 role, address account) internal view returns (bool) {
        return _roles()[role].members[account];
    }

    /*
      Revert with a standard message if `_msgSender()` is missing `role`.
      Overriding this function changes the behavior of the {onlyRole} modifier.
     
      Format of the revert message is described in {_checkRole}.
     
      Available since v4.6.
    */
    function _checkRole(bytes32 role) internal view {
        _checkRole(role, _msgSender());
    }

    /*
      Revert with a standard message if `account` is missing `role`.
     
      The format of the revert reason is given by the following regular expression:
     
       /^AccessControl: account (0x[0-9a-f]{40}) is missing role (0x[0-9a-f]{64})$/.
    */
    function _checkRole(bytes32 role, address account) internal view {
        if (!hasRole(role, account)) {
            revert(
                string(
                    abi.encodePacked(
                        "AccessControl: account ",
                        Strings.toHexString(uint160(account), 20),
                        " is missing role ",
                        Strings.toHexString(uint256(role), 32)
                    )
                )
            );
        }
    }

    /*
      Returns the admin role that controls `role`. See {grantRole} and
      {revokeRole}.
     
      To change a role's admin, use {_setRoleAdmin}.
    */
    function getRoleAdmin(bytes32 role) internal view returns (bytes32) {
        return _roles()[role].adminRole;
    }

    /*
      Grants `role` to `account`.
     
      If `account` had not been already granted `role`, emits a {RoleGranted}
      event.
     
      Requirements:
     
      - the caller must have ``role``'s admin role.
     
      May emit a {RoleGranted} event.
    */
    function grantRole(bytes32 role, address account) internal onlyRole(getRoleAdmin(role)) {
        _grantRole(role, account);
    }

    /*
      Revokes `role` from `account`.
     
      If `account` had been granted `role`, emits a {RoleRevoked} event.
     
      Requirements:
     
      - the caller must have ``role``'s admin role.
     
      * May emit a {RoleRevoked} event.
    */
    function revokeRole(bytes32 role, address account) internal onlyRole(getRoleAdmin(role)) {
        _revokeRole(role, account);
    }

    /*
      Revokes `role` from the calling account.
     
      Roles are often managed via {grantRole} and {revokeRole}: this function's
      purpose is to provide a mechanism for accounts to lose their privileges
      if they are compromised (such as when a trusted device is misplaced).
     
      If the calling account had been revoked `role`, emits a {RoleRevoked}
      event.
     
      Requirements:
     
      - the caller must be `account`.
     
      May emit a {RoleRevoked} event.
    */
    function renounceRole(bytes32 role, address account) internal {
        require(account == _msgSender(), "AccessControl: can only renounce roles for self");

        _revokeRole(role, account);
    }

    /*
      Grants `role` to `account`.
     
      If `account` had not been already granted `role`, emits a {RoleGranted}
      event. Note that unlike {grantRole}, this function doesn't perform any
      checks on the calling account.
     
      May emit a {RoleGranted} event.
     
      [WARNING]virtual
      ====
      This function should only be called from the constructor when setting
      up the initial roles for the system.
     
      Using this function in any other way is effectively circumventing the admin
      system imposed by {AccessControl}.
      ====
     
      NOTE: This function is deprecated in favor of {_grantRole}.
    */
    function _setupRole(bytes32 role, address account) internal {
        _grantRole(role, account);
    }

    /*
      Sets `adminRole` as ``role``'s admin role.
     
      Emits a {RoleAdminChanged} event.
    */
    function _setRoleAdmin(bytes32 role, bytes32 adminRole) internal {
        bytes32 previousAdminRole = getRoleAdmin(role);
        _roles()[role].adminRole = adminRole;
        emit RoleAdminChanged(role, previousAdminRole, adminRole);
    }

    /*
      Grants `role` to `account`.
     
      Internal function without access restriction.
     
      May emit a {RoleGranted} event.
    */
    function _grantRole(bytes32 role, address account) internal {
        if (!hasRole(role, account)) {
            _roles()[role].members[account] = true;
            emit RoleGranted(role, account, _msgSender());
        }
    }

    /*
      Revokes `role` from `account`.
     
      Internal function without access restriction.
     
      May emit a {RoleRevoked} event.
    */
    function _revokeRole(bytes32 role, address account) internal {
        if (hasRole(role, account)) {
            _roles()[role].members[account] = false;
            emit RoleRevoked(role, account, _msgSender());
        }
    }
}
