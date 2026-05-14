// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {SparklNetworkConfig} from "../src/SparklNetworkConfig.sol";

contract SparklNetworkConfigTest is Test {
    bytes32 internal constant NETWORK_CONFIG_SALT = keccak256("sparkl.network.config.v1");

    address internal alice = address(0xA11ce);
    address internal bob = address(0xB0B);
    address internal reg = address(0xBEEF);
    address internal escrow = address(0xE5C0);
    address internal oracle = address(0x0A51);

    function test_setAddresses_revert_NotOwner() public {
        vm.prank(alice);
        SparklNetworkConfig cfg = new SparklNetworkConfig(alice);

        vm.prank(bob);
        vm.expectRevert(SparklNetworkConfig.NotOwner.selector);
        cfg.setAddresses(reg, escrow, oracle);
    }

    function test_setAddresses_version_and_getters() public {
        vm.prank(alice);
        SparklNetworkConfig cfg = new SparklNetworkConfig(alice);

        assertEq(cfg.version(), 0);
        assertEq(cfg.providerRegistry(), address(0));
        assertEq(cfg.settlementEscrow(), address(0));
        assertEq(cfg.priceOracle(), address(0));

        vm.prank(alice);
        vm.expectEmit(true, true, true, true);
        emit SparklNetworkConfig.AddressesUpdated(reg, escrow, oracle, 1);
        cfg.setAddresses(reg, escrow, oracle);

        assertEq(cfg.version(), 1);
        assertEq(cfg.providerRegistry(), reg);
        assertEq(cfg.settlementEscrow(), escrow);
        assertEq(cfg.priceOracle(), oracle);

        vm.prank(alice);
        cfg.setAddresses(reg, escrow, oracle);
        assertEq(cfg.version(), 2);
    }

    function test_transferOwnership() public {
        vm.prank(alice);
        SparklNetworkConfig cfg = new SparklNetworkConfig(alice);
        assertEq(cfg.owner(), alice);

        vm.prank(alice);
        vm.expectEmit(true, true, true, true);
        emit SparklNetworkConfig.OwnershipTransferred(alice, bob);
        cfg.transferOwnership(bob);

        assertEq(cfg.owner(), bob);

        vm.prank(alice);
        vm.expectRevert(SparklNetworkConfig.NotOwner.selector);
        cfg.setAddresses(reg, escrow, oracle);

        vm.prank(bob);
        cfg.setAddresses(reg, escrow, oracle);
        assertEq(cfg.version(), 1);
    }

    function test_create2_predicted_matches_deployed() public {
        uint256 pk = 0xABCD;
        address deployer = vm.addr(pk);
        vm.deal(deployer, 1 ether);

        bytes32 initCodeHash = keccak256(
            abi.encodePacked(type(SparklNetworkConfig).creationCode, abi.encode(deployer))
        );

        vm.startPrank(deployer);
        address predicted = vm.computeCreate2Address(NETWORK_CONFIG_SALT, initCodeHash, deployer);
        SparklNetworkConfig cfg = new SparklNetworkConfig{salt: NETWORK_CONFIG_SALT}(deployer);
        vm.stopPrank();

        assertEq(address(cfg), predicted, "CREATE2 address mismatch");
    }
}
