// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

enum SecurityTier {
    BEST_EFFORT,
    TEE_VERIFIED
}

struct ProviderInfo {
    address payout;
    uint16 feeBps;
    bool active;
    bool supportsBestEffort;
    bool supportsTEE;
    bytes32 teeReportHash;
}
