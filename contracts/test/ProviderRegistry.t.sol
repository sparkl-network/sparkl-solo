// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SecurityTier, NodeInfo, NodeLifecycle} from "../src/SecurityTypes.sol";

/// @dev Implements `openSessionCountByNode` for registry `markDefunct` tests.
contract MockEscrowOpenCounts {
    mapping(bytes32 => uint256) internal _counts;

    function openSessionCountByNode(bytes32 nodeId) external view returns (uint256) {
        return _counts[nodeId];
    }

    function setOpenCount(bytes32 nodeId, uint256 v) external {
        _counts[nodeId] = v;
    }
}

contract ProviderRegistryTest is Test {
    ProviderRegistry internal reg;

    address internal owner = address(0xAce0);
    address internal attestation = address(0xA777);
    address internal operator = address(0xB00B);
    /// @dev Address used only to derive a test `bytes32` node id.
    address internal nodeAddr = address(0xC0DE);
    address internal payout = address(0xCAFE);

    function _nid(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }

    function setUp() public {
        vm.prank(owner);
        reg = new ProviderRegistry(owner, attestation);
    }

    function test_registerNode_setNodePricing_metadata() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, true, "ipfs://meta", bytes32(0));
        vm.stopPrank();

        assertEq(reg.nodeOperator(nodeId), operator);
        bytes32[] memory on = reg.operatorNodes(operator);
        assertEq(on.length, 1);
        assertEq(on[0], nodeId);

        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.payout, payout);
        assertTrue(n.active);
        assertTrue(n.supportsBestEffort);
        assertTrue(n.supportsTEE);
        assertEq(n.teeReportHash, bytes32(0));
        assertEq(keccak256(bytes(n.metadataURI)), keccak256(bytes("ipfs://meta")));
        assertEq(keccak256(bytes(reg.getMetadataURI(nodeId))), keccak256(bytes("ipfs://meta")));
        assertEq(uint8(reg.getProvider(nodeId).lifecycle), uint8(NodeLifecycle.Active));

        vm.prank(operator);
        reg.setNodePricing(nodeId, SecurityTier.BEST_EFFORT, 123);
        assertEq(reg.getPricePer1k(nodeId, SecurityTier.BEST_EFFORT), 123);

        vm.prank(operator);
        reg.setNodePricing(nodeId, SecurityTier.TEE_VERIFIED, 456);
        assertEq(reg.getPricePer1k(nodeId, SecurityTier.TEE_VERIFIED), 456);
    }

    function test_registerNode_zeroPayout_defaultsToOperator() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, address(0), true, false, "", bytes32(0));
        assertEq(reg.getProvider(nodeId).payout, operator);
    }

    function test_registerNode_revert_zeroId() public {
        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.ZeroNodeId.selector);
        reg.registerNode(bytes32(0), payout, true, false, "", bytes32(0));
    }

    function test_supportsTier_bestEffort_inactive() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        assertTrue(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));

        vm.prank(operator);
        reg.setNodeActive(nodeId, false);
        assertFalse(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_bestEffort_falseWhenNotSupported() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, false, false, "", bytes32(0));
        assertFalse(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_teeOnlyNode_rejectsBestEffort() public {
        bytes32 teeOnly = _nid(address(0xC0DE2));
        vm.prank(operator);
        reg.registerNode(teeOnly, payout, false, true, "", bytes32(0));
        vm.prank(attestation);
        reg.setTEEProof(teeOnly, bytes32(uint256(0x99)));

        assertFalse(reg.supportsTier(teeOnly, SecurityTier.BEST_EFFORT));
        assertTrue(reg.supportsTier(teeOnly, SecurityTier.TEE_VERIFIED));
    }

    function test_supportsTier_teeDeclaredButNeedsProofForTier() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, true, "", bytes32(0));
        assertFalse(reg.supportsTier(nodeId, SecurityTier.TEE_VERIFIED));

        vm.prank(attestation);
        reg.setTEEProof(nodeId, bytes32(uint256(1)));
        assertTrue(reg.supportsTier(nodeId, SecurityTier.TEE_VERIFIED));

        NodeInfo memory n = reg.getProvider(nodeId);
        assertTrue(n.supportsTEE);
        assertEq(n.teeReportHash, bytes32(uint256(1)));
    }

    function test_setTEEProof_revert_notAttestation() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));

        vm.prank(address(0xDEAD));
        vm.expectRevert(ProviderRegistry.NotAttestationService.selector);
        reg.setTEEProof(nodeId, bytes32(uint256(1)));
    }

    function test_setTEEProof_revert_zeroHash() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));

        vm.prank(attestation);
        vm.expectRevert(ProviderRegistry.InvalidTEEProof.selector);
        reg.setTEEProof(nodeId, bytes32(0));
    }

    function test_setTEEProof_revert_unregistered() public {
        vm.prank(attestation);
        vm.expectRevert(ProviderRegistry.NodeNotRegistered.selector);
        reg.setTEEProof(_nid(nodeAddr), bytes32(uint256(1)));
    }

    function test_setNodePayout_and_fee() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));

        address p2 = address(0x1111);
        vm.prank(operator);
        reg.setNodePayout(nodeId, p2);
        assertEq(reg.getProvider(nodeId).payout, p2);

        vm.prank(owner);
        reg.setNodeFee(nodeId, 100);
        assertEq(reg.getProvider(nodeId).feeBps, 100);
    }

    function test_setNodeFee_revert_notOwner() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));

        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.NotOwner.selector);
        reg.setNodeFee(nodeId, 1);
    }

    function test_onlyNodeOperator_revert() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));

        vm.prank(address(0xBAD));
        vm.expectRevert(ProviderRegistry.NotNodeOperator.selector);
        reg.setNodeActive(nodeId, false);
    }

    function test_registerNode_revert_twice() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        vm.expectRevert(ProviderRegistry.NodeAlreadyRegistered.selector);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        vm.stopPrank();
    }

    function test_supportsTier_false_when_chilled() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        assertTrue(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));

        vm.prank(operator);
        reg.chillNode(nodeId);
        assertFalse(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));
        assertEq(uint8(reg.getProvider(nodeId).lifecycle), uint8(NodeLifecycle.Chilled));
        assertFalse(reg.getProvider(nodeId).active);
    }

    function test_chillNode_revert_notActive() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        reg.chillNode(nodeId);
        vm.expectRevert(ProviderRegistry.InvalidLifecycle.selector);
        reg.chillNode(nodeId);
        vm.stopPrank();
    }

    function test_markDefunct_revert_escrow_not_configured() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        reg.chillNode(nodeId);
        vm.expectRevert(ProviderRegistry.EscrowNotConfigured.selector);
        reg.markDefunct(nodeId);
        vm.stopPrank();
    }

    function test_markDefunct_revert_open_sessions_remain() public {
        MockEscrowOpenCounts mockEsc = new MockEscrowOpenCounts();
        vm.prank(owner);
        reg.setSettlementEscrow(address(mockEsc));

        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        reg.chillNode(nodeId);
        mockEsc.setOpenCount(nodeId, 1);
        vm.expectRevert(ProviderRegistry.OpenSessionsRemain.selector);
        reg.markDefunct(nodeId);
        vm.stopPrank();
    }

    function test_markDefunct_ok_then_purge_allows_reregister() public {
        MockEscrowOpenCounts mockEsc = new MockEscrowOpenCounts();
        vm.prank(owner);
        reg.setSettlementEscrow(address(mockEsc));

        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        reg.setNodePricing(nodeId, SecurityTier.BEST_EFFORT, 99);
        reg.chillNode(nodeId);
        mockEsc.setOpenCount(nodeId, 0);
        reg.markDefunct(nodeId);
        vm.stopPrank();

        assertEq(uint8(reg.getProvider(nodeId).lifecycle), uint8(NodeLifecycle.Defunct));
        assertEq(reg.nodeOperator(nodeId), operator);

        vm.prank(owner);
        reg.purgeDefunctNode(nodeId);

        assertEq(reg.nodeOperator(nodeId), address(0));
        assertEq(reg.getPricePer1k(nodeId, SecurityTier.BEST_EFFORT), 0);
        assertEq(reg.operatorNodes(operator).length, 0);
        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.payout, address(0));

        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "meta2", bytes32(0));
        assertEq(reg.nodeOperator(nodeId), operator);
    }

    function test_purgeDefunctNode_revert_notOwner() public {
        MockEscrowOpenCounts mockEsc = new MockEscrowOpenCounts();
        vm.prank(owner);
        reg.setSettlementEscrow(address(mockEsc));

        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        reg.chillNode(nodeId);
        reg.markDefunct(nodeId);
        vm.stopPrank();

        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.NotOwner.selector);
        reg.purgeDefunctNode(nodeId);
    }

    function test_transferOwnership_and_attestationService() public {
        address next = address(0xBEEF);
        vm.prank(owner);
        reg.transferOwnership(next);
        assertEq(reg.owner(), next);

        address nextAtt = address(0xBEAD);
        vm.prank(next);
        reg.setAttestationService(nextAtt);
        assertEq(reg.attestationService(), nextAtt);
    }

    function test_registerNode_initialEncryptionKey() public {
        bytes32 nodeId = _nid(nodeAddr);
        bytes32 pk = bytes32(uint256(0x112233));
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, true, "", pk);
        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.encryptionPubkey, pk);
        assertEq(n.encryptionKeyVersion, 1);
        assertEq(n.encryptionKeysLastVersion, 1);
        (bytes32 epk, uint64 act, uint64 dep, bool rev) = reg.encryptionKeys(nodeId, 1);
        assertEq(epk, pk);
        assertGt(act, 0);
        assertEq(dep, 0);
        assertFalse(rev);
    }

    function test_rotateEncryptionKey_deprecatesPrevious() public {
        bytes32 nodeId = _nid(nodeAddr);
        bytes32 pk1 = bytes32(uint256(1));
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, true, "", pk1);
        bytes32 pk2 = bytes32(uint256(2));
        reg.rotateEncryptionKey(nodeId, pk2, 3600);
        vm.stopPrank();

        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.encryptionKeyVersion, 2);
        assertEq(n.encryptionPubkey, pk2);
        (, , uint64 dep1, ) = reg.encryptionKeys(nodeId, 1);
        assertEq(dep1, uint64(block.timestamp + 3600));
        (bytes32 ep2, , , bool r2) = reg.encryptionKeys(nodeId, 2);
        assertEq(ep2, pk2);
        assertFalse(r2);
    }

    function test_revokeEncryptionKey_clearsHeadline() public {
        bytes32 nodeId = _nid(nodeAddr);
        bytes32 pk1 = bytes32(uint256(1));
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, true, "", pk1);
        reg.revokeEncryptionKey(nodeId, 1);
        vm.stopPrank();

        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.encryptionKeyVersion, 0);
        assertEq(n.encryptionPubkey, bytes32(0));
        (, , , bool r1) = reg.encryptionKeys(nodeId, 1);
        assertTrue(r1);

        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.EncryptionKeyAlreadyRevoked.selector);
        reg.revokeEncryptionKey(nodeId, 1);
    }

    function test_revokeEncryptionKey_revertsNotFound() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", bytes32(0));
        vm.expectRevert(ProviderRegistry.EncryptionKeyNotFound.selector);
        reg.revokeEncryptionKey(nodeId, 1);
        vm.stopPrank();
    }

    function test_rotateEncryptionKey_revert_zeroPubkey() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, true, "", bytes32(uint256(1)));
        vm.expectRevert(ProviderRegistry.ZeroEncryptionPubkey.selector);
        reg.rotateEncryptionKey(nodeId, bytes32(0), 0);
        vm.stopPrank();
    }

    function test_rotateEncryptionKey_whenNoInitialKey_createsVersion1() public {
        bytes32 nodeId = _nid(nodeAddr);
        bytes32 pk = bytes32(uint256(0xabc));
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, true, "", bytes32(0));
        NodeInfo memory n0 = reg.getProvider(nodeId);
        assertEq(n0.encryptionKeyVersion, 0);
        reg.rotateEncryptionKey(nodeId, pk, 60);
        vm.stopPrank();

        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.encryptionKeyVersion, 1);
        assertEq(n.encryptionPubkey, pk);
        assertEq(n.encryptionKeysLastVersion, 1);
        (bytes32 epk, uint64 act, uint64 dep, bool rev) = reg.encryptionKeys(nodeId, 1);
        assertEq(epk, pk);
        assertGt(act, 0);
        assertEq(dep, 0);
        assertFalse(rev);
    }

    function test_purgeDefunctNode_clearsEncryptionKeySlots() public {
        MockEscrowOpenCounts mockEsc = new MockEscrowOpenCounts();
        vm.prank(owner);
        reg.setSettlementEscrow(address(mockEsc));

        bytes32 nodeId = _nid(nodeAddr);
        bytes32 pk = bytes32(uint256(0x99));
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "", pk);
        reg.chillNode(nodeId);
        mockEsc.setOpenCount(nodeId, 0);
        reg.markDefunct(nodeId);
        vm.stopPrank();

        vm.prank(owner);
        reg.purgeDefunctNode(nodeId);

        (bytes32 epk, uint64 act,, ) = reg.encryptionKeys(nodeId, 1);
        assertEq(epk, bytes32(0));
        assertEq(act, 0);
    }
}
