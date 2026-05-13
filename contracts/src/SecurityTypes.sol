// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

enum SecurityTier {
    BEST_EFFORT,
    TEE_VERIFIED
}

/// @notice Lifecycle for operator-driven rundown; record is retained until optional owner purge.
enum NodeLifecycle {
    Active,
    Chilled,
    Defunct
}

/// @notice On-chain record for a provider node. `nodeId` is the registry key (node identity), not necessarily the operator.
struct NodeInfo {
    address payout;
    uint16 feeBps;
    bool active;
    bool supportsBestEffort;
    bool supportsTEE;
    bytes32 teeReportHash;
    string metadataURI;
    NodeLifecycle lifecycle;
}
