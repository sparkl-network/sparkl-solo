// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";

/// @notice Deploy mocks + core contracts for local Anvil / Hardhat node.
/// @dev Usage (Anvil): `anvil &` then
///      `forge script script/DeployLocal.s.sol:DeployLocal --rpc-url http://127.0.0.1:8545 --broadcast`
contract DeployLocal is Script {
    function run() external {
        uint256 pk = vm.envOr("PRIVATE_KEY", uint256(0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80));
        address deployer = vm.addr(pk);

        vm.startBroadcast(pk);

        MockOracle oracle = new MockOracle();
        oracle.set(1_000_000);

        MockERC20 usdc = new MockERC20("USDC", 6);

        ProviderRegistry registry = new ProviderRegistry(deployer, deployer);
        SettlementEscrow escrow = new SettlementEscrow(registry, oracle, usdc);

        vm.stopBroadcast();

        console2.log("deployer", deployer);
        console2.log("MockOracle", address(oracle));
        console2.log("MockERC20 USDC", address(usdc));
        console2.log("ProviderRegistry", address(registry));
        console2.log("SettlementEscrow", address(escrow));
    }
}
