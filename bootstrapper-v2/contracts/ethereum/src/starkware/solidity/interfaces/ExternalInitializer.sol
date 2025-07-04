// SPDX-License-Identifier: Apache-2.0.
pragma solidity ^0.8.0;

interface ExternalInitializer {
    event LogExternalInitialize(bytes data);

    function initialize(bytes calldata data) external;
}
