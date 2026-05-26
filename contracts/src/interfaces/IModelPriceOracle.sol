// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Per-model inference pricing for on-chain billing (internal DOT units per 1k tokens).
interface IModelPriceOracle {
    /// @return inputPer1k internal DOT units per 1k input tokens
    /// @return outputPer1k internal DOT units per 1k output tokens
    /// @return usedDefault true when active model price was unavailable and default was used
    function getEffectivePrice(bytes32 modelId)
        external
        view
        returns (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault);
}
