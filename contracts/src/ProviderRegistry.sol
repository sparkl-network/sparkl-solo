// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier, ProviderInfo} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";

/// @title ProviderRegistry
/// @notice On-chain registry of providers, payout addresses, tier capabilities, TEE evidence hash, and per-tier pricing.
contract ProviderRegistry is IProviderRegistry {
    address public owner;
    address public attestationService;

    mapping(address => ProviderInfo) internal _providers;
    mapping(address => mapping(SecurityTier => uint256)) internal _pricePer1k;
    mapping(address => string) internal _metadataURI;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event AttestationServiceUpdated(address indexed previous, address indexed next);
    event ProviderRegistered(
        address indexed provider,
        address payout,
        bool supportsBestEffort,
        bool supportsTEE,
        string metadataURI
    );
    event ProviderPayoutUpdated(address indexed provider, address payout);
    event ProviderActiveUpdated(address indexed provider, bool active);
    event ProviderFeeUpdated(address indexed provider, uint16 feeBps);
    event TEEProofSet(address indexed provider, bytes32 teeReportHash);
    event PricingUpdated(address indexed provider, SecurityTier tier, uint256 pricePer1kTokens);

    error NotOwner();
    error NotAttestationService();
    error ZeroAddress();
    error ProviderNotRegistered();
    error InvalidTEEProof();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlyAttestationService() {
        if (msg.sender != attestationService) revert NotAttestationService();
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

    /// @notice Provider self-registration. `feeBps` defaults to 0; use `setProviderFee` (owner) for platform rev-share if needed.
    function registerProvider(
        address payout,
        bool supportsBestEffort,
        bool supportsTEE,
        string calldata metadataURI
    ) external {
        address provider = msg.sender;
        if (payout == address(0)) revert ZeroAddress();

        _providers[provider] = ProviderInfo({
            payout: payout,
            feeBps: 0,
            active: true,
            supportsBestEffort: supportsBestEffort,
            // TEE-verified tier is only valid after attestation (`setTEEProof`).
            supportsTEE: false,
            teeReportHash: bytes32(0)
        });
        _metadataURI[provider] = metadataURI;

        emit ProviderRegistered(provider, payout, supportsBestEffort, supportsTEE, metadataURI);
    }

    function setProviderPayout(address payout) external {
        address provider = msg.sender;
        if (_providers[provider].payout == address(0)) revert ProviderNotRegistered();
        if (payout == address(0)) revert ZeroAddress();
        _providers[provider].payout = payout;
        emit ProviderPayoutUpdated(provider, payout);
    }

    function setProviderActive(bool active) external {
        address provider = msg.sender;
        if (_providers[provider].payout == address(0)) revert ProviderNotRegistered();
        _providers[provider].active = active;
        emit ProviderActiveUpdated(provider, active);
    }

    function setProviderFee(address provider, uint16 feeBps) external onlyOwner {
        if (_providers[provider].payout == address(0)) revert ProviderNotRegistered();
        _providers[provider].feeBps = feeBps;
        emit ProviderFeeUpdated(provider, feeBps);
    }

    /// @notice Attestation service records evidence; this is the on-chain source of truth for Tier A (`TEE_VERIFIED`).
    /// @dev MVP stub: sibling folder services/tee-attestation-stub; verify attestation payloads before production use.
    function setTEEProof(address provider, bytes32 teeReportHash) external onlyAttestationService {
        if (_providers[provider].payout == address(0)) revert ProviderNotRegistered();
        if (teeReportHash == bytes32(0)) revert InvalidTEEProof();
        _providers[provider].supportsTEE = true;
        _providers[provider].teeReportHash = teeReportHash;
        emit TEEProofSet(provider, teeReportHash);
    }

    /// @notice Declared price for `tier` in DOT internal units (1e18 per whole DOT) per 1_000 tokens, matching `SettlementEscrow` accounting.
    function setPricing(SecurityTier tier, uint256 pricePer1kTokens) external {
        address provider = msg.sender;
        if (_providers[provider].payout == address(0)) revert ProviderNotRegistered();
        _pricePer1k[provider][tier] = pricePer1kTokens;
        emit PricingUpdated(provider, tier, pricePer1kTokens);
    }

    function getProvider(address provider) external view returns (ProviderInfo memory) {
        return _providers[provider];
    }

    function getMetadataURI(address provider) external view returns (string memory) {
        return _metadataURI[provider];
    }

    function getPricePer1k(address provider, SecurityTier tier) external view returns (uint256) {
        return _pricePer1k[provider][tier];
    }

    /// @notice Used by escrow and off-chain aggregators to enforce tier eligibility.
    function supportsTier(address provider, SecurityTier tier) external view returns (bool) {
        ProviderInfo memory p = _providers[provider];
        if (!p.active || p.payout == address(0)) return false;
        if (tier == SecurityTier.BEST_EFFORT) return p.supportsBestEffort;
        return p.supportsTEE && p.teeReportHash != bytes32(0);
    }
}
