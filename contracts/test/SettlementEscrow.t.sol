// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {SecurityTier} from "../src/SecurityTypes.sol";

contract SettlementEscrowTest is Test {
    ProviderRegistry internal reg;
    MockOracle internal oracle;
    MockERC20 internal usdc;
    SettlementEscrow internal esc;

    address internal owner = address(0xAce0);
    address internal attestation = address(0xA777);
    address internal providerA = address(0xB00B);
    address internal payout = address(0xCAFE);
    address internal alice = address(0xA11CE);

    /// @dev USDC (6-dec) smallest per 1e18 internal DOT: 0.5 USDC per DOT → 1 USDC buys 2 DOT internal.
    uint256 internal constant USDC_PER_DOT = 500_000;

    function _internalPer1Usdc() internal pure returns (uint256) {
        return (1_000_000 * 1e18) / USDC_PER_DOT;
    }

    function setUp() public {
        vm.prank(owner);
        reg = new ProviderRegistry(owner, attestation);

        oracle = new MockOracle();
        oracle.set(USDC_PER_DOT);

        usdc = new MockERC20("USDC", 6);

        esc = new SettlementEscrow(reg, oracle, usdc);

        vm.prank(providerA);
        reg.registerProvider(payout, true, true, "");
        vm.prank(attestation);
        reg.setTEEProof(providerA, bytes32(uint256(0xbeef)));

        vm.deal(alice, 1000 ether);
    }

    function test_mockOracle_priceAndTimestamp() public view {
        (uint256 p, uint256 ts) = oracle.getUsdcPerDotWithTimestamp();
        assertEq(p, USDC_PER_DOT);
        assertEq(ts, block.timestamp);

        assertEq(oracle.getUsdcPerDot(), USDC_PER_DOT);
        assertEq(oracle.getDotForUsdc(500_000), 1e18);
    }

    function test_depositDot_withdrawRoundTrip() public {
        uint256 nativeIn = 10 * 10 ** 10;
        uint256 internalExpected = 10 * 10 ** 18;

        vm.prank(alice);
        esc.depositDot{value: nativeIn}();
        assertEq(esc.dotBalances(alice), internalExpected);

        uint256 balBefore = alice.balance;
        vm.prank(alice);
        esc.withdrawDot(internalExpected);
        assertEq(esc.dotBalances(alice), 0);
        assertEq(alice.balance - balBefore, nativeIn);
    }

    function test_depositDot_revert_zero() public {
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.BadAmount.selector);
        esc.depositDot{value: 0}();
    }

    function test_withdraw_revert_insufficient() public {
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.InsufficientBalance.selector);
        esc.withdrawDot(1);
    }

    function test_depositUsdcAsDot() public {
        uint256 usdcAmount = 3 * 1_000_000;
        usdc.mint(alice, usdcAmount);

        vm.startPrank(alice);
        usdc.approve(address(esc), usdcAmount);
        esc.depositUsdcAsDot(usdcAmount);
        vm.stopPrank();

        uint256 expInternal = 3 * _internalPer1Usdc();
        assertEq(expInternal, 3 * 2e18);
        assertEq(esc.dotBalances(alice), expInternal);
        assertEq(usdc.balanceOf(address(esc)), usdcAmount);
    }

    function test_depositUsdcAsDot_revert_zeroOraclePrice() public {
        oracle.set(0);
        usdc.mint(alice, 1_000_000);
        vm.startPrank(alice);
        usdc.approve(address(esc), 1_000_000);
        vm.expectRevert(SettlementEscrow.BadAmount.selector);
        esc.depositUsdcAsDot(1_000_000);
        vm.stopPrank();
    }

    function test_openSession_fromBalance_bestEffort() public {
        address providerB = address(0xB0B2);
        vm.prank(providerB);
        reg.registerProvider(payout, true, false, "");

        uint256 nativeIn = 5 * 10 ** 10;
        vm.prank(alice);
        esc.depositDot{value: nativeIn}();

        uint256 amountInternal = 3 * 10 ** 18;
        vm.prank(alice);
        esc.openSession(providerB, SecurityTier.BEST_EFFORT, amountInternal);

        assertEq(esc.nextSessionId(), 1);
        (address u, address p, SecurityTier t, uint256 amt, uint64 opened, bool settled) = esc.sessions(0);
        assertEq(u, alice);
        assertEq(p, providerB);
        assertEq(uint8(t), uint8(SecurityTier.BEST_EFFORT));
        assertEq(amt, amountInternal);
        assertGt(opened, 0);
        assertFalse(settled);
        assertEq(esc.dotBalances(alice), 5 * 10 ** 18 - amountInternal);
    }

    function test_openSession_payable_native() public {
        uint256 amountInternal = 2 * 10 ** 18;
        uint256 native = 2 * 10 ** 10;

        vm.prank(alice);
        esc.openSession{value: native}(providerA, SecurityTier.TEE_VERIFIED, amountInternal);

        assertEq(esc.dotBalances(alice), 0);
        assertEq(address(esc).balance, native);
        (, address p, SecurityTier t, uint256 amt,,) = esc.sessions(0);
        assertEq(p, providerA);
        assertEq(uint8(t), uint8(SecurityTier.TEE_VERIFIED));
        assertEq(amt, amountInternal);
    }

    function test_openSession_revert_unsupportedTier() public {
        address providerB = address(0xB0B2);
        vm.prank(providerB);
        reg.registerProvider(payout, true, false, "");

        uint256 amountInternal = 1 * 10 ** 18;
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.UnsupportedTier.selector);
        esc.openSession(providerB, SecurityTier.TEE_VERIFIED, amountInternal);
    }

    function test_openSession_revert_wrongMsgValue() public {
        uint256 amountInternal = 1 * 10 ** 18;
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.BadAmount.selector);
        esc.openSession{value: 1}(providerA, SecurityTier.TEE_VERIFIED, amountInternal);
    }

    function test_openSession_revert_insufficientBalancePath() public {
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.InsufficientBalance.selector);
        esc.openSession(providerA, SecurityTier.TEE_VERIFIED, 10 ** 18);
    }
}
