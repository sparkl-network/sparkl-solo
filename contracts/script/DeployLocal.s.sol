// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {DeploySparklBase} from "./DeploySparklBase.sol";
import {RateSetter} from "../src/RateSetter.sol";
import {console2} from "forge-std/console2.sol";

/// @notice Local Anvil: deploy RateSetter + Sparkl core contracts; record addresses to `deployments/local.json`.
/// @dev Uses Anvil account #0 private key by default (`PRIVATE_KEY` override). Optional env:
///      `ORACLE_UPDATER_ADDRESS` — `RateSetter.setRate` caller (defaults to deployer).
///      `ORACLE_MAX_STALENESS` — seconds (default 3600). When deployer is updater, initial rate is pushed.
///      `ORACLE_USDC_PER_DOT` — baseline USDC smallest per 1e18 DOT (default 1_340_000).
///      `DEPLOYMENTS_OUT` — JSON path relative to `contracts/` (default `deployments/local.json`).
///
/// `anvil &` then:
/// `forge script script/DeployLocal.s.sol:DeployLocal --rpc-url http://127.0.0.1:8545 --broadcast`
contract DeployLocal is DeploySparklBase {
    /// @notice Anvil dev key #0 (public, well-known — local only).
    uint256 internal constant ANVIL_DEFAULT_PK =
        uint256(0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);

    function run() external {
        uint256 pk = vm.envOr("PRIVATE_KEY", ANVIL_DEFAULT_PK);
        address deployer = vm.addr(pk);

        address attest = vm.envOr("ATTESTATION_SERVICE", deployer);
        address oracleUpdater = vm.envOr("ORACLE_UPDATER_ADDRESS", deployer);
        uint256 maxStaleness = vm.envOr("ORACLE_MAX_STALENESS", uint256(3600));
        uint256 usdcPerDot = vm.envOr("ORACLE_USDC_PER_DOT", DEFAULT_USDC_PER_DOT);
        uint256 dotPerUsdc = dotPerUsdcFromUsdcPerDot(usdcPerDot);

        vm.startBroadcast(pk);

        RateSetter rateOracle = new RateSetter(oracleUpdater, maxStaleness);
        if (oracleUpdater == deployer) {
            rateOracle.setRate(usdcPerDot, dotPerUsdc);
            console2.log("RateSetter initial rate pushed by deployer");
        } else {
            console2.log("RateSetter deployed; updater must call setRate before escrow USDC deposits");
        }

        vm.stopBroadcast();

        Deployment memory dep = deploySparklCoreWithOracle(deployer, attest, pk, 18, rateOracle);

        string memory jsonPath = "deployments/local.json";
        if (vm.envExists("DEPLOYMENTS_OUT")) {
            jsonPath = vm.envString("DEPLOYMENTS_OUT");
        }

        _writeDeployJson(dep, deployer, oracleUpdater, block.chainid, jsonPath);

        console2.log("network", "anvil-local");
        console2.log("chainId", block.chainid);
        console2.log("deployer", deployer);
        console2.log("attestationService", attest);
        console2.log("oracleUpdater", oracleUpdater);
        console2.log("RateSetter", dep.mockOracle);
        console2.log("MockERC20 USDC", dep.mockUsdc);
        console2.log("ProviderRegistry", dep.providerRegistry);
        console2.log("SettlementEscrow", dep.settlementEscrow);
        console2.log("SparklNetworkConfig", dep.sparklNetworkConfig);
        console2.log("wrote deployments file", jsonPath);
    }

    function _writeDeployJson(
        Deployment memory dep,
        address deployer,
        address oracleUpdater,
        uint256 chainId,
        string memory path
    ) internal {
        string memory root = "local";
        vm.serializeString(root, "network", "anvil-local");
        vm.serializeUint(root, "chainId", chainId);
        vm.serializeUint(root, "deployedAtTimestamp", block.timestamp);
        vm.serializeAddress(root, "deployer", deployer);
        vm.serializeAddress(root, "registryOwner", dep.registryOwner);
        vm.serializeAddress(root, "attestationService", dep.attestationService);
        vm.serializeAddress(root, "oracleUpdater", oracleUpdater);
        vm.serializeAddress(root, "rateSetter", dep.mockOracle);
        vm.serializeAddress(root, "priceOracle", dep.mockOracle);
        vm.serializeAddress(root, "mockUsdc", dep.mockUsdc);
        vm.serializeAddress(root, "providerRegistry", dep.providerRegistry);
        vm.serializeAddress(root, "settlementEscrow", dep.settlementEscrow);
        vm.serializeAddress(root, "sparklNetworkConfig", dep.sparklNetworkConfig);
        string memory json = vm.serializeBytes32(root, "networkConfigSalt", NETWORK_CONFIG_SALT);
        vm.writeJson(json, path);
    }
}
