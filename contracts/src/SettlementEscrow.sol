// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {SecurityTier} from "./SecurityTypes.sol";
import {IProviderRegistry} from "./interfaces/IProviderRegistry.sol";
import {IPriceOracle} from "./interfaces/IPriceOracle.sol";
import {IModelPriceOracle} from "./interfaces/IModelPriceOracle.sol";
import {IERC20} from "./interfaces/IERC20.sol";

/// @title SettlementEscrow
/// @notice Holds user balances in native DOT units (internal 1e18 fixed per whole DOT) and opens tier-aware sessions per provider.
/// @dev Accounting uses `INTERNAL_DOT_DECIMALS` (18) for all internal balances, matching `DIAPriceOracle` output scale.
///      Native transfers use `payable` with `msg.value`; wrapped DOT on some networks would use a different deposit path later.
contract SettlementEscrow {
    uint8 internal constant INTERNAL_DOT_DECIMALS = 18;

    IProviderRegistry public immutable registry;
    IPriceOracle public immutable priceOracle;
    IModelPriceOracle public immutable modelPriceOracle;
    IERC20 public immutable usdc;

    /// @notice Smallest-units per whole native token on this chain (Hub DOT: 10 = Planck; standard EVM dev: 18 = wei).
    uint8 public immutable nativeDotDecimals;

    /// @notice TEE tier billing multiplier in basis points (default 15_000 = 1.5x).
    uint256 public teePriceMultiplierBps = 15_000;

    uint256 internal constant BPS_DENOM = 10_000;
    uint256 internal constant MIN_TEE_MULTIPLIER_BPS = 10_000;
    uint256 internal constant MAX_TEE_MULTIPLIER_BPS = 30_000;

    /// @notice Privileged role that finalizes settlement splits (`settleByOperator*`). Set by registry governance (`registry.owner()`).
    address public settlementOperator;

    /// @notice Router / infrastructure EOA authorized to call `recordUsage` (metering role).
    address public recordUsageRole;

    /// @notice Treasury wallet for protocol fee accrual at settlement (internal DOT balance via `protocolBalances`).
    address public protocolTreasury;

    /// @notice Protocol fee on provider payout at settlement, in basis points (100 = 1%).
    uint256 public protocolFeeBps = 100;

    uint256 internal constant MAX_PROTOCOL_FEE_BPS = 1000;

    /// @notice Internal DOT accrued for the protocol treasury from settlement fees.
    uint256 public protocolBalances;

    mapping(address => uint256) public dotBalances;
    /// @notice Internal DOT credited to a node (`nodeId`) from session settles; withdrawn by the node operator.
    mapping(bytes32 => uint256) public providerBalances;

    /// @notice Sum of `Session.lockedInternal` across non-settled sessions (and settled sessions hold `lockedInternal == 0`).
    uint256 public totalLockedInternal;

    /// @notice Total internal‑DOT liabilities: free user balances, locked funds, and provider balances.
    /// @dev Decremented only when internal DOT leaves the model via `withdrawDot` / `withdrawProviderDot` (converted to native).
    uint256 public internalCirculating;

    struct Session {
        address user;
        /// @notice Registry node key (e.g. Substrate PeerId hash), not an EVM address.
        bytes32 nodeId;
        /// @notice keccak256(abi.encodePacked(modelName)) — billed via ModelPriceOracle.
        bytes32 modelId;
        SecurityTier tier;
        uint256 lockedInternal;
        uint256 usageRecorded;
        /// @notice Running total of gross internal DOT claimed toward `usageRecorded` via provider settles.
        uint256 paidToProviderInternal;
        /// @notice Running total of protocol fee slice settled from this session.
        uint256 paidToProtocolInternal;
        uint256 openingInternal;
        uint64 openedAt;
        bool settled;
        uint64 inputTokensRecorded;
        uint64 outputTokensRecorded;
        /// @notice ModelPriceOracle `inputPer1k` at `openSession` (internal DOT per 1k input tokens).
        uint256 inputPricePer1kAtOpen;
        /// @notice ModelPriceOracle `outputPer1k` at `openSession` (internal DOT per 1k output tokens).
        uint256 outputPricePer1kAtOpen;
        /// @notice `priceOracle.getUsdcPerDot()` at `openSession` (USDC 6-dec per 1e18 internal DOT).
        uint256 usdcPerDotAtOpen;
        /// @notice True when `getEffectivePrice` used the oracle default for this `modelId`.
        bool pricingUsedDefault;
        /// @notice Optional user label (max 128 bytes UTF-8).
        string name;
    }

    uint256 public nextSessionId;
    /// @notice Non-settled sessions per `nodeId`; decremented exactly once when `settled` becomes true.
    mapping(bytes32 nodeId => uint256) public openSessionCountByNode;
    mapping(uint256 => Session) internal sessionById;

    /// @dev Explicit getter avoids stack-too-deep in the compiler-generated public mapping getter.
    function sessions(uint256 sessionId) external view returns (Session memory) {
        return sessionById[sessionId];
    }

    event DotDeposited(address indexed user, uint256 amountNative, uint256 creditedInternal);
    event DotWithdrawn(address indexed user, uint256 burnedInternal, uint256 paidNative);
    event ProviderDotWithdrawn(address indexed provider, uint256 burnedInternal, uint256 paidNative);
    event UsdcDepositedAsDot(address indexed user, uint256 usdcAmount, uint256 creditedInternal);
    event SessionOpened(
        uint256 indexed sessionId,
        address indexed user,
        bytes32 indexed nodeId,
        SecurityTier tier,
        bytes32 modelId,
        uint256 lockedInternal,
        uint256 inputPricePer1kAtOpen,
        uint256 outputPricePer1kAtOpen,
        uint256 usdcPerDotAtOpen,
        bool pricingUsedDefault,
        string name
    );
    event UsageRecorded(
        uint256 indexed sessionId,
        uint256 inputTokensDelta,
        uint256 outputTokensDelta,
        uint256 usageTotalInternal
    );
    event TeePriceMultiplierUpdated(uint256 previousBps, uint256 nextBps);
    event SessionFundsReleased(
        uint256 indexed sessionId, uint256 toProvider, uint256 toUser, uint256 remainingLockedInternal
    );
    event SettlementOperatorUpdated(address indexed previous, address indexed next);
    event RecordUsageUpdated(address indexed previous, address indexed next);
    event ProtocolTreasuryUpdated(address indexed previous, address indexed next);
    event ProtocolFeeBpsUpdated(uint256 previousBps, uint256 nextBps);
    event ProtocolDotWithdrawn(address indexed treasury, uint256 burnedInternal, uint256 paidNative);
    event ProtocolFeeAccrued(
        uint256 indexed sessionId, uint256 grossToProvider, uint256 protocolFee, uint256 providerNet
    );

    error UnsupportedTier();
    error BadAmount();
    error BadNativeDecimals();
    error TransferFailed();
    error InsufficientBalance();
    error UnknownSession();
    error AlreadySettled();
    error NotSessionUser();
    error NotSessionProvider();
    error BadSettleSplit();
    error OracleStale();
    error Slippage();
    error NotSettlementOperator();
    error NotRegistryOwner();
    error OpenSessionCounterUnderflow();
    error BadTokenDelta();
    error BadTeeMultiplier();
    error BadSessionName();
    error NotRecordUsage();
    error NotProtocolTreasury();
    error BadProtocolFeeBps();

    modifier onlySettlementOperator() {
        if (msg.sender != settlementOperator) revert NotSettlementOperator();
        _;
    }

    constructor(
        IProviderRegistry registry_,
        IPriceOracle priceOracle_,
        IModelPriceOracle modelPriceOracle_,
        IERC20 usdc_,
        uint8 nativeDotDecimals_
    ) {
        registry = registry_;
        priceOracle = priceOracle_;
        modelPriceOracle = modelPriceOracle_;
        usdc = usdc_;
        if (nativeDotDecimals_ == 0 || nativeDotDecimals_ > INTERNAL_DOT_DECIMALS) {
            revert BadNativeDecimals();
        }
        nativeDotDecimals = nativeDotDecimals_;
    }

    /// @notice Registry owner sets the TEE tier price multiplier (10_000 = 1x, 15_000 = 1.5x).
    function setTeePriceMultiplierBps(uint256 bps) external {
        if (msg.sender != registry.owner()) revert NotRegistryOwner();
        if (bps < MIN_TEE_MULTIPLIER_BPS || bps > MAX_TEE_MULTIPLIER_BPS) revert BadTeeMultiplier();
        emit TeePriceMultiplierUpdated(teePriceMultiplierBps, bps);
        teePriceMultiplierBps = bps;
    }

    /// @notice Registry owner assigns the settlement operator (may be `address(0)` to disable operator settles).
    function setSettlementOperator(address next) external {
        if (msg.sender != registry.owner()) revert NotRegistryOwner();
        emit SettlementOperatorUpdated(settlementOperator, next);
        settlementOperator = next;
    }

    /// @notice Registry owner assigns the router metering EOA for `recordUsage`.
    function setRecordUsage(address next) external {
        if (msg.sender != registry.owner()) revert NotRegistryOwner();
        emit RecordUsageUpdated(recordUsageRole, next);
        recordUsageRole = next;
    }

    /// @notice Registry owner assigns the protocol treasury wallet.
    function setProtocolTreasury(address next) external {
        if (msg.sender != registry.owner()) revert NotRegistryOwner();
        emit ProtocolTreasuryUpdated(protocolTreasury, next);
        protocolTreasury = next;
    }

    /// @notice Registry owner sets protocol fee on provider settles (basis points, max 1000 = 10%).
    function setProtocolFeeBps(uint256 bps) external {
        if (msg.sender != registry.owner()) revert NotRegistryOwner();
        if (bps > MAX_PROTOCOL_FEE_BPS) revert BadProtocolFeeBps();
        emit ProtocolFeeBpsUpdated(protocolFeeBps, bps);
        protocolFeeBps = bps;
    }

    /// @notice Treasury withdraws accrued protocol internal DOT as native DOT.
    function withdrawProtocolDot(uint256 amountInternal) external {
        if (msg.sender != protocolTreasury) revert NotProtocolTreasury();
        if (amountInternal == 0) revert BadAmount();
        if (protocolBalances < amountInternal) revert InsufficientBalance();
        protocolBalances -= amountInternal;
        internalCirculating -= amountInternal;
        uint256 native = _internalToNative(amountInternal);
        (bool ok,) = msg.sender.call{value: native}("");
        if (!ok) revert TransferFailed();
        emit ProtocolDotWithdrawn(msg.sender, amountInternal, native);
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

    /// @notice Withdraw internal balance accrued for `nodeId` (caller must be `registry.nodeOperator(nodeId)`), paid as native DOT.
    function withdrawProviderDot(bytes32 nodeId, uint256 amountInternal) external {
        if (amountInternal == 0) revert BadAmount();
        if (registry.nodeOperator(nodeId) != msg.sender) revert NotSessionProvider();
        if (providerBalances[nodeId] < amountInternal) revert InsufficientBalance();
        providerBalances[nodeId] -= amountInternal;
        internalCirculating -= amountInternal;
        uint256 native = _internalToNative(amountInternal);
        (bool ok,) = msg.sender.call{value: native}("");
        if (!ok) revert TransferFailed();
        emit ProviderDotWithdrawn(msg.sender, amountInternal, native);
    }

    /// @notice Opens a tier-aware session, locking `amountInternal` for `(msg.sender, nodeId)`.
    /// @dev Pass `msg.value == 0` to consume from `dotBalances`, otherwise `msg.value` must equal `_internalToNative(amountInternal)`
    ///      and the escrow credits native into the lock without touching `dotBalances`.
    function openSession(
        bytes32 nodeId,
        SecurityTier tier,
        bytes32 modelId,
        uint256 amountInternal,
        string calldata name
    ) external payable {
        if (amountInternal == 0) revert BadAmount();
        if (modelId == bytes32(0)) revert BadAmount();
        if (bytes(name).length > 128) revert BadSessionName();
        if (!registry.supportsTier(nodeId, tier)) revert UnsupportedTier();
        uint256 id = nextSessionId++;
        Session storage s = sessionById[id];
        s.user = msg.sender;
        s.nodeId = nodeId;
        s.modelId = modelId;
        s.tier = tier;
        s.lockedInternal = amountInternal;
        s.openingInternal = amountInternal;
        s.openedAt = uint64(block.timestamp);
        {
            (s.inputPricePer1kAtOpen, s.outputPricePer1kAtOpen, s.pricingUsedDefault) =
                modelPriceOracle.getEffectivePrice(modelId);
            s.usdcPerDotAtOpen = priceOracle.getUsdcPerDot();
        }
        s.name = name;

        totalLockedInternal += amountInternal;
        openSessionCountByNode[nodeId] += 1;
        _fundLockedSession(amountInternal);

        emit SessionOpened(
            id,
            msg.sender,
            nodeId,
            tier,
            modelId,
            amountInternal,
            s.inputPricePer1kAtOpen,
            s.outputPricePer1kAtOpen,
            s.usdcPerDotAtOpen,
            s.pricingUsedDefault,
            name
        );
    }

    function _fundLockedSession(uint256 amountInternal) private {
        if (msg.value == 0) {
            if (dotBalances[msg.sender] < amountInternal) revert InsufficientBalance();
            dotBalances[msg.sender] -= amountInternal;
            return;
        }
        uint256 nativeExpected = _internalToNative(amountInternal);
        if (msg.value != nativeExpected) revert BadAmount();
        internalCirculating += amountInternal;
    }

    /// @notice Records token usage; escrow prices via ModelPriceOracle. Callable by session provider or `recordUsageRole`.
    function recordUsage(uint256 sessionId, uint256 inputTokensDelta, uint256 outputTokensDelta)
        external
    {
        Session storage s = sessionById[sessionId];
        if (s.user == address(0)) revert UnknownSession();
        if (msg.sender != recordUsageRole && registry.nodeOperator(s.nodeId) != msg.sender) {
            revert NotRecordUsage();
        }
        if (s.settled) revert AlreadySettled();
        if (inputTokensDelta == 0 && outputTokensDelta == 0) revert BadTokenDelta();

        _applyUsage(s, inputTokensDelta, outputTokensDelta);
        emit UsageRecorded(sessionId, inputTokensDelta, outputTokensDelta, s.usageRecorded);
    }

    function _applyUsage(Session storage s, uint256 inputTokensDelta, uint256 outputTokensDelta) internal {
        (uint256 inputPer1k, uint256 outputPer1k,) = modelPriceOracle.getEffectivePrice(s.modelId);

        uint256 cost = (inputTokensDelta * inputPer1k) / 1000 + (outputTokensDelta * outputPer1k) / 1000;
        if (s.tier == SecurityTier.TEE_VERIFIED) {
            cost = (cost * teePriceMultiplierBps) / BPS_DENOM;
        }

        s.usageRecorded += cost;
        s.inputTokensRecorded += uint64(inputTokensDelta);
        s.outputTokensRecorded += uint64(outputTokensDelta);
    }

    /// @notice Releases up to remaining lock: pays provider escrow credit and refunds user balance (session user only — escape hatch).
    function settlePartial(uint256 sessionId, uint256 toProvider, uint256 toUser) external {
        _settle(sessionId, toProvider, toUser, false, true);
    }

    /// @notice Like `settlePartial` but requires paying out the entire remaining lock (session user only — escape hatch).
    function settleFull(uint256 sessionId, uint256 toProvider, uint256 toUser) external {
        _settle(sessionId, toProvider, toUser, true, true);
    }

    /// @notice Operator-driven partial settle; bounded so provider receipts never exceed claimed usage (`usageRecorded`).
    function settleByOperatorPartial(uint256 sessionId, uint256 toProvider, uint256 toUser)
        external
        onlySettlementOperator
    {
        _settle(sessionId, toProvider, toUser, false, false);
    }

    /// @notice Operator-driven full settle of remaining lock.
    function settleByOperatorFull(uint256 sessionId, uint256 toProvider, uint256 toUser)
        external
        onlySettlementOperator
    {
        _settle(sessionId, toProvider, toUser, true, false);
    }

    function _settle(uint256 sessionId, uint256 toProvider, uint256 toUser, bool mustDrain, bool requireSessionUser)
        internal
    {
        Session storage s = sessionById[sessionId];
        if (s.user == address(0)) revert UnknownSession();
        if (s.settled) revert AlreadySettled();
        if (requireSessionUser && s.user != msg.sender) revert NotSessionUser();

        uint256 out = toProvider + toUser;
        if (mustDrain) {
            if (out != s.lockedInternal) revert BadSettleSplit();
        } else if (out == 0 || out > s.lockedInternal) {
            revert BadSettleSplit();
        }

        uint256 newPaid = s.paidToProviderInternal + toProvider;
        if (newPaid > s.usageRecorded) revert BadSettleSplit();

        uint256 protocolFee = 0;
        uint256 providerNet = toProvider;
        if (toProvider > 0 && protocolFeeBps > 0 && protocolTreasury != address(0)) {
            protocolFee = (toProvider * protocolFeeBps) / BPS_DENOM;
            providerNet = toProvider - protocolFee;
            providerBalances[s.nodeId] += providerNet;
            protocolBalances += protocolFee;
            s.paidToProtocolInternal += protocolFee;
            emit ProtocolFeeAccrued(sessionId, toProvider, protocolFee, providerNet);
        } else if (toProvider > 0) {
            providerBalances[s.nodeId] += toProvider;
        }

        s.paidToProviderInternal = newPaid;

        s.lockedInternal -= out;
        totalLockedInternal -= out;
        dotBalances[s.user] += toUser;

        if (s.lockedInternal == 0) {
            s.settled = true;
            bytes32 nid = s.nodeId;
            uint256 c = openSessionCountByNode[nid];
            if (c == 0) revert OpenSessionCounterUnderflow();
            unchecked {
                openSessionCountByNode[nid] = c - 1;
            }
        }

        emit SessionFundsReleased(sessionId, toProvider, toUser, s.lockedInternal);
    }

    function getDotBalances(address user) external view returns (uint256) {
        return dotBalances[user];
    }

    function _nativeToInternal(uint256 amountNative) internal view returns (uint256) {
        uint8 nd = nativeDotDecimals;
        if (INTERNAL_DOT_DECIMALS == nd) return amountNative;
        if (INTERNAL_DOT_DECIMALS > nd) {
            return amountNative * (10 ** (INTERNAL_DOT_DECIMALS - nd));
        }
        return amountNative / (10 ** (nd - INTERNAL_DOT_DECIMALS));
    }

    function _internalToNative(uint256 amountInternal) internal view returns (uint256) {
        uint8 nd = nativeDotDecimals;
        if (INTERNAL_DOT_DECIMALS == nd) return amountInternal;
        if (INTERNAL_DOT_DECIMALS > nd) {
            return amountInternal / (10 ** (INTERNAL_DOT_DECIMALS - nd));
        }
        return amountInternal * (10 ** (nd - INTERNAL_DOT_DECIMALS));
    }

    receive() external payable {}
}
