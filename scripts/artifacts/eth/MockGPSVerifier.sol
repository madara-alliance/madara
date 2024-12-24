// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract MockGPSVerifier {
    // Returns true for any input fact hash
    function isValid(bytes32) public pure returns (bool) {
        return true;
    }
}