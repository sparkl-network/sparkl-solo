// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {DeploySparklBase} from "./DeploySparklBase.sol";
import {console2} from "forge-std/console2.sol";

/// @notice Deploy mocks + core contracts for local Anvil / Hardhat node.
/// @dev Usage (Anvil): `anvil &` then
///      `forge script script/DeployLocal.s.sol:DeployLocal --rpc-url http://127.0.0.1:8545 --broadcast`
contract DeployLocal is DeploySparklBase {
    /// @notice Anvil dev key #1 (public, well-known — local only).
    uint256 internal constant ANVIL_DEFAULT_PK =
        uint256(0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

    function run() external {
        uint256 pk = vm.envOr("PRIVATE_KEY", ANVIL_DEFAULT_PK);
        address deployer = vm.addr(pk);

        Deployment memory dep = deploySparklCore(deployer, deployer, pk, 18);

        console2.log("deployer", deployer);
        console2.log("MockOracle", dep.mockOracle);
        console2.log("MockERC20 USDC", dep.mockUsdc);
        console2.log("ProviderRegistry", dep.providerRegistry);
        console2.log("SettlementEscrow", dep.settlementEscrow);
    }
}
