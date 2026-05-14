// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {DeploySparklBase} from "./DeploySparklBase.sol";
import {console2} from "forge-std/console2.sol";

/// @notice Paseo (Hub testnet EVM): deploy mocks + Sparkl core contracts; record addresses to `deployments/paseo.json`.
/// @dev Requires `PRIVATE_KEY` in env. Optionally `ATTESTATION_SERVICE` — defaults to the deployer. Optional `DEPLOYMENTS_OUT` overrides output path (relative to repo `contracts/`).
///      Invoke with `--rpc-url` pointing at the Paseo Hub EVM JSON-RPC endpoint, e.g. `PASEO_RPC`.
///
/// forge script script/DeployPaseo.s.sol:DeployPaseo --rpc-url $PASEO_RPC --broadcast [--verify …]
contract DeployPaseo is DeploySparklBase {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        address attest = vm.envOr("ATTESTATION_SERVICE", deployer);

        Deployment memory dep = deploySparklCore(deployer, attest, pk, 10);

        string memory jsonPath = "deployments/paseo.json";
        if (vm.envExists("DEPLOYMENTS_OUT")) {
            jsonPath = vm.envString("DEPLOYMENTS_OUT");
        }

        _writeDeployJson(dep, deployer, block.chainid, jsonPath);

        console2.log("network", "paseo");
        console2.log("chainId", block.chainid);
        console2.log("deployer", deployer);
        console2.log("attestationService", attest);
        console2.log("MockOracle", dep.mockOracle);
        console2.log("MockERC20 USDC", dep.mockUsdc);
        console2.log("ProviderRegistry", dep.providerRegistry);
        console2.log("SettlementEscrow", dep.settlementEscrow);
        console2.log("SparklNetworkConfig", dep.sparklNetworkConfig);
        console2.log("wrote deployments file", jsonPath);
    }

    function _writeDeployJson(Deployment memory dep, address deployer, uint256 chainId, string memory path) internal {
        string memory root = "paseo";
        vm.serializeString(root, "network", "paseo-hub-evm");
        vm.serializeUint(root, "chainId", chainId);
        vm.serializeUint(root, "deployedAtTimestamp", block.timestamp);
        vm.serializeAddress(root, "deployer", deployer);
        vm.serializeAddress(root, "registryOwner", dep.registryOwner);
        vm.serializeAddress(root, "attestationService", dep.attestationService);
        vm.serializeAddress(root, "mockOracle", dep.mockOracle);
        vm.serializeAddress(root, "mockUsdc", dep.mockUsdc);
        vm.serializeAddress(root, "providerRegistry", dep.providerRegistry);
        vm.serializeAddress(root, "settlementEscrow", dep.settlementEscrow);
        vm.serializeAddress(root, "sparklNetworkConfig", dep.sparklNetworkConfig);
        string memory json = vm.serializeBytes32(root, "networkConfigSalt", NETWORK_CONFIG_SALT);
        vm.writeJson(json, path);
    }
}
