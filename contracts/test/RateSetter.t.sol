// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {RateSetter} from "../src/RateSetter.sol";

contract RateSetterTest is Test {
    RateSetter internal oracle;
    address internal owner;
    address internal updater;
    address internal stranger;

    uint256 internal constant USDC_PER_DOT = 1_340_000;
    uint256 internal constant DOT_PER_USDC = 1e24 / USDC_PER_DOT;

    function setUp() public {
        owner = makeAddr("owner");
        updater = makeAddr("updater");
        stranger = makeAddr("stranger");
        vm.prank(owner);
        oracle = new RateSetter(updater, 3600);
    }

    function test_setRate_and_read() public {
        vm.prank(updater);
        oracle.setRate(USDC_PER_DOT, DOT_PER_USDC);
        assertEq(oracle.getUsdcPerDot(), USDC_PER_DOT);
        // 1_000_000 USDC smallest units = 1 whole USDC
        assertEq(oracle.getDotForUsdc(1_000_000), DOT_PER_USDC);
        assertEq(oracle.priceUpdatedAt(), block.timestamp);
    }

    function test_setRate_revert_unauthorised() public {
        vm.prank(stranger);
        vm.expectRevert(RateSetter.Unauthorised.selector);
        oracle.setRate(USDC_PER_DOT, DOT_PER_USDC);
    }

    function test_setRate_revert_zero() public {
        vm.prank(updater);
        vm.expectRevert(RateSetter.InvalidRate.selector);
        oracle.setRate(0, DOT_PER_USDC);
    }

    function test_setRate_revert_product_mismatch() public {
        vm.prank(updater);
        vm.expectRevert(RateSetter.InvalidRate.selector);
        oracle.setRate(USDC_PER_DOT, 1);
    }

    function test_getUsdcPerDot_revert_stale_before_first_push() public {
        vm.expectRevert(RateSetter.RateTooStale.selector);
        oracle.getUsdcPerDot();
    }

    function test_maxStaleness_zero_skips_freshness() public {
        vm.prank(owner);
        RateSetter fresh = new RateSetter(updater, 0);
        assertEq(fresh.getUsdcPerDot(), 0);
    }

    function test_setUpdater_and_setMaxStaleness() public {
        address nextUpdater = makeAddr("nextUpdater");
        vm.prank(owner);
        oracle.setUpdater(nextUpdater);
        assertEq(oracle.updater(), nextUpdater);

        vm.prank(owner);
        oracle.setMaxStaleness(7200);
        assertEq(oracle.maxStaleness(), 7200);
    }

    function test_transferOwnership() public {
        address nextOwner = makeAddr("nextOwner");
        vm.prank(owner);
        oracle.transferOwnership(nextOwner);
        assertEq(oracle.owner(), nextOwner);
    }
}
