// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {SecurityTier, ProviderInfo} from "../src/SecurityTypes.sol";

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

    /// @dev USDC (6-dec) smallest per 1e18 internal DOT: baseline ~1.34 USD/DOT (May 2026 spot).
    uint256 internal constant USDC_PER_DOT = 1_340_000;

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
        assertEq(oracle.getDotForUsdc(USDC_PER_DOT), 1e18);
        assertEq(oracle.priceUpdatedAt(), block.timestamp);
    }

    function test_depositDot_withdrawRoundTrip() public {
        uint256 nativeIn = 10 * 10 ** 10;
        uint256 internalExpected = 10 * 10 ** 18;

        vm.prank(alice);
        esc.depositDot{value: nativeIn}();
        assertEq(esc.dotBalances(alice), internalExpected);
        assertEq(esc.internalCirculating(), internalExpected);

        uint256 balBefore = alice.balance;
        vm.prank(alice);
        esc.withdrawDot(internalExpected);
        assertEq(esc.dotBalances(alice), 0);
        assertEq(esc.internalCirculating(), 0);
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

        // Matches contract formula `(usdc * 1e18) / usdcPerDot` (not `3 * per‑1‑USDC` — integer division differs).
        uint256 expInternal = (usdcAmount * 1e18) / USDC_PER_DOT;
        assertEq(esc.dotBalances(alice), expInternal);
        assertEq(esc.internalCirculating(), expInternal);
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

    function test_depositUsdcAsDot_revert_roundingToZeroInternal() public {
        // Force `credited == 0`: at default price 1 wei USDC would still mint; use an extreme oracle.
        oracle.set(type(uint256).max / 2);
        uint256 amt = 1_000_000;
        usdc.mint(alice, amt);
        vm.startPrank(alice);
        usdc.approve(address(esc), amt);
        vm.expectRevert(SettlementEscrow.BadAmount.selector);
        esc.depositUsdcAsDot(amt);
        vm.stopPrank();
    }

    function test_depositUsdcAsDot_revert_slippage() public {
        uint256 usdcAmount = 1_000_000;
        usdc.mint(alice, usdcAmount);
        vm.startPrank(alice);
        usdc.approve(address(esc), usdcAmount);
        uint256 expected = _internalPer1Usdc();
        vm.expectRevert(SettlementEscrow.Slippage.selector);
        esc.depositUsdcAsDot(usdcAmount, expected + 1, type(uint256).max);
        vm.stopPrank();
    }

    function test_depositUsdcAsDot_revert_oracleStale() public {
        vm.warp(50_000);
        oracle.set(USDC_PER_DOT);
        oracle.setTimestamp(10_000);

        uint256 usdcAmount = 1_000_000;
        usdc.mint(alice, usdcAmount);

        vm.startPrank(alice);
        usdc.approve(address(esc), usdcAmount);
        vm.expectRevert(SettlementEscrow.OracleStale.selector);
        esc.depositUsdcAsDot(usdcAmount, 0, 9999);
        vm.stopPrank();
    }

    function test_depositUsdcAsDot_ok_whenMaxAgeUnchecked() public {
        uint256 usdcAmount = 1_000_000;
        usdc.mint(alice, usdcAmount);
        oracle.setTimestamp(1);

        vm.startPrank(alice);
        usdc.approve(address(esc), usdcAmount);
        esc.depositUsdcAsDot(usdcAmount, 0, type(uint256).max);
        vm.stopPrank();
        assertEq(esc.dotBalances(alice), _internalPer1Usdc());
    }

    function test_depositUsdcAsDot_overflow_revertsSolidityCheckedMultiply() public {
        uint256 humongous = type(uint256).max / 1e18 + 1;
        usdc.mint(alice, humongous);
        vm.startPrank(alice);
        usdc.approve(address(esc), humongous);
        vm.expectRevert();
        esc.depositUsdcAsDot(humongous);
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
        uint256 circBefore = esc.internalCirculating();
        vm.prank(alice);
        esc.openSession(providerB, SecurityTier.BEST_EFFORT, amountInternal);

        assertEq(esc.nextSessionId(), 1);
        (
            address u,
            address p,
            SecurityTier t,
            uint256 locked,
            uint256 usage,
            uint256 opening,
            uint64 opened,
            bool settled
        ) = esc.sessions(0);
        assertEq(u, alice);
        assertEq(p, providerB);
        assertEq(uint8(t), uint8(SecurityTier.BEST_EFFORT));
        assertEq(locked, amountInternal);
        assertEq(usage, 0);
        assertEq(opening, amountInternal);
        assertGt(opened, 0);
        assertFalse(settled);
        assertEq(esc.dotBalances(alice), 5 * 10 ** 18 - amountInternal);
        assertEq(esc.totalLockedInternal(), amountInternal);
        assertEq(esc.internalCirculating(), circBefore);
    }

    function test_openSession_payable_native() public {
        uint256 amountInternal = 2 * 10 ** 18;
        uint256 native = 2 * 10 ** 10;

        vm.prank(alice);
        esc.openSession{value: native}(providerA, SecurityTier.TEE_VERIFIED, amountInternal);

        assertEq(esc.dotBalances(alice), 0);
        assertEq(address(esc).balance, native);
        assertEq(esc.internalCirculating(), amountInternal);
        assertEq(esc.totalLockedInternal(), amountInternal);
        (, address p, SecurityTier t, uint256 locked,, uint256 opening,,) = esc.sessions(0);
        assertEq(p, providerA);
        assertEq(uint8(t), uint8(SecurityTier.TEE_VERIFIED));
        assertEq(locked, amountInternal);
        assertEq(opening, amountInternal);
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

    function test_tieredProvider_teeSessionRequiresAttestation() public {
        address hybrid = address(0xBEEF11);
        vm.prank(hybrid);
        reg.registerProvider(payout, true, false, "");

        uint256 amt = 1 ether;
        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.UnsupportedTier.selector);
        esc.openSession(hybrid, SecurityTier.TEE_VERIFIED, amt);

        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.InsufficientBalance.selector);
        esc.openSession(hybrid, SecurityTier.BEST_EFFORT, amt);

        vm.prank(alice);
        esc.depositDot{value: 5 * 10 ** 10}();

        vm.prank(alice);
        esc.openSession(hybrid, SecurityTier.BEST_EFFORT, amt);

        vm.prank(attestation);
        reg.setTEEProof(hybrid, bytes32(uint256(0xcee)));

        vm.prank(alice);
        esc.openSession(hybrid, SecurityTier.TEE_VERIFIED, amt);
    }

    /// @notice `registerProvider` always stores `supportsTEE=false` until the attestation service calls `setTEEProof`.
    function test_requestedTeeDoesNotExposeTierUntilProof() public {
        address newbie = address(0xC001);
        vm.prank(newbie);
        reg.registerProvider(payout, true, true, "");

        ProviderInfo memory p = reg.getProvider(newbie);
        assertFalse(p.supportsTEE);

        assertTrue(reg.supportsTier(newbie, SecurityTier.BEST_EFFORT));
        assertFalse(reg.supportsTier(newbie, SecurityTier.TEE_VERIFIED));

        vm.prank(attestation);
        reg.setTEEProof(newbie, bytes32(uint256(0x01)));
        assertTrue(reg.supportsTier(newbie, SecurityTier.TEE_VERIFIED));
    }

    function test_sessionLifecycle_partialThenFull_thenWithdrawProvider() public {
        uint256 lockAmt = 10 * 10 ** 18;

        vm.prank(alice);
        esc.depositDot{value: 10 * 10 ** 10}();

        vm.prank(alice);
        esc.openSession(providerA, SecurityTier.TEE_VERIFIED, lockAmt);

        assertEq(esc.totalLockedInternal(), lockAmt);

        vm.prank(providerA);
        esc.recordUsage(0, 1 * 10 ** 18);

        vm.prank(providerA);
        esc.recordUsage(0, 2 * 10 ** 18);
        (,,, uint256 lockedAfterRecord,,,,) = esc.sessions(0);
        assertEq(lockedAfterRecord, lockAmt);
        (,,,, uint256 usage,,,) = esc.sessions(0);
        assertEq(usage, 3 * 10 ** 18);

        vm.prank(alice);
        esc.settlePartial(0, 4 * 10 ** 18, 1 * 10 ** 18);
        assertEq(esc.providerBalances(providerA), 4 * 10 ** 18);
        assertEq(esc.dotBalances(alice), 1 * 10 ** 18);
        assertEq(esc.totalLockedInternal(), 5 * 10 ** 18);
        (,,, uint256 locked2,,,, bool settledMid) = esc.sessions(0);
        assertEq(locked2, 5 * 10 ** 18);
        assertFalse(settledMid);

        vm.prank(alice);
        esc.settleFull(0, 5 * 10 ** 18, 0);
        assertEq(esc.totalLockedInternal(), 0);

        (,,, uint256 locked3,,,, bool settledFinal) = esc.sessions(0);
        assertEq(locked3, 0);
        assertTrue(settledFinal);

        assertEq(esc.providerBalances(providerA), 9 * 10 ** 18);
        vm.deal(address(esc), address(esc).balance + 100 ether);

        vm.prank(payout);
        vm.expectRevert(SettlementEscrow.InsufficientBalance.selector);
        esc.withdrawProviderDot(9 * 10 ** 18);

        uint256 providerNativeBefore = providerA.balance;

        vm.prank(providerA);
        esc.withdrawProviderDot(9 * 10 ** 18);

        assertGt(providerA.balance, providerNativeBefore);
        assertEq(esc.providerBalances(providerA), 0);
        assertEq(esc.internalCirculating(), 1 * 10 ** 18);
    }

    function test_recordUsageAndSettleRevertAfterSettled() public {
        vm.prank(alice);
        esc.depositDot{value: 10 ** 11}(); // 10 dot internal

        vm.prank(alice);
        esc.openSession(providerA, SecurityTier.TEE_VERIFIED, 10 ** 18);

        vm.prank(alice);
        esc.settleFull(0, 0, 10 ** 18);

        vm.prank(providerA);
        vm.expectRevert(SettlementEscrow.AlreadySettled.selector);
        esc.recordUsage(0, 1);

        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.AlreadySettled.selector);
        esc.settlePartial(0, 0, 0);
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

    function test_settle_partial_revert_notUser() public {
        vm.prank(alice);
        esc.depositDot{value: 10 ** 11}();

        vm.prank(alice);
        esc.openSession(providerA, SecurityTier.TEE_VERIFIED, 10 ** 18);

        vm.prank(providerA);
        vm.expectRevert(SettlementEscrow.NotSessionUser.selector);
        esc.settlePartial(0, 0, 0);
    }

    function test_bestEffortVsTee_providersStayPartitionedOnRegistry() public {
        address bee = address(0xBEE1);
        vm.prank(bee);
        reg.registerProvider(payout, true, false, "");
        vm.prank(attestation);
        reg.setTEEProof(bee, bytes32(uint256(0xbaa)));

        address teeExclusive = address(0xABC2);
        vm.prank(teeExclusive);
        reg.registerProvider(payout, false, true, "");

        vm.prank(attestation);
        reg.setTEEProof(teeExclusive, bytes32(uint256(0xabc)));

        assertTrue(reg.supportsTier(bee, SecurityTier.BEST_EFFORT));
        assertTrue(reg.supportsTier(bee, SecurityTier.TEE_VERIFIED));
        assertFalse(reg.supportsTier(teeExclusive, SecurityTier.BEST_EFFORT));
        assertTrue(reg.supportsTier(teeExclusive, SecurityTier.TEE_VERIFIED));

        vm.prank(alice);
        esc.depositDot{value: 10 ** 14}(); // abundant internal DOT

        vm.prank(alice);
        esc.openSession(bee, SecurityTier.BEST_EFFORT, 10 ** 18);

        vm.prank(alice);
        vm.expectRevert(SettlementEscrow.UnsupportedTier.selector);
        esc.openSession(teeExclusive, SecurityTier.BEST_EFFORT, 10 ** 18);

        vm.prank(alice);
        esc.openSession(teeExclusive, SecurityTier.TEE_VERIFIED, 10 ** 18);
    }
}
