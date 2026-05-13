// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice View surface used by `ProviderRegistry` to gate `markDefunct`.
interface ISettlementEscrowOpenSessions {
    function openSessionCountByNode(bytes32 nodeId) external view returns (uint256);
}
