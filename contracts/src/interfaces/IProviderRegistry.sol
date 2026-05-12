// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, ProviderInfo} from "../SecurityTypes.sol";

/// @title IProviderRegistry
/// @notice Minimal surface required by `SettlementEscrow`.
interface IProviderRegistry {
    function getProvider(address provider) external view returns (ProviderInfo memory);
    function supportsTier(address provider, SecurityTier tier) external view returns (bool);
}
