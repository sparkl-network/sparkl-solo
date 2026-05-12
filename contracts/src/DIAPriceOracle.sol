// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPriceOracle} from "./interfaces/IPriceOracle.sol";
import {IDIAOracle} from "./interfaces/IDIAOracle.sol";

/// @title DIAPriceOracle
/// @notice MVP oracle: combines DIA DOT/USD and USDC/USD feeds (USD per whole coin, shared scale) into USDC-per-DOT and DOT-for-USDC.
/// @dev Internal DOT uses 1e18 per whole DOT, matching `SettlementEscrow`.
contract DIAPriceOracle is IPriceOracle {
    IDIAOracle public immutable dotUsdFeed;
    IDIAOracle public immutable usdcUsdFeed;
    string public dotKey;
    string public usdcKey;
    uint8 public immutable feedDecimals;

    error BadPrice();
    error ZeroUsdcPrice();

    constructor(IDIAOracle dotUsd, IDIAOracle usdcUsd_, string memory dotKey_, string memory usdcKey_, uint8 feedDecimals_) {
        dotUsdFeed = dotUsd;
        usdcUsdFeed = usdcUsd_;
        dotKey = dotKey_;
        usdcKey = usdcKey_;
        feedDecimals = feedDecimals_;
    }

    function _usdPerWhole(IDIAOracle feed, string memory key) internal view returns (uint256) {
        (uint128 v,) = feed.getValue(key);
        if (v == 0) revert BadPrice();
        return uint256(v);
    }

    /// @inheritdoc IPriceOracle
    function getUsdcPerDot() public view returns (uint256 usdcPerWholeDot) {
        uint256 dotUsd = _usdPerWhole(dotUsdFeed, dotKey);
        uint256 usdcUsd = _usdPerWhole(usdcUsdFeed, usdcKey);
        if (usdcUsd == 0) revert ZeroUsdcPrice();
        // Same scale on both feeds: USD per 1 whole coin → USDC (6-dec) smallest per 1 whole DOT.
        usdcPerWholeDot = (1_000_000 * dotUsd) / usdcUsd;
        if (usdcPerWholeDot == 0) revert BadPrice();
    }

    /// @inheritdoc IPriceOracle
    function getDotForUsdc(uint256 usdcAmount) external view returns (uint256 dotAmount) {
        uint256 perDot = getUsdcPerDot();
        dotAmount = (usdcAmount * 1e18) / perDot;
    }

    /// @inheritdoc IPriceOracle
    /// @dev Conservative: freshness is the older of the two feed timestamps.
    function priceUpdatedAt() external view returns (uint256 ts) {
        (, uint128 dotTs) = dotUsdFeed.getValue(dotKey);
        (, uint128 usdcTs) = usdcUsdFeed.getValue(usdcKey);
        ts = uint256(dotTs <= usdcTs ? dotTs : usdcTs);
    }
}
