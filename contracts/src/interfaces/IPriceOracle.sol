// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IPriceOracle
/// @notice Abstraction for DOT-denominated pricing used by `SettlementEscrow`.
/// @dev Primary rate is **USDC per DOT** (`getUsdcPerDot`). Internal DOT uses 1e18 per whole DOT
///      (same scale as `depositDot` / escrow accounting).
interface IPriceOracle {
    /// @notice USDC smallest units (typically 6 decimals) per 1e18 internal DOT (one whole DOT).
    function getUsdcPerDot() external view returns (uint256 usdcPerWholeDot);

    /// @notice Internal DOT amount (`1e18` = 1 DOT) equivalent to `usdcAmount` USDC smallest units.
    /// @dev Algebraically `usdcAmount * 1e18 / getUsdcPerDot()`; implementers should keep this consistent with `getUsdcPerDot`.
    function getDotForUsdc(uint256 usdcAmount) external view returns (uint256 dotAmount);

    /// @notice Wall-clock second of the freshest price sample backing `getUsdcPerDot()`, or `0` if unsupported.
    /// @dev Escrow guarded deposits treat `0` as stale whenever a max-age bound is enforced.
    function priceUpdatedAt() external view returns (uint256);
}
