// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPriceOracle} from "./interfaces/IPriceOracle.sol";

/// @title RateSetter
/// @notice Oracle implementation where a trusted off-chain service pushes rates.
///         Replaces DIAPriceOracle until a decentralised oracle (DIA Spectra / Pyth)
///         is available on this chain.
///
/// @dev Rate model:
///   - `usdcPerDot`  — USDC smallest units (6 dec) per 1 whole DOT (1e18 internal units)
///                     e.g. DOT = $8.50  →  usdcPerDot = 8_500_000
///   - `dotPerUsdc`  — internal DOT units (1e18 = 1 DOT) per 1 USDC smallest unit
///                     e.g. DOT = $8.50  →  dotPerUsdc = 1e18 / 8.5 ≈ 117_647_058_823_529
///   Both values are stored so consumers avoid division at read time.
///
/// Events:
///   `RateUpdated(uint256 usdcPerDot, uint256 dotPerUsdc, uint256 timestamp)`
///   — queryable via eth_getLogs for full rate history.
contract RateSetter is IPriceOracle {
    // ─── Errors ───────────────────────────────────────────────────────────────
    error Unauthorised();
    error InvalidRate();
    error RateTooStale();

    // ─── Events ───────────────────────────────────────────────────────────────
    /// @notice Emitted on every successful rate update.
    event RateUpdated(uint256 indexed usdcPerDot, uint256 indexed dotPerUsdc, uint256 timestamp);

    /// @notice Emitted when the updater address changes.
    event UpdaterChanged(address indexed previous, address indexed next);

    /// @notice Emitted when the owner changes.
    event OwnerChanged(address indexed previous, address indexed next);

    // ─── State ────────────────────────────────────────────────────────────────
    address public owner;
    address public updater; // the oracle service wallet — only this may call setRate

    uint256 private _usdcPerDot; // USDC 6-dec smallest units per 1 whole DOT
    uint256 private _dotPerUsdc; // 1e18 DOT units per 1 USDC smallest unit
    uint256 private _updatedAt; // unix timestamp of last successful push

    uint256 public maxStaleness; // seconds before getUsdcPerDot() reverts (0 = no check)

    // ─── Constructor ──────────────────────────────────────────────────────────
    constructor(address _updater, uint256 _maxStaleness) {
        owner = msg.sender;
        updater = _updater;
        maxStaleness = _maxStaleness;
    }

    // ─── Rate setter (called by oracle service) ───────────────────────────────
    /// @notice Push a new DOT/USDC rate.
    /// @param usdcPerDot_  USDC 6-dec units per 1 whole DOT. e.g. $8.50 → 8_500_000.
    /// @param dotPerUsdc_  1e18 DOT units per 1 USDC smallest unit.
    ///                     Must satisfy: usdcPerDot_ * dotPerUsdc_ ≈ 1e24
    ///                     (checked within 0.5% tolerance to catch miscalculation).
    function setRate(uint256 usdcPerDot_, uint256 dotPerUsdc_) external {
        if (msg.sender != updater) revert Unauthorised();
        if (usdcPerDot_ == 0 || dotPerUsdc_ == 0) revert InvalidRate();

        // Sanity: usdcPerDot * dotPerUsdc should equal 1e24 (= 1e6 * 1e18).
        // Allow ±0.5% tolerance for rounding.
        uint256 product = usdcPerDot_ * dotPerUsdc_;
        uint256 expected = 1e24;
        uint256 delta = product > expected ? product - expected : expected - product;
        if (delta * 1000 > expected * 5) revert InvalidRate(); // > 0.5% deviation

        _usdcPerDot = usdcPerDot_;
        _dotPerUsdc = dotPerUsdc_;
        _updatedAt = block.timestamp;

        emit RateUpdated(usdcPerDot_, dotPerUsdc_, block.timestamp);
    }

    // ─── IPriceOracle ─────────────────────────────────────────────────────────
    /// @inheritdoc IPriceOracle
    function getUsdcPerDot() external view override returns (uint256) {
        _assertFresh();
        return _usdcPerDot;
    }

    /// @inheritdoc IPriceOracle
    function getDotForUsdc(uint256 usdcAmount) external view override returns (uint256) {
        _assertFresh();
        if (_dotPerUsdc == 0) revert InvalidRate();
        return (usdcAmount * _dotPerUsdc) / 1e6;
    }

    /// @inheritdoc IPriceOracle
    function priceUpdatedAt() external view override returns (uint256) {
        return _updatedAt;
    }

    // ─── Admin ────────────────────────────────────────────────────────────────
    function setUpdater(address next) external {
        if (msg.sender != owner) revert Unauthorised();
        emit UpdaterChanged(updater, next);
        updater = next;
    }

    function setMaxStaleness(uint256 seconds_) external {
        if (msg.sender != owner) revert Unauthorised();
        maxStaleness = seconds_;
    }

    function transferOwnership(address next) external {
        if (msg.sender != owner) revert Unauthorised();
        emit OwnerChanged(owner, next);
        owner = next;
    }

    // ─── Internal ─────────────────────────────────────────────────────────────
    function _assertFresh() private view {
        if (maxStaleness == 0) return;
        if (_updatedAt == 0 || block.timestamp - _updatedAt > maxStaleness) revert RateTooStale();
    }
}
