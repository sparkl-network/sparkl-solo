// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, NodeInfo} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";

/// @title ProviderRegistry
/// @notice Registry of provider nodes: `nodeId` is a stable identity (e.g. Substrate PeerId hash); `msg.sender` registers as the operator.
contract ProviderRegistry is IProviderRegistry {
    address public owner;
    address public attestationService;

    mapping(bytes32 nodeId => NodeInfo) public nodes;
    mapping(bytes32 nodeId => address operator) public nodeOperator;
    mapping(address operator => bytes32[] nodeIds) internal _operatorNodes;
    mapping(bytes32 nodeId => mapping(SecurityTier tier => uint256 pricePer1k)) internal _pricePer1k;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event AttestationServiceUpdated(address indexed previous, address indexed next);
    event NodeRegistered(bytes32 indexed nodeId, address indexed operator, string metadataURI);
    event NodePayoutUpdated(bytes32 indexed nodeId, address payout);
    event NodeActiveUpdated(bytes32 indexed nodeId, bool active);
    event NodeFeeUpdated(bytes32 indexed nodeId, uint16 feeBps);
    event NodeMetadataUpdated(bytes32 indexed nodeId, string metadataURI);
    event TEEProofSet(bytes32 indexed nodeId, bytes32 teeReportHash);
    event PricingUpdated(bytes32 indexed nodeId, SecurityTier tier, uint256 pricePer1kTokens);
    event NodeDeregistered(bytes32 indexed nodeId, address indexed operator);

    error NotOwner();
    error NotAttestationService();
    error NotNodeOperator();
    error ZeroAddress();
    error ZeroNodeId();
    error NodeNotRegistered();
    error NodeAlreadyRegistered();
    error InvalidTEEProof();

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

    /// @notice Register a node identity; caller becomes the operator. `payout == address(0)` defaults to `msg.sender`.
    function registerNode(
        bytes32 nodeId,
        address payout,
        bool supportsBestEffort,
        bool supportsTEE,
        string calldata metadataURI
    ) external {
        if (nodeId == bytes32(0)) revert ZeroNodeId();
        if (nodeOperator[nodeId] != address(0)) revert NodeAlreadyRegistered();

        nodeOperator[nodeId] = msg.sender;
        _operatorNodes[msg.sender].push(nodeId);
        nodes[nodeId] = NodeInfo({
            payout: payout == address(0) ? msg.sender : payout,
            feeBps: 0,
            active: true,
            supportsBestEffort: supportsBestEffort,
            supportsTEE: supportsTEE,
            teeReportHash: bytes32(0),
            metadataURI: metadataURI
        });

        emit NodeRegistered(nodeId, msg.sender, metadataURI);
    }

    /// @notice Permanently remove this node from the registry. The same `nodeId` may be registered again later (e.g. new operator).
    function deregisterNode(bytes32 nodeId) external onlyNodeOperator(nodeId) {
        address op = msg.sender;
        _removeOperatorNode(op, nodeId);
        _pricePer1k[nodeId][SecurityTier.BEST_EFFORT] = 0;
        _pricePer1k[nodeId][SecurityTier.TEE_VERIFIED] = 0;
        delete nodes[nodeId];
        nodeOperator[nodeId] = address(0);
        emit NodeDeregistered(nodeId, op);
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

    /// @notice Declared price for `tier` per 1_000 tokens (internal DOT units), matching `SettlementEscrow` accounting.
    function setNodePricing(bytes32 nodeId, SecurityTier tier, uint256 pricePer1kTokens)
        external
        onlyNodeOperator(nodeId)
    {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        _pricePer1k[nodeId][tier] = pricePer1kTokens;
        emit PricingUpdated(nodeId, tier, pricePer1kTokens);
    }

    /// @dev Argument is `nodeId`. Name kept for ABI compatibility with existing integrations.
    function getProvider(bytes32 nodeId) external view returns (NodeInfo memory) {
        return nodes[nodeId];
    }

    function getMetadataURI(bytes32 nodeId) external view returns (string memory) {
        return nodes[nodeId].metadataURI;
    }

    function getPricePer1k(bytes32 nodeId, SecurityTier tier) external view returns (uint256) {
        return _pricePer1k[nodeId][tier];
    }

    /// @notice Used by escrow and off-chain aggregators to enforce tier eligibility.
    function supportsTier(bytes32 nodeId, SecurityTier tier) external view returns (bool) {
        NodeInfo memory n = nodes[nodeId];
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
