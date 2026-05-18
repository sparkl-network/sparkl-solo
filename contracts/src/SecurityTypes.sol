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
    /// @dev Raw X25519 public key (32 bytes) for ConsumerKey-style encryption; zero = not advertised.
    bytes32 encryptionPubkey;
    /// @dev Currently active encryption key version, or 0 if none.
    uint32 encryptionKeyVersion;
    /// @dev Highest version index ever issued; bounds `purgeDefunctNode` cleanup of `encryptionKeys`.
    uint32 encryptionKeysLastVersion;
}

/// @notice Versioned encryption key material for a node (`encryptionKeys[nodeId][version]`).
struct EncryptionKey {
    bytes32 pubkey;
    uint64 activatedAt;
    uint64 deprecatedAt;
    bool revoked;
}
