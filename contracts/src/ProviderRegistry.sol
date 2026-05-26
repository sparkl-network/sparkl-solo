// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, NodeInfo, NodeLifecycle, EncryptionKey} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";
import {ISettlementEscrowOpenSessions} from "./interfaces/ISettlementEscrowOpenSessions.sol";

/// @title ProviderRegistry
/// @notice Registry of provider nodes: `nodeId` is a stable identity (e.g. Substrate PeerId hash); `msg.sender` registers as the operator.
contract ProviderRegistry is IProviderRegistry {
    address public owner;
    address public attestationService;
    /// @notice Escrow reporting open session counts per node; set by owner after deploy.
    address public settlementEscrow;

    mapping(bytes32 nodeId => NodeInfo) public nodes;
    mapping(bytes32 nodeId => address operator) public nodeOperator;
    mapping(address operator => bytes32[] nodeIds) internal _operatorNodes;

    mapping(bytes32 nodeId => mapping(uint32 version => EncryptionKey)) public encryptionKeys;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event AttestationServiceUpdated(address indexed previous, address indexed next);
    event SettlementEscrowUpdated(address indexed previous, address indexed next);
    event NodeRegistered(bytes32 indexed nodeId, address indexed operator, string metadataURI);
    event NodePayoutUpdated(bytes32 indexed nodeId, address payout);
    event NodeActiveUpdated(bytes32 indexed nodeId, bool active);
    event NodeFeeUpdated(bytes32 indexed nodeId, uint16 feeBps);
    event NodeMetadataUpdated(bytes32 indexed nodeId, string metadataURI);
    event TEEProofSet(bytes32 indexed nodeId, bytes32 teeReportHash);
    event NodeChilled(bytes32 indexed nodeId, address indexed operator);
    event NodeMarkedDefunct(bytes32 indexed nodeId, address indexed operator);
    event NodePurged(bytes32 indexed nodeId, address indexed operator);
    event EncryptionKeyRotated(
        bytes32 indexed nodeId,
        uint32 newVersion,
        bytes32 newPubkey,
        uint32 previousVersion,
        uint64 gracePeriodEnd
    );
    event EncryptionKeyRevoked(bytes32 indexed nodeId, uint32 version);

    error NotOwner();
    error NotAttestationService();
    error NotNodeOperator();
    error ZeroAddress();
    error ZeroNodeId();
    error NodeNotRegistered();
    error NodeAlreadyRegistered();
    error InvalidTEEProof();
    error InvalidLifecycle();
    error EscrowNotConfigured();
    error OpenSessionsRemain();
    error EncryptionKeyAlreadyRevoked();
    error ZeroEncryptionPubkey();
    error EncryptionKeyNotFound();
    error GracePeriodTooLong();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlyAttestationService() {
        if (msg.sender != attestationService) revert NotAttestationService();
        _;
    }

    modifier onlyNodeOperator(bytes32 nodeId) {
        if (nodeOperator[nodeId] != msg.sender) revert NotNodeOperator();
        _;
    }

    constructor(address initialOwner, address initialAttestationService) {
        if (initialOwner == address(0)) revert ZeroAddress();
        owner = initialOwner;
        attestationService = initialAttestationService;
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    function setAttestationService(address next) external onlyOwner {
        emit AttestationServiceUpdated(attestationService, next);
        attestationService = next;
    }

    /// @notice Wire the settlement escrow used for `openSessionCountByNode` reads (e.g. after both contracts are deployed).
    function setSettlementEscrow(address escrow) external onlyOwner {
        emit SettlementEscrowUpdated(settlementEscrow, escrow);
        settlementEscrow = escrow;
    }

    /// @notice Register a node identity; caller becomes the operator. `payout == address(0)` defaults to `msg.sender`.
    /// @param initialEncryptionPubkey Optional X25519 pubkey (`bytes32(0)` = opt-out). Non-zero installs version 1 on-chain.
    function registerNode(
        bytes32 nodeId,
        address payout,
        bool supportsBestEffort,
        bool supportsTEE,
        string calldata metadataURI,
        bytes32 initialEncryptionPubkey
    ) external {
        if (nodeId == bytes32(0)) revert ZeroNodeId();
        if (nodeOperator[nodeId] != address(0)) revert NodeAlreadyRegistered();

        nodeOperator[nodeId] = msg.sender;
        _operatorNodes[msg.sender].push(nodeId);

        if (initialEncryptionPubkey != bytes32(0)) {
            encryptionKeys[nodeId][1] = EncryptionKey({
                pubkey: initialEncryptionPubkey,
                activatedAt: uint64(block.timestamp),
                deprecatedAt: 0,
                revoked: false
            });
            nodes[nodeId] = NodeInfo({
                payout: payout == address(0) ? msg.sender : payout,
                feeBps: 0,
                active: true,
                supportsBestEffort: supportsBestEffort,
                supportsTEE: supportsTEE,
                teeReportHash: bytes32(0),
                metadataURI: metadataURI,
                lifecycle: NodeLifecycle.Active,
                encryptionPubkey: initialEncryptionPubkey,
                encryptionKeyVersion: 1,
                encryptionKeysLastVersion: 1
            });
        } else {
            nodes[nodeId] = NodeInfo({
                payout: payout == address(0) ? msg.sender : payout,
                feeBps: 0,
                active: true,
                supportsBestEffort: supportsBestEffort,
                supportsTEE: supportsTEE,
                teeReportHash: bytes32(0),
                metadataURI: metadataURI,
                lifecycle: NodeLifecycle.Active,
                encryptionPubkey: bytes32(0),
                encryptionKeyVersion: 0,
                encryptionKeysLastVersion: 0
            });
        }

        emit NodeRegistered(nodeId, msg.sender, metadataURI);
    }

    /// @notice Replace the active encryption key; previous key remains valid for verification until `gracePeriodSecs` elapse.
    function rotateEncryptionKey(bytes32 nodeId, bytes32 newPubkey, uint64 gracePeriodSecs)
        external
        onlyNodeOperator(nodeId)
    {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        if (newPubkey == bytes32(0)) revert ZeroEncryptionPubkey();

        NodeInfo storage n = nodes[nodeId];
        uint32 cur = n.encryptionKeyVersion;
        uint32 last = n.encryptionKeysLastVersion;
        uint32 newVer = last + 1;

        uint64 graceEnd = uint64(block.timestamp);
        if (cur > 0) {
            uint256 dep = uint256(block.timestamp) + uint256(gracePeriodSecs);
            if (dep > type(uint64).max) revert GracePeriodTooLong();
            graceEnd = uint64(dep);
            encryptionKeys[nodeId][cur].deprecatedAt = graceEnd;
            emit EncryptionKeyRotated(nodeId, newVer, newPubkey, cur, graceEnd);
        } else {
            emit EncryptionKeyRotated(nodeId, newVer, newPubkey, 0, graceEnd);
        }

        encryptionKeys[nodeId][newVer] = EncryptionKey({
            pubkey: newPubkey,
            activatedAt: uint64(block.timestamp),
            deprecatedAt: 0,
            revoked: false
        });

        n.encryptionPubkey = newPubkey;
        n.encryptionKeyVersion = newVer;
        n.encryptionKeysLastVersion = newVer;
    }

    /// @notice Mark a key version as unusable; if revoking the active version, headline pubkey is cleared (current = 0).
    function revokeEncryptionKey(bytes32 nodeId, uint32 version) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();

        EncryptionKey storage k = encryptionKeys[nodeId][version];
        if (k.pubkey == bytes32(0) && k.activatedAt == 0) revert EncryptionKeyNotFound();
        if (k.revoked) revert EncryptionKeyAlreadyRevoked();

        k.revoked = true;
        emit EncryptionKeyRevoked(nodeId, version);

        NodeInfo storage n = nodes[nodeId];
        if (n.encryptionKeyVersion == version) {
            n.encryptionPubkey = bytes32(0);
            n.encryptionKeyVersion = 0;
        }
    }

    /// @notice Active → Chilled: stops new sessions (`supportsTier`). Existing escrow sessions may still settle.
    function chillNode(bytes32 nodeId) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        NodeInfo storage n = nodes[nodeId];
        if (n.lifecycle != NodeLifecycle.Active) revert InvalidLifecycle();
        n.lifecycle = NodeLifecycle.Chilled;
        n.active = false;
        emit NodeChilled(nodeId, msg.sender);
        emit NodeActiveUpdated(nodeId, false);
    }

    /// @notice Chilled → Defunct: requires zero open sessions in the configured escrow.
    function markDefunct(bytes32 nodeId) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        NodeInfo storage n = nodes[nodeId];
        if (n.lifecycle != NodeLifecycle.Chilled) revert InvalidLifecycle();
        address esc = settlementEscrow;
        if (esc == address(0)) revert EscrowNotConfigured();
        if (ISettlementEscrowOpenSessions(esc).openSessionCountByNode(nodeId) != 0) revert OpenSessionsRemain();
        n.lifecycle = NodeLifecycle.Defunct;
        emit NodeMarkedDefunct(nodeId, msg.sender);
    }

    /// @notice Owner-only: removes a defunct node's storage so `nodeId` may be registered again.
    function purgeDefunctNode(bytes32 nodeId) external onlyOwner {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        NodeInfo storage n = nodes[nodeId];
        if (n.lifecycle != NodeLifecycle.Defunct) revert InvalidLifecycle();
        address op = nodeOperator[nodeId];
        uint32 lastVer = n.encryptionKeysLastVersion;
        _removeOperatorNode(op, nodeId);
        for (uint32 v = 1; v <= lastVer; v++) {
            delete encryptionKeys[nodeId][v];
        }
        delete nodes[nodeId];
        nodeOperator[nodeId] = address(0);
        emit NodePurged(nodeId, op);
    }

    function _removeOperatorNode(address operator, bytes32 nodeId) internal {
        bytes32[] storage ids = _operatorNodes[operator];
        uint256 len = ids.length;
        for (uint256 i = 0; i < len; i++) {
            if (ids[i] == nodeId) {
                ids[i] = ids[len - 1];
                ids.pop();
                return;
            }
        }
        revert NodeNotRegistered();
    }

    function setNodePayout(bytes32 nodeId, address payout) external onlyNodeOperator(nodeId) {
        if (payout == address(0)) revert ZeroAddress();
        nodes[nodeId].payout = payout;
        emit NodePayoutUpdated(nodeId, payout);
    }

    function setNodeActive(bytes32 nodeId, bool active) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].active = active;
        emit NodeActiveUpdated(nodeId, active);
    }

    function setNodeMetadata(bytes32 nodeId, string calldata uri) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].metadataURI = uri;
        emit NodeMetadataUpdated(nodeId, uri);
    }

    function setNodeFee(bytes32 nodeId, uint16 feeBps) external onlyOwner {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].feeBps = feeBps;
        emit NodeFeeUpdated(nodeId, feeBps);
    }

    /// @notice Attestation service records TEE evidence; Tier `TEE_VERIFIED` requires a non-zero hash (`supportsTier`).
    function setTEEProof(bytes32 nodeId, bytes32 teeReportHash) external onlyAttestationService {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        if (teeReportHash == bytes32(0)) revert InvalidTEEProof();
        nodes[nodeId].supportsTEE = true;
        nodes[nodeId].teeReportHash = teeReportHash;
        emit TEEProofSet(nodeId, teeReportHash);
    }

    /// @dev Argument is `nodeId`. Name kept for ABI compatibility with existing integrations.
    function getProvider(bytes32 nodeId) external view returns (NodeInfo memory) {
        return nodes[nodeId];
    }

    function getMetadataURI(bytes32 nodeId) external view returns (string memory) {
        return nodes[nodeId].metadataURI;
    }

    /// @notice Used by escrow and off-chain aggregators to enforce tier eligibility.
    function supportsTier(bytes32 nodeId, SecurityTier tier) external view returns (bool) {
        NodeInfo memory n = nodes[nodeId];
        if (n.lifecycle != NodeLifecycle.Active) return false;
        if (!n.active || n.payout == address(0)) return false;
        if (tier == SecurityTier.BEST_EFFORT) return n.supportsBestEffort;
        return n.supportsTEE && n.teeReportHash != bytes32(0);
    }

    /// @notice All node IDs controlled by `operator` (order matches registration).
    /// @dev Exposed as a function instead of `public mapping(...)` because Solidity maps-to-array getters use `(operator, index)`, not a full slice.
    function operatorNodes(address operator) external view returns (bytes32[] memory) {
        return _operatorNodes[operator];
    }
}
