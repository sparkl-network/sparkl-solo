// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ModelPriceOracle} from "../src/ModelPriceOracle.sol";

contract ModelPriceOracleTest is Test {
  ModelPriceOracle internal oracle;
  address internal owner;
  address internal updater;
  address internal stranger;

  bytes32 internal constant LLAMA3_8B =
    keccak256(abi.encodePacked("llama3:8b"));
  bytes32 internal constant GPT4O = keccak256(abi.encodePacked("gpt-4o"));

  uint256 internal constant INPUT_PER1K = 100_000;
  uint256 internal constant OUTPUT_PER1K = 300_000;

  function setUp() public {
    owner = makeAddr("owner");
    updater = makeAddr("updater");
    stranger = makeAddr("stranger");
    vm.prank(owner);
    oracle = new ModelPriceOracle(updater);
  }

  function test_setModelPrice_and_read() public {
    vm.prank(updater);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);

    (
      uint256 inputPer1k,
      uint256 outputPer1k,
      uint64 updatedAt,
      bool active
    ) = oracle.prices(LLAMA3_8B);

    assertEq(inputPer1k, INPUT_PER1K);
    assertEq(outputPer1k, OUTPUT_PER1K);
    assertEq(updatedAt, uint64(block.timestamp));
    assertTrue(active);
    assertEq(oracle.modelIdsLength(), 1);
    assertEq(oracle.modelIds(0), LLAMA3_8B);
  }

  function test_setModelPrice_revert_unauthorised() public {
    vm.prank(stranger);
    vm.expectRevert(ModelPriceOracle.Unauthorised.selector);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);
  }

  function test_setModelPrice_revert_zero() public {
    vm.prank(updater);
    vm.expectRevert(ModelPriceOracle.InvalidPrice.selector);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", 0, OUTPUT_PER1K);
  }

  function test_setModelPrice_does_not_duplicate_modelIds() public {
    vm.startPrank(updater);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K + 1, OUTPUT_PER1K + 1);
    vm.stopPrank();

    assertEq(oracle.modelIdsLength(), 1);
    (uint256 inputPer1k,,,) = oracle.prices(LLAMA3_8B);
    assertEq(inputPer1k, INPUT_PER1K + 1);
  }

  function test_delistModel() public {
    vm.prank(updater);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);

    vm.prank(updater);
    oracle.delistModel(LLAMA3_8B);

    (uint256 inputPer1k, uint256 outputPer1k,, bool active) = oracle.prices(LLAMA3_8B);
    assertEq(inputPer1k, INPUT_PER1K);
    assertEq(outputPer1k, OUTPUT_PER1K);
    assertFalse(active);
    assertEq(oracle.modelIdsLength(), 1);
  }

  function test_delistModel_revert_unauthorised() public {
    vm.prank(updater);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);

    vm.prank(stranger);
    vm.expectRevert(ModelPriceOracle.Unauthorised.selector);
    oracle.delistModel(LLAMA3_8B);
  }

  function test_delistModel_revert_not_found() public {
    vm.prank(updater);
    vm.expectRevert(ModelPriceOracle.ModelNotFound.selector);
    oracle.delistModel(LLAMA3_8B);
  }

  function test_multiple_models() public {
    vm.startPrank(updater);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);
    oracle.setModelPrice(GPT4O, "gpt-4o", INPUT_PER1K * 2, OUTPUT_PER1K * 2);
    vm.stopPrank();

    assertEq(oracle.modelIdsLength(), 2);
    assertEq(oracle.modelIds(0), LLAMA3_8B);
    assertEq(oracle.modelIds(1), GPT4O);
  }

  function test_setUpdater_and_transferOwnership() public {
    address nextUpdater = makeAddr("nextUpdater");
    vm.prank(owner);
    oracle.setUpdater(nextUpdater);
    assertEq(oracle.updater(), nextUpdater);

    address nextOwner = makeAddr("nextOwner");
    vm.prank(owner);
    oracle.transferOwnership(nextOwner);
    assertEq(oracle.owner(), nextOwner);
  }

  function test_setDefaultPrice_and_getEffectivePrice_unknown_model() public {
    uint256 defaultIn = 50_000;
    uint256 defaultOut = 150_000;

    vm.prank(updater);
    oracle.setDefaultPrice(defaultIn, defaultOut);

    bytes32 unknown = keccak256(abi.encodePacked("unknown-model"));
    (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault) =
      oracle.getEffectivePrice(unknown);

    assertEq(inputPer1k, defaultIn);
    assertEq(outputPer1k, defaultOut);
    assertTrue(usedDefault);
  }

  function test_getEffectivePrice_active_model() public {
    vm.startPrank(updater);
    oracle.setDefaultPrice(1, 1);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);
    vm.stopPrank();

    (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault) =
      oracle.getEffectivePrice(LLAMA3_8B);

    assertEq(inputPer1k, INPUT_PER1K);
    assertEq(outputPer1k, OUTPUT_PER1K);
    assertFalse(usedDefault);
  }

  function test_getEffectivePrice_delisted_uses_default() public {
    vm.startPrank(updater);
    oracle.setDefaultPrice(50_000, 150_000);
    oracle.setModelPrice(LLAMA3_8B, "llama3:8b", INPUT_PER1K, OUTPUT_PER1K);
    oracle.delistModel(LLAMA3_8B);
    vm.stopPrank();

    (uint256 inputPer1k, uint256 outputPer1k, bool usedDefault) =
      oracle.getEffectivePrice(LLAMA3_8B);

    assertEq(inputPer1k, 50_000);
    assertEq(outputPer1k, 150_000);
    assertTrue(usedDefault);
  }

  function test_getEffectivePrice_revert_no_default() public {
    bytes32 unknown = keccak256(abi.encodePacked("unknown-model"));
    vm.expectRevert(ModelPriceOracle.InvalidPrice.selector);
    oracle.getEffectivePrice(unknown);
  }
}
