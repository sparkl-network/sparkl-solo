// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";
import {IPriceOracle} from "./interfaces/IPriceOracle.sol";
import {IERC20} from "./interfaces/IERC20.sol";

/// @title SettlementEscrow
/// @notice Holds user balances in native DOT units (internal 1e18 fixed per whole DOT) and opens tier-aware sessions per provider.
/// @dev Accounting uses `INTERNAL_DOT_DECIMALS` (18) for all internal balances, matching `DIAPriceOracle` output scale.
///      Native transfers use `payable` with `msg.value`; wrapped DOT on some networks would use a different deposit path later.
contract SettlementEscrow {
    uint8 internal constant INTERNAL_DOT_DECIMALS = 18;

    IProviderRegistry public immutable registry;
    IPriceOracle public immutable priceOracle;
    IERC20 public immutable usdc;

    mapping(address => uint256) public dotBalances;

    struct Session {
        address user;
        address provider;
        SecurityTier tier;
        uint256 amountInternal;
        uint64 openedAt;
        bool settled;
    }

    uint256 public nextSessionId;
    mapping(uint256 => Session) public sessions;

    event DotDeposited(address indexed user, uint256 amountNative, uint256 creditedInternal);
    event DotWithdrawn(address indexed user, uint256 burnedInternal, uint256 paidNative);
    event UsdcDepositedAsDot(address indexed user, uint256 usdcAmount, uint256 creditedInternal);
    event SessionOpened(
        uint256 indexed sessionId, address indexed user, address indexed provider, SecurityTier tier, uint256 amountInternal
    );

    error UnsupportedTier();
    error BadAmount();
    error TransferFailed();
    error InsufficientBalance();
    error UnknownSession();
    error AlreadySettled();

    constructor(IProviderRegistry registry_, IPriceOracle priceOracle_, IERC20 usdc_) {
        registry = registry_;
        priceOracle = priceOracle_;
        usdc = usdc_;
    }

    /// @notice Accept native DOT and credit internal DOT-denominated balance.
    function depositDot() external payable {
        if (msg.value == 0) revert BadAmount();
        uint256 credited = _nativeToInternal(msg.value);
        dotBalances[msg.sender] += credited;
        emit DotDeposited(msg.sender, msg.value, credited);
    }

    /// @notice Withdraw internal balance back to native DOT.
    function withdrawDot(uint256 amountInternal) external {
        if (amountInternal == 0) revert BadAmount();
        if (dotBalances[msg.sender] < amountInternal) revert InsufficientBalance();
        dotBalances[msg.sender] -= amountInternal;
        uint256 native = _internalToNative(amountInternal);
        (bool ok,) = msg.sender.call{value: native}("");
        if (!ok) revert TransferFailed();
        emit DotWithdrawn(msg.sender, amountInternal, native);
    }

    /// @notice Pull USDC from `msg.sender` and credit internal DOT using `IPriceOracle.getUsdcPerDot()` (USDC per 1e18 internal DOT).
    function depositUsdcAsDot(uint256 usdcAmount) external {
        if (usdcAmount == 0) revert BadAmount();
        if (!usdc.transferFrom(msg.sender, address(this), usdcAmount)) revert TransferFailed();
        uint256 usdcPerDot = priceOracle.getUsdcPerDot();
        if (usdcPerDot == 0) revert BadAmount();
        uint256 credited = (usdcAmount * 1e18) / usdcPerDot;
        if (credited == 0) revert BadAmount();
        dotBalances[msg.sender] += credited;
        emit UsdcDepositedAsDot(msg.sender, usdcAmount, credited);
    }

    /// @notice Opens a tier-aware session, locking `amountInternal` for `(msg.sender, provider)`.
    /// @dev Pass `msg.value == 0` to consume from `dotBalances`, otherwise `msg.value` must equal `_internalToNative(amountInternal)`
    ///      and the escrow credits native into the lock without touching `dotBalances`.
    function openSession(address provider, SecurityTier tier, uint256 amountInternal) external payable {
        if (amountInternal == 0) revert BadAmount();
        if (!registry.supportsTier(provider, tier)) revert UnsupportedTier();

        uint256 id = nextSessionId++;
        sessions[id] = Session({
            user: msg.sender,
            provider: provider,
            tier: tier,
            amountInternal: amountInternal,
            openedAt: uint64(block.timestamp),
            settled: false
        });

        if (msg.value == 0) {
            if (dotBalances[msg.sender] < amountInternal) revert InsufficientBalance();
            dotBalances[msg.sender] -= amountInternal;
        } else {
            uint256 nativeExpected = _internalToNative(amountInternal);
            if (msg.value != nativeExpected) revert BadAmount();
        }

        emit SessionOpened(id, msg.sender, provider, tier, amountInternal);
    }

    function getDotBalances(address user) external view returns (uint256) {
        return dotBalances[user];
    }

    /// @notice Polkadot Asset Hub native DOT uses 10 decimals (Planck). Internal balances use 18 decimals per whole DOT.
    function _nativeDecimals() internal pure returns (uint8) {
        return 10;
    }

    function _nativeToInternal(uint256 amountNative) internal pure returns (uint256) {
        if (INTERNAL_DOT_DECIMALS == _nativeDecimals()) return amountNative;
        if (INTERNAL_DOT_DECIMALS > _nativeDecimals()) {
            return amountNative * (10 ** (INTERNAL_DOT_DECIMALS - _nativeDecimals()));
        }
        return amountNative / (10 ** (_nativeDecimals() - INTERNAL_DOT_DECIMALS));
    }

    function _internalToNative(uint256 amountInternal) internal pure returns (uint256) {
        if (INTERNAL_DOT_DECIMALS == _nativeDecimals()) return amountInternal;
        if (INTERNAL_DOT_DECIMALS > _nativeDecimals()) {
            return amountInternal / (10 ** (INTERNAL_DOT_DECIMALS - _nativeDecimals()));
        }
        return amountInternal * (10 ** (_nativeDecimals() - INTERNAL_DOT_DECIMALS));
    }

    receive() external payable {}
}
