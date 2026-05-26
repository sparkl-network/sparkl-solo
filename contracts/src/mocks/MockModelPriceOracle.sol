// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IModelPriceOracle} from "../interfaces/IModelPriceOracle.sol";

/// @notice Test helper: set per-model and default prices without updater auth.
contract MockModelPriceOracle is IModelPriceOracle {
    error InvalidPrice();

    struct ModelPrice {
        uint256 inputPer1k;
        uint256 outputPer1k;
        bool active;
    }

    uint256 public defaultInputPer1k;
    uint256 public defaultOutputPer1k;
    mapping(bytes32 => ModelPrice) internal _prices;

    function setDefault(uint256 inputPer1k, uint256 outputPer1k) external {
        if (inputPer1k == 0 || outputPer1k == 0) revert InvalidPrice();
        defaultInputPer1k = inputPer1k;
        defaultOutputPer1k = outputPer1k;
    }

    function setModel(bytes32 modelId, uint256 inputPer1k, uint256 outputPer1k) external {
        if (inputPer1k == 0 || outputPer1k == 0) revert InvalidPrice();
        _prices[modelId] = ModelPrice({inputPer1k: inputPer1k, outputPer1k: outputPer1k, active: true});
    }

    function delistModel(bytes32 modelId) external {
        _prices[modelId].active = false;
    }

    function getEffectivePrice(bytes32 modelId)
        external
        view
        returns (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault)
    {
        ModelPrice storage p = _prices[modelId];
        if (p.active && p.inputPer1k != 0) {
            return (p.inputPer1k, p.outputPer1k, false);
        }
        if (defaultInputPer1k == 0 || defaultOutputPer1k == 0) revert InvalidPrice();
        return (defaultInputPer1k, defaultOutputPer1k, true);
    }
}
