// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {ProviderRegistry} from "../src/ProviderRegistry.sol";
import {SecurityTier, ProviderInfo} from "../src/SecurityTypes.sol";

contract ProviderRegistryTest is Test {
    ProviderRegistry internal reg;

    address internal owner = address(0xAce0);
    address internal attestation = address(0xA777);
    address internal provider = address(0xB00B);
    address internal payout = address(0xCAFE);

    function setUp() public {
        vm.prank(owner);
        reg = new ProviderRegistry(owner, attestation);
    }

    function test_registerProvider_setPricing_metadata() public {
        vm.startPrank(provider);
        reg.registerProvider(payout, true, true, "ipfs://meta");
        vm.stopPrank();

        ProviderInfo memory p = reg.getProvider(provider);
        assertEq(p.payout, payout);
        assertTrue(p.active);
        assertTrue(p.supportsBestEffort);
        assertFalse(p.supportsTEE);
        assertEq(p.teeReportHash, bytes32(0));
        assertEq(keccak256(bytes(reg.getMetadataURI(provider))), keccak256(bytes("ipfs://meta")));

        vm.prank(provider);
        reg.setPricing(SecurityTier.BEST_EFFORT, 123);
        assertEq(reg.getPricePer1k(provider, SecurityTier.BEST_EFFORT), 123);

        vm.prank(provider);
        reg.setPricing(SecurityTier.TEE_VERIFIED, 456);
        assertEq(reg.getPricePer1k(provider, SecurityTier.TEE_VERIFIED), 456);
    }

    function test_supportsTier_bestEffort_inactive() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, false, "");
        assertTrue(reg.supportsTier(provider, SecurityTier.BEST_EFFORT));

        vm.prank(provider);
        reg.setProviderActive(false);
        assertFalse(reg.supportsTier(provider, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_bestEffort_falseWhenNotSupported() public {
        vm.prank(provider);
        reg.registerProvider(payout, false, false, "");
        assertFalse(reg.supportsTier(provider, SecurityTier.BEST_EFFORT));
    }

    function test_supportsTier_teeOnlyAfterProof() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, true, "");
        assertFalse(reg.supportsTier(provider, SecurityTier.TEE_VERIFIED));

        vm.prank(attestation);
        reg.setTEEProof(provider, bytes32(uint256(1)));
        assertTrue(reg.supportsTier(provider, SecurityTier.TEE_VERIFIED));

        ProviderInfo memory p = reg.getProvider(provider);
        assertTrue(p.supportsTEE);
        assertEq(p.teeReportHash, bytes32(uint256(1)));
    }

    function test_setTEEProof_revert_notAttestation() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, false, "");

        vm.prank(address(0xDEAD));
        vm.expectRevert(ProviderRegistry.NotAttestationService.selector);
        reg.setTEEProof(provider, bytes32(uint256(1)));
    }

    function test_setTEEProof_revert_zeroHash() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, false, "");

        vm.prank(attestation);
        vm.expectRevert(ProviderRegistry.InvalidTEEProof.selector);
        reg.setTEEProof(provider, bytes32(0));
    }

    function test_setTEEProof_revert_unregistered() public {
        vm.prank(attestation);
        vm.expectRevert(ProviderRegistry.ProviderNotRegistered.selector);
        reg.setTEEProof(provider, bytes32(uint256(1)));
    }

    function test_setProviderPayout_and_fee() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, false, "");

        address p2 = address(0x1111);
        vm.prank(provider);
        reg.setProviderPayout(p2);
        assertEq(reg.getProvider(provider).payout, p2);

        vm.prank(owner);
        reg.setProviderFee(provider, 100);
        assertEq(reg.getProvider(provider).feeBps, 100);
    }

    function test_setProviderFee_revert_notOwner() public {
        vm.prank(provider);
        reg.registerProvider(payout, true, false, "");

        vm.prank(provider);
        vm.expectRevert(ProviderRegistry.NotOwner.selector);
        reg.setProviderFee(provider, 1);
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
