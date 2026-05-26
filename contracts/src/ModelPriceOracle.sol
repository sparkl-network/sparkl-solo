// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IModelPriceOracle} from "./interfaces/IModelPriceOracle.sol";

/// @title ModelPriceOracle
/// @notice Network reference pricing for inference models. A trusted off-chain service
///         pushes market-rate input/output prices per 1k tokens in internal DOT units.
///         SettlementEscrow bills sessions using `getEffectivePrice`.
contract ModelPriceOracle is IModelPriceOracle {
  // ─── Errors ───────────────────────────────────────────────────────────────
  error Unauthorised();
  error InvalidPrice();
  error ModelNotFound();

  // ─── Events ───────────────────────────────────────────────────────────────
  event PriceUpdated(
    bytes32 indexed modelId,
    string name,
    uint256 inputPer1k,
    uint256 outputPer1k
  );
  event DefaultPriceUpdated(uint256 inputPer1k, uint256 outputPer1k, uint256 timestamp);
  event ModelDelisted(bytes32 indexed modelId);
  event UpdaterChanged(address indexed previous, address indexed next);
  event OwnerChanged(address indexed previous, address indexed next);

  // ─── Types ────────────────────────────────────────────────────────────────
  struct ModelPrice {
    uint256 inputPer1kTokens; // internal DOT units per 1k input tokens
    uint256 outputPer1kTokens; // internal DOT units per 1k output tokens
    uint64 updatedAt; // unix timestamp
    bool active; // false = model delisted
  }

  struct DefaultPrice {
    uint256 inputPer1kTokens;
    uint256 outputPer1kTokens;
    uint64 updatedAt;
  }

  // ─── State ────────────────────────────────────────────────────────────────
  address public owner;
  address public updater; // the oracle service key

  DefaultPrice public defaultPrice;

  // modelId is keccak256(abi.encodePacked(modelName)) — e.g. keccak256("llama3:8b")
  mapping(bytes32 modelId => ModelPrice) public prices;
  bytes32[] public modelIds; // enumerable list for the /model UI
  mapping(bytes32 modelId => bool) private _knownModel;

  // ─── Constructor ──────────────────────────────────────────────────────────
  constructor(address _updater) {
    owner = msg.sender;
    updater = _updater;
  }

  // ─── Price setter (called by oracle service) ──────────────────────────────
  /// @notice Push or update reference pricing for a model.
  function setModelPrice(
    bytes32 modelId,
    string calldata name,
    uint256 inputPer1k,
    uint256 outputPer1k
  ) external {
    if (msg.sender != updater) revert Unauthorised();
    if (inputPer1k == 0 || outputPer1k == 0) revert InvalidPrice();

    if (!_knownModel[modelId]) {
      _knownModel[modelId] = true;
      modelIds.push(modelId);
    }

    prices[modelId] = ModelPrice({
      inputPer1kTokens: inputPer1k,
      outputPer1kTokens: outputPer1k,
      updatedAt: uint64(block.timestamp),
      active: true
    });

    emit PriceUpdated(modelId, name, inputPer1k, outputPer1k);
  }

  /// @notice Set fallback pricing for unknown or delisted models.
  function setDefaultPrice(uint256 inputPer1k, uint256 outputPer1k) external {
    if (msg.sender != updater) revert Unauthorised();
    if (inputPer1k == 0 || outputPer1k == 0) revert InvalidPrice();

    defaultPrice = DefaultPrice({
      inputPer1kTokens: inputPer1k,
      outputPer1kTokens: outputPer1k,
      updatedAt: uint64(block.timestamp)
    });

    emit DefaultPriceUpdated(inputPer1k, outputPer1k, block.timestamp);
  }

  /// @notice Delist a model without deleting its last-known pricing.
  function delistModel(bytes32 modelId) external {
    if (msg.sender != updater) revert Unauthorised();
    ModelPrice storage p = prices[modelId];
    if (!_knownModel[modelId] || p.updatedAt == 0) revert ModelNotFound();

    p.active = false;

    emit ModelDelisted(modelId);
  }

  // ─── IModelPriceOracle ────────────────────────────────────────────────────
  /// @inheritdoc IModelPriceOracle
  function getEffectivePrice(bytes32 modelId)
    external
    view
    returns (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault)
  {
    ModelPrice storage p = prices[modelId];
    if (p.active && p.updatedAt != 0) {
      return (p.inputPer1kTokens, p.outputPer1kTokens, false);
    }

    DefaultPrice storage d = defaultPrice;
    if (d.updatedAt == 0 || d.inputPer1kTokens == 0 || d.outputPer1kTokens == 0) {
      revert InvalidPrice();
    }
    return (d.inputPer1kTokens, d.outputPer1kTokens, true);
  }

  // ─── Views ────────────────────────────────────────────────────────────────
  function modelIdsLength() external view returns (uint256) {
    return modelIds.length;
  }

  // ─── Admin ────────────────────────────────────────────────────────────────
  function setUpdater(address next) external {
    if (msg.sender != owner) revert Unauthorised();
    emit UpdaterChanged(updater, next);
    updater = next;
  }

  function transferOwnership(address next) external {
    if (msg.sender != owner) revert Unauthorised();
    emit OwnerChanged(owner, next);
    owner = next;
  }
}
