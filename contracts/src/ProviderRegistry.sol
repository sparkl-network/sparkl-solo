// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, NodeInfo} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";

/// @title ProviderRegistry
/// @notice Registry of provider nodes: `nodeId` is the node identity; `msg.sender` registers as the operator.
contract ProviderRegistry is IProviderRegistry {
    address public owner;
    address public attestationService;

    mapping(address nodeId => NodeInfo) public nodes;
    mapping(address nodeId => address operator) public nodeOperator;
    mapping(address operator => address[] nodeIds) internal _operatorNodes;
    mapping(address nodeId => mapping(SecurityTier tier => uint256 pricePer1k)) internal _pricePer1k;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event AttestationServiceUpdated(address indexed previous, address indexed next);
    event NodeRegistered(address indexed nodeId, address indexed operator, string metadataURI);
    event NodePayoutUpdated(address indexed nodeId, address payout);
    event NodeActiveUpdated(address indexed nodeId, bool active);
    event NodeFeeUpdated(address indexed nodeId, uint16 feeBps);
    event NodeMetadataUpdated(address indexed nodeId, string metadataURI);
    event TEEProofSet(address indexed nodeId, bytes32 teeReportHash);
    event PricingUpdated(address indexed nodeId, SecurityTier tier, uint256 pricePer1kTokens);

    error NotOwner();
    error NotAttestationService();
    error NotNodeOperator();
    error ZeroAddress();
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

    modifier onlyNodeOperator(address nodeId) {
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
        address nodeId,
        address payout,
        bool supportsBestEffort,
        bool supportsTEE,
        string calldata metadataURI
    ) external {
        if (nodeId == address(0)) revert ZeroAddress();
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

    function setNodePayout(address nodeId, address payout) external onlyNodeOperator(nodeId) {
        if (payout == address(0)) revert ZeroAddress();
        nodes[nodeId].payout = payout;
        emit NodePayoutUpdated(nodeId, payout);
    }

    function setNodeActive(address nodeId, bool active) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].active = active;
        emit NodeActiveUpdated(nodeId, active);
    }

    function setNodeMetadata(address nodeId, string calldata uri) external onlyNodeOperator(nodeId) {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].metadataURI = uri;
        emit NodeMetadataUpdated(nodeId, uri);
    }

    function setNodeFee(address nodeId, uint16 feeBps) external onlyOwner {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        nodes[nodeId].feeBps = feeBps;
        emit NodeFeeUpdated(nodeId, feeBps);
    }

    /// @notice Attestation service records TEE evidence; Tier `TEE_VERIFIED` requires a non-zero hash (`supportsTier`).
    function setTEEProof(address nodeId, bytes32 teeReportHash) external onlyAttestationService {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        if (teeReportHash == bytes32(0)) revert InvalidTEEProof();
        nodes[nodeId].supportsTEE = true;
        nodes[nodeId].teeReportHash = teeReportHash;
        emit TEEProofSet(nodeId, teeReportHash);
    }

    /// @notice Declared price for `tier` per 1_000 tokens (internal DOT units), matching `SettlementEscrow` accounting.
    function setNodePricing(address nodeId, SecurityTier tier, uint256 pricePer1kTokens)
        external
        onlyNodeOperator(nodeId)
    {
        if (nodeOperator[nodeId] == address(0)) revert NodeNotRegistered();
        _pricePer1k[nodeId][tier] = pricePer1kTokens;
        emit PricingUpdated(nodeId, tier, pricePer1kTokens);
    }

    /// @dev Argument is `nodeId`. Name kept for ABI compatibility with existing integrations.
    function getProvider(address nodeId) external view returns (NodeInfo memory) {
        return nodes[nodeId];
    }

    function getMetadataURI(address nodeId) external view returns (string memory) {
        return nodes[nodeId].metadataURI;
    }

    function getPricePer1k(address nodeId, SecurityTier tier) external view returns (uint256) {
        return _pricePer1k[nodeId][tier];
    }

    /// @notice Used by escrow and off-chain aggregators to enforce tier eligibility.
    function supportsTier(address nodeId, SecurityTier tier) external view returns (bool) {
        NodeInfo memory n = nodes[nodeId];
        if (!n.active || n.payout == address(0)) return false;
        if (tier == SecurityTier.BEST_EFFORT) return n.supportsBestEffort;
        return n.supportsTEE && n.teeReportHash != bytes32(0);
    }

    /// @notice All node IDs controlled by `operator` (order matches registration).
    /// @dev Exposed as a function instead of `public mapping(...)` because Solidity maps-to-array getters use `(operator, index)`, not a full slice.
    function operatorNodes(address operator) external view returns (address[] memory) {
        return _operatorNodes[operator];
    }
}
