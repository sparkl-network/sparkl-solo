// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {IPriceOracle} from "../src/interfaces/IPriceOracle.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {SparklNetworkConfig} from "../src/SparklNetworkConfig.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";

/// @notice Deploy MockOracle + mock USDC + `ProviderRegistry` + `SettlementEscrow` used by deploy scripts.
/// @dev MVP testnet deployments use mocks; Hub USDC/oracle integrations replace these for production-shaped deploys later.
///      `deploySparklCore` calls `registry.setSettlementEscrow(escrow)` in the broadcaster context — **`registryOwner` must equal
///      `vm.addr(deployerPk)`** so that call succeeds without a separate owner transaction.
///
///      CREATE2 bootstrap: `SparklNetworkConfig` uses `NETWORK_CONFIG_SALT = keccak256("sparkl.network.config.v1")`.
///      Foundry `new Contract{salt:}` uses the canonical CREATE2 deployer (`vm.computeCreate2Address` two-arg form).
///      Init code is `type(SparklNetworkConfig).creationCode` **concat** `abi.encode(registryOwner)` (constructor `address`).
abstract contract DeploySparklBase is Script {
    uint256 internal constant DEFAULT_USDC_PER_DOT = 1_340_000; // baseline ~1.34 USD/DOT (USDC smallest per 1e18 internal DOT)

    /// @dev Must match `contracts/src/SparklNetworkConfig.sol` comments and Rust operator docs.
    bytes32 internal constant NETWORK_CONFIG_SALT = keccak256("sparkl.network.config.v1");

    struct Deployment {
        address registryOwner;
        address attestationService;
        address mockOracle;
        address mockUsdc;
        address providerRegistry;
        address settlementEscrow;
        address sparklNetworkConfig;
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

        address broadcaster = vm.addr(deployerPk);
        require(broadcaster == registryOwner, "DeploySparklBase: registryOwner must equal broadcaster EOA");

        vm.startBroadcast(deployerPk);

        MockOracle oracle = new MockOracle();
        oracle.set(DEFAULT_USDC_PER_DOT);

        MockERC20 usdc = new MockERC20("USDC", 6);

        ProviderRegistry registry = new ProviderRegistry(registryOwner, attestationService);
        SettlementEscrow escrow = new SettlementEscrow(registry, oracle, usdc, escrowNativeDecimals);
        registry.setSettlementEscrow(address(escrow));

        bytes32 initCodeHash = keccak256(
            abi.encodePacked(type(SparklNetworkConfig).creationCode, abi.encode(registryOwner))
        );
        address predictedNetCfg = vm.computeCreate2Address(NETWORK_CONFIG_SALT, initCodeHash);
        console2.log("SparklNetworkConfig CREATE2 predicted:", predictedNetCfg);
        console2.logBytes32(NETWORK_CONFIG_SALT);

        SparklNetworkConfig netCfg = new SparklNetworkConfig{salt: NETWORK_CONFIG_SALT}(registryOwner);
        require(address(netCfg) == predictedNetCfg, "SparklNetworkConfig CREATE2 address mismatch");
        netCfg.setAddresses(address(registry), address(escrow), address(oracle));

        vm.stopBroadcast();

        d.mockOracle = address(oracle);
        d.mockUsdc = address(usdc);
        d.providerRegistry = address(registry);
        d.settlementEscrow = address(escrow);
        d.sparklNetworkConfig = address(netCfg);
    }

    /// @dev Same as `deploySparklCore` but uses a pre-deployed `IPriceOracle` (e.g. `RateSetter` on Paseo).
    function deploySparklCoreWithOracle(
        address registryOwner,
        address attestationService,
        uint256 deployerPk,
        uint8 escrowNativeDecimals,
        IPriceOracle priceOracle
    ) internal returns (Deployment memory d) {
        d.registryOwner = registryOwner;
        d.attestationService = attestationService;

        address broadcaster = vm.addr(deployerPk);
        require(broadcaster == registryOwner, "DeploySparklBase: registryOwner must equal broadcaster EOA");

        vm.startBroadcast(deployerPk);

        MockERC20 usdc = new MockERC20("USDC", 6);

        ProviderRegistry registry = new ProviderRegistry(registryOwner, attestationService);
        SettlementEscrow escrow = new SettlementEscrow(registry, priceOracle, usdc, escrowNativeDecimals);
        registry.setSettlementEscrow(address(escrow));

        bytes32 initCodeHash = keccak256(
            abi.encodePacked(type(SparklNetworkConfig).creationCode, abi.encode(registryOwner))
        );
        address predictedNetCfg = vm.computeCreate2Address(NETWORK_CONFIG_SALT, initCodeHash);
        console2.log("SparklNetworkConfig CREATE2 predicted:", predictedNetCfg);
        console2.logBytes32(NETWORK_CONFIG_SALT);

        SparklNetworkConfig netCfg = new SparklNetworkConfig{salt: NETWORK_CONFIG_SALT}(registryOwner);
        require(address(netCfg) == predictedNetCfg, "SparklNetworkConfig CREATE2 address mismatch");
        netCfg.setAddresses(address(registry), address(escrow), address(priceOracle));

        vm.stopBroadcast();

        d.mockOracle = address(priceOracle);
        d.mockUsdc = address(usdc);
        d.providerRegistry = address(registry);
        d.settlementEscrow = address(escrow);
        d.sparklNetworkConfig = address(netCfg);
    }

    /// @notice `dotPerUsdc` so `usdcPerDot * dotPerUsdc` is within RateSetter's ±0.5% of `1e24`.
    function dotPerUsdcFromUsdcPerDot(uint256 usdcPerDot) internal pure returns (uint256) {
        return 1e24 / usdcPerDot;
    }
}
