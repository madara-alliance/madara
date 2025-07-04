// SPDX-License-Identifier: Apache-2.0.
pragma solidity ^0.8.0;

import "src/starkware/solidity/libraries/NamedStorage.sol";

/*
  Holds the Proxy-specific state variables.
  to prevent collision hazard.
*/
contract ProxyStorage {
    // Random storage slot tags.
    string constant ENABLED_TIME_TAG = "PROXY_5_ENABLED_TIME";
    string constant DISABLED_TIME_TAG = "PROXY_5_DISABLED_TIME";
    string constant INTIALIZED_TAG = "PROXY_5_INITIALIZED";

    // The time after which we can switch to the implementation.
    // Hash(implementation, data, finalize) => time.
    function enabledTime() internal pure returns (mapping(bytes32 => uint256) storage) {
        return NamedStorage.bytes32ToUint256Mapping(ENABLED_TIME_TAG);
    }

    // The time after which we can NO LONGER switch to the implementation.
    // Implementation is valid to switch in time t,  enableTime <= t  <= disableTime.
    // Hash(implementation, data, finalize) => time.
    function expirationTime() internal pure returns (mapping(bytes32 => uint256) storage) {
        return NamedStorage.bytes32ToUint256Mapping(DISABLED_TIME_TAG);
    }

    // A central storage of the flags whether implementation has been initialized.
    // Note - it can be used flexibly enough to accommodate multiple levels of initialization
    // (i.e. using different key salting schemes for different initialization levels).
    function initialized() internal pure returns (mapping(bytes32 => bool) storage) {
        return NamedStorage.bytes32ToBoolMapping(INTIALIZED_TAG);
    }
}
