// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SecurityTier, NodeInfo} from "../src/SecurityTypes.sol";

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
        reg.registerNode(nodeId, payout, true, true, "ipfs://meta");
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
        reg.registerNode(nodeId, address(0), true, false, "");
        assertEq(reg.getProvider(nodeId).payout, operator);
    }

    function test_registerNode_revert_zeroId() public {
        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.ZeroNodeId.selector);
        reg.registerNode(bytes32(0), payout, true, false, "");
    }

    function test_supportsTier_bestEffort_inactive() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "");
        assertTrue(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));

        vm.prank(operator);
        reg.setNodeActive(nodeId, false);
        assertFalse(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_bestEffort_falseWhenNotSupported() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, false, false, "");
        assertFalse(reg.supportsTier(nodeId, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_teeOnlyNode_rejectsBestEffort() public {
        bytes32 teeOnly = _nid(address(0xC0DE2));
        vm.prank(operator);
        reg.registerNode(teeOnly, payout, false, true, "");
        vm.prank(attestation);
        reg.setTEEProof(teeOnly, bytes32(uint256(0x99)));

        assertFalse(reg.supportsTier(teeOnly, SecurityTier.BEST_EFFORT));
        assertTrue(reg.supportsTier(teeOnly, SecurityTier.TEE_VERIFIED));
    }

    function test_supportsTier_teeDeclaredButNeedsProofForTier() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, true, "");
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
        reg.registerNode(nodeId, payout, true, false, "");

        vm.prank(address(0xDEAD));
        vm.expectRevert(ProviderRegistry.NotAttestationService.selector);
        reg.setTEEProof(nodeId, bytes32(uint256(1)));
    }

    function test_setTEEProof_revert_zeroHash() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "");

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
        reg.registerNode(nodeId, payout, true, false, "");

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
        reg.registerNode(nodeId, payout, true, false, "");

        vm.prank(operator);
        vm.expectRevert(ProviderRegistry.NotOwner.selector);
        reg.setNodeFee(nodeId, 1);
    }

    function test_onlyNodeOperator_revert() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "");

        vm.prank(address(0xBAD));
        vm.expectRevert(ProviderRegistry.NotNodeOperator.selector);
        reg.setNodeActive(nodeId, false);
    }

    function test_registerNode_revert_twice() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "");
        vm.expectRevert(ProviderRegistry.NodeAlreadyRegistered.selector);
        reg.registerNode(nodeId, payout, true, false, "");
        vm.stopPrank();
    }

    function test_deregisterNode_clears_state_and_operator_list() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.startPrank(operator);
        reg.registerNode(nodeId, payout, true, false, "");
        reg.setNodePricing(nodeId, SecurityTier.BEST_EFFORT, 99);
        reg.deregisterNode(nodeId);
        vm.stopPrank();

        assertEq(reg.nodeOperator(nodeId), address(0));
        assertEq(reg.getPricePer1k(nodeId, SecurityTier.BEST_EFFORT), 0);
        assertEq(reg.operatorNodes(operator).length, 0);
        NodeInfo memory n = reg.getProvider(nodeId);
        assertEq(n.payout, address(0));

        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "meta2");
        assertEq(reg.nodeOperator(nodeId), operator);
    }

    function test_deregisterNode_revert_notOperator() public {
        bytes32 nodeId = _nid(nodeAddr);
        vm.prank(operator);
        reg.registerNode(nodeId, payout, true, false, "");

        vm.prank(address(0xBAD));
        vm.expectRevert(ProviderRegistry.NotNodeOperator.selector);
        reg.deregisterNode(nodeId);
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
}
