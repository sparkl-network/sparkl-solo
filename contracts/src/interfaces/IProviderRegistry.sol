// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, NodeInfo} from "../SecurityTypes.sol";

/// @title IProviderRegistry
/// @notice Minimal surface required by `SettlementEscrow`.
interface IProviderRegistry {
    function owner() external view returns (address);

    function nodeOperator(bytes32 nodeId) external view returns (address);

    /// @param nodeId Registry key for the node (e.g. PeerId hash).
    function getProvider(bytes32 nodeId) external view returns (NodeInfo memory);

    function supportsTier(bytes32 nodeId, SecurityTier tier) external view returns (bool);
}
