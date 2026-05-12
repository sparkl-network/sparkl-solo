// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

enum SecurityTier {
    BEST_EFFORT,
    TEE_VERIFIED
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
}
