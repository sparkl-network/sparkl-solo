// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IDIAOracle
/// @notice Minimal DIA value feed surface used by `DIAPriceOracle` (MVP).
/// @dev Many DIA deployments expose `getValue(string key) returns (uint128 value, uint128 timestamp)`.
///      Feed decimals are deployment-specific; `DIAPriceOracle` normalizes to a single internal scale.
interface IDIAOracle {
    function getValue(string calldata key) external view returns (uint128 value, uint128 timestamp);
}
