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
    mapping(address => uint256) public providerBalances;

    /// @notice Sum of `Session.lockedInternal` across non-settled sessions (and settled sessions hold `lockedInternal == 0`).
    uint256 public totalLockedInternal;

    /// @notice Total internal‑DOT liabilities: free user balances, locked funds, and provider balances.
    /// @dev Decremented only when internal DOT leaves the model via `withdrawDot` / `withdrawProviderDot` (converted to native).
    uint256 public internalCirculating;

    struct Session {
        address user;
        address provider;
        SecurityTier tier;
        uint256 lockedInternal;
        uint256 usageRecorded;
        uint256 openingInternal;
        uint64 openedAt;
        bool settled;
    }

    uint256 public nextSessionId;
    mapping(uint256 => Session) public sessions;

    event DotDeposited(address indexed user, uint256 amountNative, uint256 creditedInternal);
    event DotWithdrawn(address indexed user, uint256 burnedInternal, uint256 paidNative);
    event ProviderDotWithdrawn(address indexed provider, uint256 burnedInternal, uint256 paidNative);
    event UsdcDepositedAsDot(address indexed user, uint256 usdcAmount, uint256 creditedInternal);
    event SessionOpened(
        uint256 indexed sessionId,
        address indexed user,
        address indexed provider,
        SecurityTier tier,
        uint256 lockedInternal
    );
    event UsageRecorded(uint256 indexed sessionId, uint256 usageTotalInternal);
    event SessionFundsReleased(
        uint256 indexed sessionId, uint256 toProvider, uint256 toUser, uint256 remainingLockedInternal
    );

    error UnsupportedTier();
    error BadAmount();
    error TransferFailed();
    error InsufficientBalance();
    error UnknownSession();
    error AlreadySettled();
    error NotSessionUser();
    error NotSessionProvider();
    error BadSettleSplit();
    error OracleStale();
    error Slippage();

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
        internalCirculating += credited;
        emit DotDeposited(msg.sender, msg.value, credited);
    }

    /// @notice Withdraw internal balance back to native DOT.
    function withdrawDot(uint256 amountInternal) external {
        if (amountInternal == 0) revert BadAmount();
        if (dotBalances[msg.sender] < amountInternal) revert InsufficientBalance();
        dotBalances[msg.sender] -= amountInternal;
        internalCirculating -= amountInternal;
        uint256 native = _internalToNative(amountInternal);
        (bool ok,) = msg.sender.call{value: native}("");
        if (!ok) revert TransferFailed();
        emit DotWithdrawn(msg.sender, amountInternal, native);
    }

    /// @notice Pull USDC using current oracle rates (no staleness bound, zero min dot out).
    function depositUsdcAsDot(uint256 usdcAmount) external {
        _depositUsdcAsDot(usdcAmount, 0, type(uint256).max);
    }

    /// @notice Pull USDC with optional slippage (`minDotInternalOut`) and oracle freshness (`maxOracleAgeSecs`, use `type(uint256).max` to skip).
    function depositUsdcAsDot(uint256 usdcAmount, uint256 minDotInternalOut, uint256 maxOracleAgeSecs) external {
        _depositUsdcAsDot(usdcAmount, minDotInternalOut, maxOracleAgeSecs);
    }

    function _depositUsdcAsDot(uint256 usdcAmount, uint256 minDotInternalOut, uint256 maxOracleAgeSecs) internal {
        if (usdcAmount == 0) revert BadAmount();
        if (maxOracleAgeSecs != type(uint256).max) {
            uint256 pu = priceOracle.priceUpdatedAt();
            if (pu == 0 || block.timestamp > pu + maxOracleAgeSecs) revert OracleStale();
        }

        uint256 usdcPerDot = priceOracle.getUsdcPerDot();
        if (usdcPerDot == 0) revert BadAmount();
        uint256 credited = (usdcAmount * 1e18) / usdcPerDot;
        if (credited == 0) revert BadAmount();
        if (minDotInternalOut != 0 && credited < minDotInternalOut) revert Slippage();

        if (!usdc.transferFrom(msg.sender, address(this), usdcAmount)) revert TransferFailed();

        dotBalances[msg.sender] += credited;
        internalCirculating += credited;
        emit UsdcDepositedAsDot(msg.sender, usdcAmount, credited);
    }

    /// @notice Withdraw internal provider balance accrued from settles, paid as native DOT.
    function withdrawProviderDot(uint256 amountInternal) external {
        if (amountInternal == 0) revert BadAmount();
        if (providerBalances[msg.sender] < amountInternal) revert InsufficientBalance();
        providerBalances[msg.sender] -= amountInternal;
        internalCirculating -= amountInternal;
        uint256 native = _internalToNative(amountInternal);
        (bool ok,) = msg.sender.call{value: native}("");
        if (!ok) revert TransferFailed();
        emit ProviderDotWithdrawn(msg.sender, amountInternal, native);
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
            lockedInternal: amountInternal,
            usageRecorded: 0,
            openingInternal: amountInternal,
            openedAt: uint64(block.timestamp),
            settled: false
        });

        totalLockedInternal += amountInternal;

        if (msg.value == 0) {
            if (dotBalances[msg.sender] < amountInternal) revert InsufficientBalance();
            dotBalances[msg.sender] -= amountInternal;
        } else {
            uint256 nativeExpected = _internalToNative(amountInternal);
            if (msg.value != nativeExpected) revert BadAmount();
            internalCirculating += amountInternal;
        }

        emit SessionOpened(id, msg.sender, provider, tier, amountInternal);
    }

    /// @notice Provider records cumulative usage toward off-chain metering (does not move funds).
    function recordUsage(uint256 sessionId, uint256 usageDeltaInternal) external {
        Session storage s = sessions[sessionId];
        if (s.user == address(0)) revert UnknownSession();
        if (s.provider != msg.sender) revert NotSessionProvider();
        if (s.settled) revert AlreadySettled();
        if (usageDeltaInternal == 0) revert BadAmount();
        s.usageRecorded += usageDeltaInternal;
        emit UsageRecorded(sessionId, s.usageRecorded);
    }

    /// @notice Releases up to remaining lock: pays provider escrow credit and refunds user balance.
    function settlePartial(uint256 sessionId, uint256 toProvider, uint256 toUser) external {
        _releaseSessionFunds(sessionId, toProvider, toUser, false);
    }

    /// @notice Like `settlePartial` but requires paying out the entire remaining lock.
    function settleFull(uint256 sessionId, uint256 toProvider, uint256 toUser) external {
        _releaseSessionFunds(sessionId, toProvider, toUser, true);
    }

    function _releaseSessionFunds(uint256 sessionId, uint256 toProvider, uint256 toUser, bool mustDrain) internal {
        Session storage s = sessions[sessionId];
        if (s.user == address(0)) revert UnknownSession();
        if (s.user != msg.sender) revert NotSessionUser();
        if (s.settled) revert AlreadySettled();
        uint256 out = toProvider + toUser;
        if (mustDrain) {
            if (out != s.lockedInternal) revert BadSettleSplit();
        } else if (out == 0 || out > s.lockedInternal) {
            revert BadSettleSplit();
        }

        s.lockedInternal -= out;
        totalLockedInternal -= out;
        providerBalances[s.provider] += toProvider;
        dotBalances[s.user] += toUser;

        if (s.lockedInternal == 0) s.settled = true;

        emit SessionFundsReleased(sessionId, toProvider, toUser, s.lockedInternal);
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
