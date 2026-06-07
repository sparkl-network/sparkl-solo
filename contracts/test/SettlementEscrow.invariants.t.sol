// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {StdInvariant} from "forge-std/StdInvariant.sol";
import {Test} from "forge-std/Test.sol";

import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SettlementEscrow} from "../src/SettlementEscrow.sol";
import {MockOracle} from "../src/mocks/MockOracle.sol";
import {MockModelPriceOracle} from "../src/mocks/MockModelPriceOracle.sol";
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

    function _nid(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }

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
        reg.registerNode(_nid(p0), payoutAddr, true, true, "", bytes32(0));
        vm.prank(attestation_);
        reg.setTEEProof(_nid(p0), bytes32(uint256(0xA11CE)));

        vm.prank(p1);
        reg.registerNode(_nid(p1), payoutAddr, true, true, "", bytes32(0));
        vm.prank(attestation_);
        reg.setTEEProof(_nid(p1), bytes32(uint256(0xB22CE)));

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

    bytes32 internal constant TEST_MODEL = keccak256(abi.encodePacked("invariant-model"));

    function step_openSession(uint256 lockAmt, bool teeTier, bool pickProvider1) external {
        address p = pickProvider1 ? provider1 : provider0;
        SecurityTier t = teeTier ? SecurityTier.TEE_VERIFIED : SecurityTier.BEST_EFFORT;
        lockAmt = bound(lockAmt, 1, 500 * (10 ** 18));

        _ensureAliceSpendable(lockAmt);
        vm.prank(alice);
        try escrow.openSession(_nid(p), t, TEST_MODEL, lockAmt, "") {
            _openedLockSum[p] += lockAmt;
        } catch {}
    }

    function step_recordUsage(uint256 seed, uint256 usageAmt) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = bound(seed, 0, n - 1);
        SettlementEscrow.Session memory s = escrow.sessions(sid);
        if (s.settled) return;

        bytes32 nodeId = s.nodeId;
        uint256 inputTok = bound(usageAmt, 1, 1_000_000);

        vm.prank(reg.nodeOperator(nodeId));
        try escrow.recordUsage(sid, inputTok, 0) {} catch {}
    }

    function step_settlePartial(uint256 seed, uint256 a, uint256 b) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = seed % n;
        SettlementEscrow.Session memory sess = escrow.sessions(sid);
        if (sess.settled || sess.lockedInternal == 0 || sess.user != alice) return;
        bytes32 nodeId = sess.nodeId;
        uint256 locked = sess.lockedInternal;

        uint256 mix = uint256(keccak256(abi.encode(seed, a, b)));
        uint256 sum = bound(mix, 1, locked);
        uint256 toP = bound(a, 0, sum);
        uint256 toU = sum - toP;

        vm.prank(alice);
        try escrow.settlePartial(sid, toP, toU) {
            uint256 fee = (toP * escrow.protocolFeeBps()) / 10_000;
            _paidToProviderSum[reg.nodeOperator(nodeId)] += toP - fee;
        } catch {}
    }

    function step_settleFullRemainder(uint256 seed) external {
        uint256 n = escrow.nextSessionId();
        if (n == 0) return;
        uint256 sid = seed % n;
        SettlementEscrow.Session memory sess = escrow.sessions(sid);
        if (sess.settled || sess.lockedInternal == 0 || sess.user != alice) return;
        bytes32 nodeId = sess.nodeId;
        uint256 locked = sess.lockedInternal;

        uint256 half = locked / 2;
        vm.prank(alice);
        try escrow.settleFull(sid, half, locked - half) {
            uint256 fee = (half * escrow.protocolFeeBps()) / 10_000;
            _paidToProviderSum[reg.nodeOperator(nodeId)] += half - fee;
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
        bytes32 nid = _nid(p);
        uint256 pb = escrow.providerBalances(nid);
        if (pb == 0) return;
        vm.deal(address(escrow), address(escrow).balance + 1 ether);
        vm.prank(p);
        try escrow.withdrawProviderDot(nid, pb) {} catch {}
    }
}

contract SettlementEscrowInvariantTest is StdInvariant, Test {
    bytes32 internal constant TEST_MODEL = keccak256(abi.encodePacked("invariant-model"));

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

    function _nid(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }

    function setUp() public {
        vm.prank(owner);
        reg = new ProviderRegistry(owner, attestation);

        oracle = new MockOracle();
        oracle.set(1_340_000);

        usdc = new MockERC20("USDC", 6);

        MockModelPriceOracle modelOracle = new MockModelPriceOracle();
        modelOracle.setModel(TEST_MODEL, 1e15, 1e15);
        modelOracle.setDefault(1e15, 1e15);

        esc = new SettlementEscrow(reg, oracle, modelOracle, usdc, 10);
        vm.prank(owner);
        reg.setSettlementEscrow(address(esc));
        vm.prank(owner);
        esc.setTeePriceMultiplierBps(10_000);
        handler = new SettlementEscrowInvariantHandler(esc, reg, oracle, usdc, alice, p0, p1, attestation);

        targetContract(address(handler));

        excludeContract(owner);
        excludeContract(attestation);
    }

    function invariant_circulatingMatchesBucketsSingleAlice() external view {
        uint256 buckets = esc.totalLockedInternal() + esc.dotBalances(alice);
        buckets += esc.providerBalances(_nid(p0)) + esc.providerBalances(_nid(p1));
        buckets += esc.protocolBalances();
        assertEq(buckets, esc.internalCirculating());
    }

    function invariant_providerPaidDoesNotExceedOpenedGhost() external view {
        address p0_ = handler.provider0();
        address p1_ = handler.provider1();
        assertLe(handler.paidToProviderTotal(p0_), handler.openedLockTotal(p0_));
        assertLe(handler.paidToProviderTotal(p1_), handler.openedLockTotal(p1_));
    }
}
