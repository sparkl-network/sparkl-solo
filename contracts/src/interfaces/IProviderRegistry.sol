// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, NodeInfo} from "../SecurityTypes.sol";

/// @title IProviderRegistry
/// @notice Minimal surface required by `SettlementEscrow`.
interface IProviderRegistry {
    function owner() external view returns (address);

    /// @param nodeId Registry key for the node (see `ProviderRegistry.registerNode`).
    function getProvider(address nodeId) external view returns (NodeInfo memory);

    function supportsTier(address nodeId, SecurityTier tier) external view returns (bool);
}
