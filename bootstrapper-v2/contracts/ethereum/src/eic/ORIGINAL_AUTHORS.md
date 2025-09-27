# Original Authors and Source

## Source Repository

This folder contains files derived from the **starkgate-contracts** repository:

- **Repository**: <https://github.com/starknet-io/starkgate-contracts.git>
- **Revision**: 45941888479663ac93e898cd7f8504fa9066c54c
- **License**: Apache-2.0

## Modifications Made

### 1. Import Path Adjustments

Import paths have been modified throughout the files to suit the project structure and dependency management.

### 2. Role Initialization Fix

In `ConfigureSingleBridge.sol`, the `RolesLib.initialize()` call has been commented out to
prevent reinitialization of roles. This modification was necessary because:

- The new Starkware Proxy handles role initialization during deployment in its constructor
- Calling `RolesLib.initialize()` again would cause initialization failures
- This prevents conflicts with the proxy's built-in role management

### 3. Project-Specific Adaptations

Various other adjustments have been made to integrate these contracts with the current
project architecture while maintaining the original functionality and security properties.

## Note

All files in this folder maintain the original Apache-2.0 license and copyright from the
starkgate-contracts repository. Modifications are made solely for integration purposes and
do not affect the core security or functionality of the original contracts.
