// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title SparklNetworkConfig
/// @notice Owner-gated registry of core Sparkl hub contract addresses (CREATE2-friendly single bootstrap).
/// @dev Unified CREATE2 salt everywhere: `keccak256("sparkl.network.config.v1")` (see deploy scripts).
contract SparklNetworkConfig {
    address public owner;

    address public providerRegistry;
    address public settlementEscrow;
    address public priceOracle;
    address public modelPriceOracle;

    uint64 public version;

    error NotOwner();
    error ZeroAddress();

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event AddressesUpdated(
        address indexed providerRegistry,
        address indexed settlementEscrow,
        address indexed priceOracle,
        address modelPriceOracle,
        uint64 version_
    );

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address initialOwner) {
        if (initialOwner == address(0)) revert ZeroAddress();
        owner = initialOwner;
    }

    function setAddresses(
        address registry_,
        address escrow_,
        address oracle_,
        address modelPriceOracle_
    ) external onlyOwner {
        providerRegistry = registry_;
        settlementEscrow = escrow_;
        priceOracle = oracle_;
        modelPriceOracle = modelPriceOracle_;
        unchecked {
            version += 1;
        }
        emit AddressesUpdated(registry_, escrow_, oracle_, modelPriceOracle_, version);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }
}
