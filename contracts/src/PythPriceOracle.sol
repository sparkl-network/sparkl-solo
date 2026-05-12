// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPriceOracle} from "./interfaces/IPriceOracle.sol";

/// @title PythPriceOracle
/// @notice Placeholder for a Pyth-based implementation; deploy `DIAPriceOracle` for MVP.
/// @dev Wire to Pyth’s EVM `IPyth` contract and normalize into the same USDC-per-DOT + internal-DOT scale as `SettlementEscrow`.
contract PythPriceOracle is IPriceOracle {
    error NotImplemented();

    function getUsdcPerDot() external pure returns (uint256) {
        revert NotImplemented();
    }

    function getDotForUsdc(uint256) external pure returns (uint256) {
        revert NotImplemented();
    }

    /// @inheritdoc IPriceOracle
    function priceUpdatedAt() external pure returns (uint256) {
        revert NotImplemented();
    }
}
