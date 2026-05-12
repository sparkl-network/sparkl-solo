// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPriceOracle} from "../interfaces/IPriceOracle.sol";

/// @title MockOracle
/// @notice Test / Anvil oracle fixed at `usdcPerDot` (USDC 6-dec smallest units per 1e18 internal DOT).
/// @dev For `(usdcPerDot, timestamp)` use `getUsdcPerDotWithTimestamp()` or read `timestamp` after `set`.
contract MockOracle is IPriceOracle {
    uint256 public usdcPerDot;
    uint256 public timestamp;

    function set(uint256 _usdcPerDot) external {
        usdcPerDot = _usdcPerDot;
        timestamp = block.timestamp;
    }

    /// @notice Use in tests with `vm.warp` to simulate stale oracle without changing the price fix.
    function setTimestamp(uint256 _timestamp) external {
        timestamp = _timestamp;
    }

    /// @inheritdoc IPriceOracle
    function getUsdcPerDot() external view returns (uint256) {
        return usdcPerDot;
    }

    /// @inheritdoc IPriceOracle
    function getDotForUsdc(uint256 usdcAmount) external view returns (uint256 dotAmount) {
        if (usdcPerDot == 0) return 0;
        return (usdcAmount * 1e18) / usdcPerDot;
    }

    /// @inheritdoc IPriceOracle
    function priceUpdatedAt() external view returns (uint256) {
        return timestamp;
    }

    /// @notice Returns USDC-per-DOT and last `set` time.
    function getUsdcPerDotWithTimestamp() external view returns (uint256 _usdcPerDot, uint256 _timestamp) {
        return (usdcPerDot, timestamp);
    }
}
