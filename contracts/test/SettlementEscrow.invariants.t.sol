// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {StdInvariant} from "forge-std/StdInvariant.sol";
import {Test} from "forge-std/Test.sol";

import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockERC20} from "../src/mocks/MockERC20.sol";
import {SecurityTier} from "../src/SecurityTypes.sol";

/// @notice Stateful wrapper for `SettlementEscrow` used as the invariant fuzz target.
contract SettlementEscrowInvariantHandler is Test {
    SettlementEscrow public escrow;
    ProviderRegistry public reg;
    MockOracle public oracle;
    MockERC20 public usdc;

    address public alice;
    address public provider0;
    address public provider1;
    address public payoutAddr = address(0xFAC);

    mapping(address => uint256) internal _openedLockSum;
    mapping(address => uint256) internal _paidToProviderSum;

    constructor(
        SettlementEscrow esc_,
        ProviderRegistry reg_,
        MockOracle oracle_,
        MockERC20 usdc_,
        address alice_,
        address p0,
        address p1,
        address attestation_
    ) {
        escrow = esc_;
        reg = reg_;
        oracle = oracle_;
        usdc = usdc_;
        alice = alice_;
        provider0 = p0;
        provider1 = p1;

        vm.prank(p0);
        reg.registerProvider(payoutAddr, true, true, "");
        vm.prank(attestation_);
        reg.setTEEProof(p0, bytes32(uint256(0xA11CE)));

        vm.prank(p1);
        reg.registerProvider(payoutAddr, true, true, "");
        vm.prank(attestation_);
        reg.setTEEProof(p1, bytes32(uint256(0xB22CE)));

        vm.deal(alice_, 1_000_000 ether);
        vm.deal(address(escrow), 1_000_000 ether);
    }

    function openedLockTotal(address p) external view returns (uint256) {
        return _openedLockSum[p];
    }

    function paidToProviderTotal(address p) external view returns (uint256) {
        return _paidToProviderSum[p];
    }

    function _ensureAliceSpendable(uint256 needInternal) internal {
        uint256 bal = escrow.dotBalances(alice);
        if (bal >= needInternal) return;
        uint256 nativeTopUp = (needInternal / (10 ** 8)) + 100 * (10 ** 10);
        nativeTopUp = bound(nativeTopUp, 10 ** 10, 10 ** 25);
        vm.deal(alice, alice.balance + nativeTopUp);
        vm.prank(alice);
        escrow.depositDot{value: nativeTopUp}();
    }

    function step_openSession(uint256 lockAmt, bool teeTier, bool pickProvider1) external {
        address p = pickProvider1 ? provider1 : provider0;
        SecurityTier t = teeTier ? SecurityTier.TEE_VERIFIED : SecurityTier.BEST_EFFORT;
        lockAmt = bound(lockAmt, 1, 500 * (10 ** 18));

        _ensureAliceSpendable(lockAmt);
        vm.prank(alice);
        try escrow.openSession(p, t, lockAmt) {
            _openedLockSum[p] += lockAmt;
        } catch {}
    }

    function step_recordUsage(uint256 seed, uint256 usageAmt) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = bound(seed, 0, n - 1);
        (,,,,,,, bool settled) = escrow.sessions(sid);
        if (settled) return;

        (, address providerAddr,,,,,,) = escrow.sessions(sid);
        usageAmt = bound(usageAmt, 1, (10 ** 18));

        vm.prank(providerAddr);
        try escrow.recordUsage(sid, usageAmt) {} catch {}
    }

    function step_settlePartial(uint256 seed, uint256 a, uint256 b) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = seed % n;
        (address user, address providerAddr,, uint256 locked,,,, bool settled) = escrow.sessions(sid);
        if (settled || locked == 0 || user != alice) return;

        uint256 mix = uint256(keccak256(abi.encode(seed, a, b)));
        uint256 sum = bound(mix, 1, locked);
        uint256 toP = bound(a, 0, sum);
        uint256 toU = sum - toP;

        vm.prank(alice);
        try escrow.settlePartial(sid, toP, toU) {
            _paidToProviderSum[providerAddr] += toP;
        } catch {}
    }

    function step_settleFullRemainder(uint256 seed) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = seed % n;
        (address user, address p,, uint256 locked,,,, bool settled) = escrow.sessions(sid);
        if (settled || locked == 0 || user != alice) return;

        uint256 half = locked / 2;
        vm.prank(alice);
        try escrow.settleFull(sid, half, locked - half) {
            _paidToProviderSum[p] += half;
        } catch {}
    }

    function step_depositUsdc(uint128 usdcAmt) external {
        uint256 a = bound(uint256(usdcAmt), 1, 250_000 * 1_000_000);
        usdc.mint(alice, a);
        vm.startPrank(alice);
        usdc.approve(address(escrow), a);
        try escrow.depositUsdcAsDot(a) {} catch {}
        vm.stopPrank();
    }

    function step_oracleBump(uint96 price) external {
        uint256 bounded = bound(uint256(price), 100_000, 10_000_000);
        oracle.set(bounded);
    }

    function step_withdrawAlice(uint256 amt) external {
        uint256 b = escrow.dotBalances(alice);
        if (b == 0) return;
        amt = bound(amt, 1, b);
        vm.prank(alice);
        try escrow.withdrawDot(amt) {} catch {}
    }

    function step_withdrawProviders(uint256 pick) external {
        address p = pick % 2 == 0 ? provider0 : provider1;
        uint256 pb = escrow.providerBalances(p);
        if (pb == 0) return;
        vm.deal(address(escrow), address(escrow).balance + 1 ether);
        vm.prank(p);
        try escrow.withdrawProviderDot(pb) {} catch {}
    }
}

contract SettlementEscrowInvariantTest is StdInvariant, Test {
    ProviderRegistry internal reg;
    MockOracle internal oracle;
    MockERC20 internal usdc;
    SettlementEscrow internal esc;
    SettlementEscrowInvariantHandler internal handler;

    address internal owner = address(0xC0AE);
    address internal attestation = address(0xA770);
    address internal alice = address(0xA717E);
    address internal p0 = address(0xF100);
    address internal p1 = address(0xF200);

    function setUp() public {
        vm.prank(owner);
        reg = new ProviderRegistry(owner, attestation);

        oracle = new MockOracle();
        oracle.set(1_340_000);

        usdc = new MockERC20("USDC", 6);
        esc = new SettlementEscrow(reg, oracle, usdc);
        handler =
            new SettlementEscrowInvariantHandler(esc, reg, oracle, usdc, alice, p0, p1, attestation);

        targetContract(address(handler));

        excludeContract(owner);
        excludeContract(attestation);
    }

    function invariant_circulatingMatchesBucketsSingleAlice() external view {
        uint256 buckets = esc.totalLockedInternal() + esc.dotBalances(alice);
        buckets += esc.providerBalances(handler.provider0()) + esc.providerBalances(handler.provider1());
        assertEq(buckets, esc.internalCirculating());
    }

    function invariant_providerPaidDoesNotExceedOpenedGhost() external view {
        address p0_ = handler.provider0();
        address p1_ = handler.provider1();
        assertLe(handler.paidToProviderTotal(p0_), handler.openedLockTotal(p0_));
        assertLe(handler.paidToProviderTotal(p1_), handler.openedLockTotal(p1_));
    }
}
