// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";

import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";

/// @notice Deploy MockOracle + mock USDC + `ProviderRegistry` + `SettlementEscrow` used by deploy scripts.
/// @dev MVP testnet deployments use mocks; Hub USDC/oracle integrations replace these for production-shaped deploys later.
abstract contract DeploySparklBase is Script {
    uint256 internal constant DEFAULT_USDC_PER_DOT = 1_340_000; // baseline ~1.34 USD/DOT (USDC smallest per 1e18 internal DOT)

    struct Deployment {
        address registryOwner;
        address attestationService;
        address mockOracle;
        address mockUsdc;
        address providerRegistry;
        address settlementEscrow;
    }

    /// @param registryOwner Passed to `ProviderRegistry.owner` — typically the broadcaster.
    /// @param attestationService `ProviderRegistry` constructor second arg (has `setTEEProof` authority).
    /// @param escrowNativeDecimals Hub Asset Hub DOT: 10 (Planck). Anvil / standard EVM: 18 (`msg.value` wei).
    function deploySparklCore(
        address registryOwner,
        address attestationService,
        uint256 deployerPk,
        uint8 escrowNativeDecimals
    ) internal returns (Deployment memory d) {
        d.registryOwner = registryOwner;
        d.attestationService = attestationService;

        vm.startBroadcast(deployerPk);

        MockOracle oracle = new MockOracle();
        oracle.set(DEFAULT_USDC_PER_DOT);

        MockERC20 usdc = new MockERC20("USDC", 6);

        ProviderRegistry registry = new ProviderRegistry(registryOwner, attestationService);
        SettlementEscrow escrow = new SettlementEscrow(registry, oracle, usdc, escrowNativeDecimals);

        vm.stopBroadcast();

        d.mockOracle = address(oracle);
        d.mockUsdc = address(usdc);
        d.providerRegistry = address(registry);
        d.settlementEscrow = address(escrow);
    }
}
