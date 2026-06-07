// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {IPriceOracle} from "../src/interfaces/IPriceOracle.sol";
import {IModelPriceOracle} from "../src/interfaces/IModelPriceOracle.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {SparklNetworkConfig} from "../src/SparklNetworkConfig.sol";
import {ModelPriceOracle} from "../src/ModelPriceOracle.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";

/// @notice Deploy MockOracle + mock USDC + `ProviderRegistry` + `SettlementEscrow` used by deploy scripts.
abstract contract DeploySparklBase is Script {
    uint256 internal constant DEFAULT_USDC_PER_DOT = 1_340_000;

    bytes32 internal constant NETWORK_CONFIG_SALT = keccak256("sparkl.network.config.v1");

    struct Deployment {
        address registryOwner;
        address attestationService;
        address mockOracle;
        address modelPriceOracle;
        address mockUsdc;
        address providerRegistry;
        address settlementEscrow;
        address sparklNetworkConfig;
    }

    function deploySparklCore(
        address registryOwner,
        address attestationService,
        uint256 deployerPk,
        uint8 escrowNativeDecimals
    ) internal returns (Deployment memory d) {
        vm.startBroadcast(deployerPk);

        MockOracle oracle = new MockOracle();
        oracle.set(DEFAULT_USDC_PER_DOT);

        ModelPriceOracle modelOracle = new ModelPriceOracle(vm.addr(deployerPk));

        vm.stopBroadcast();

        return deploySparklCoreWithOracle(
            registryOwner, attestationService, deployerPk, escrowNativeDecimals, oracle, modelOracle
        );
    }

    function deploySparklCoreWithOracle(
        address registryOwner,
        address attestationService,
        uint256 deployerPk,
        uint8 escrowNativeDecimals,
        IPriceOracle priceOracle,
        IModelPriceOracle modelPriceOracle
    ) internal returns (Deployment memory d) {
        d.registryOwner = registryOwner;
        d.attestationService = attestationService;

        address broadcaster = vm.addr(deployerPk);
        require(broadcaster == registryOwner, "DeploySparklBase: registryOwner must equal broadcaster EOA");

        vm.startBroadcast(deployerPk);

        MockERC20 usdc = new MockERC20("USDC", 6);

        ProviderRegistry registry = new ProviderRegistry(registryOwner, attestationService);
        SettlementEscrow escrow =
            new SettlementEscrow(registry, priceOracle, modelPriceOracle, usdc, escrowNativeDecimals);
        registry.setSettlementEscrow(address(escrow));

        bytes32 initCodeHash = keccak256(
            abi.encodePacked(type(SparklNetworkConfig).creationCode, abi.encode(registryOwner))
        );
        address predictedNetCfg = vm.computeCreate2Address(NETWORK_CONFIG_SALT, initCodeHash);
        console2.log("SparklNetworkConfig CREATE2 predicted:", predictedNetCfg);
        console2.logBytes32(NETWORK_CONFIG_SALT);

        SparklNetworkConfig netCfg;
        if (predictedNetCfg.code.length > 0) {
            console2.log("SparklNetworkConfig already at CREATE2; updating addresses");
            netCfg = SparklNetworkConfig(predictedNetCfg);
            require(netCfg.owner() == registryOwner, "SparklNetworkConfig: owner mismatch");
        } else {
            netCfg = new SparklNetworkConfig{salt: NETWORK_CONFIG_SALT}(registryOwner);
            require(address(netCfg) == predictedNetCfg, "SparklNetworkConfig CREATE2 address mismatch");
        }
        netCfg.setAddresses(
            address(registry), address(escrow), address(priceOracle), address(modelPriceOracle)
        );

        vm.stopBroadcast();

        d.mockOracle = address(priceOracle);
        d.modelPriceOracle = address(modelPriceOracle);
        d.mockUsdc = address(usdc);
        d.providerRegistry = address(registry);
        d.settlementEscrow = address(escrow);
        d.sparklNetworkConfig = address(netCfg);
    }

    function dotPerUsdcFromUsdcPerDot(uint256 usdcPerDot) internal pure returns (uint256) {
        return 1e24 / usdcPerDot;
    }

    /// @dev USD per 1k tokens in USDC micro-units (e.g. $0.0001/1k => 100). Matches sparkl-oracle-model-price `toOnChainModelPrice`.
    function modelPriceInternalFromUsdPer1kMicro(uint256 usdPer1kMicro, uint256 dotPerUsdc)
        internal
        pure
        returns (uint256)
    {
        return (usdPer1kMicro * dotPerUsdc) / 1_000_000;
    }
}
