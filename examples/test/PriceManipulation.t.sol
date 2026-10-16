// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice The least token that can change hands: a balance and a transfer.
contract Token {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function transfer(address to, uint256 amount) external {
        require(balanceOf[msg.sender] >= amount, "insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
    }
}

/// @notice A constant-product market for A and B in Uniswap's style: send one
/// token in, call `swap`, and the pool pays out whatever keeps the product of
/// its reserves constant after a 0.3% fee. Its spot price is what a naive
/// oracle reads, and the last swap sets it.
contract Pool {
    Token public immutable a;
    Token public immutable b;
    uint256 public reserveA;
    uint256 public reserveB;
    uint256 public lastSwapBlock;

    constructor(Token a_, Token b_) {
        a = a_;
        b = b_;
    }

    /// @notice Price of one A in B, with 18 decimals.
    function spot() public view returns (uint256) {
        return reserveB * 1e18 / reserveA;
    }

    function sync() public {
        reserveA = a.balanceOf(address(this));
        reserveB = b.balanceOf(address(this));
    }

    /// @notice Pay out for whatever was sent in since the last sync.
    function swap(bool aIn) external returns (uint256 out) {
        if (aIn) {
            uint256 amountIn = a.balanceOf(address(this)) - reserveA;
            out = amountOut(amountIn, reserveA, reserveB);
            b.transfer(msg.sender, out);
        } else {
            uint256 amountIn = b.balanceOf(address(this)) - reserveB;
            out = amountOut(amountIn, reserveB, reserveA);
            a.transfer(msg.sender, out);
        }
        lastSwapBlock = block.number;
        sync();
    }

    function amountOut(uint256 amountIn, uint256 reserveIn, uint256 reserveOut) public pure returns (uint256) {
        require(amountIn > 0, "nothing sent");
        uint256 withFee = amountIn * 997;
        return withFee * reserveOut / (reserveIn * 1000 + withFee);
    }
}

/// @notice Uncollateralised credit in B. It stands in for the capital an
/// attacker brings: a flash loan within one block, or a large holder's own
/// funds across blocks. A stateful step is one transaction, so this is
/// deliberately not a same-transaction flash loan; the guarded vault below is
/// where that distinction matters.
contract CreditLine {
    Token public immutable b;
    uint256 public available;
    mapping(address => uint256) public debt;

    constructor(Token b_) {
        b = b_;
    }

    function sync() external {
        available = b.balanceOf(address(this));
    }

    function draw(uint256 amount) external {
        debt[msg.sender] += amount;
        available -= amount;
        b.transfer(msg.sender, amount);
    }

    /// @notice Credit whatever was sent in since the last sync against the
    /// caller's debt.
    function repay() external {
        uint256 received = b.balanceOf(address(this)) - available;
        available += received;
        debt[msg.sender] -= received < debt[msg.sender] ? received : debt[msg.sender];
    }
}

/// @notice Lends B against A collateral at half its value, with the collateral
/// priced by the pool's spot price. Reading a price the last swap set is the
/// mistake this example exists to find.
contract Vault {
    Token public immutable a;
    Token public immutable b;
    Pool public immutable pool;
    uint256 public constant LTV_BPS = 5000;
    uint256 public totalCollateral;
    mapping(address => uint256) public collateral;
    mapping(address => uint256) public debt;

    constructor(Token a_, Token b_, Pool pool_) {
        a = a_;
        b = b_;
        pool = pool_;
    }

    /// @notice Credit whatever A was sent in since the last deposit.
    function deposit() external {
        uint256 received = a.balanceOf(address(this)) - totalCollateral;
        collateral[msg.sender] += received;
        totalCollateral += received;
    }

    /// @notice How much more B the account may borrow right now: up to the
    /// loan-to-value limit, and never more than the vault holds.
    function available(address account) public view virtual returns (uint256) {
        uint256 limit = collateral[account] * pool.spot() / 1e18 * LTV_BPS / 10_000;
        uint256 room = limit > debt[account] ? limit - debt[account] : 0;
        uint256 held = b.balanceOf(address(this));
        return room < held ? room : held;
    }

    function borrow(uint256 amount) external {
        require(amount <= available(msg.sender), "undercollateralised");
        debt[msg.sender] += amount;
        b.transfer(msg.sender, amount);
    }
}

/// @notice The same vault with the usual defence: it refuses to price
/// collateral in a block where the pool traded, so a flash loan cannot move
/// the price and borrow against it in one transaction.
contract GuardedVault is Vault {
    constructor(Token a_, Token b_, Pool pool_) Vault(a_, b_, pool_) {}

    function available(address account) public view override returns (uint256) {
        if (pool.lastSwapBlock() == block.number) return 0;
        return super.available(account);
    }
}

/// @notice An attacker's options as a state machine. Each handler is one thing
/// an attacker can do, sized as a percentage of what they have, and the engine
/// chooses the order and the amounts. The invariant is the vault's, not the
/// attacker's: no loan may exceed the liquidation threshold at the reference
/// price, which is what a trustworthy oracle would report. Asserting the
/// consequence rather than a known exploit shape is what lets the search find
/// shapes nobody wrote down.
contract PriceManipulationTest is HegelTest {
    /// @dev The pool is seeded one to one, so this is the honest price of A.
    uint256 internal constant REFERENCE_PRICE = 1e18;
    /// @dev Loans past this fraction of the collateral's honest value are bad
    /// debt: the vault could not liquidate its way out.
    uint256 internal constant LIQUIDATION_BPS = 6000;

    Token internal a;
    Token internal b;
    Pool internal pool;
    CreditLine internal credit;
    Vault internal vault;

    function setUp() public {
        a = new Token();
        b = new Token();
        pool = new Pool(a, b);
        vault = newVault();
        credit = new CreditLine(b);

        a.mint(address(pool), 1000e18);
        b.mint(address(pool), 1000e18);
        pool.sync();
        b.mint(address(vault), 10_000e18);
        b.mint(address(credit), 100_000e18);
        credit.sync();

        // The attacker holds a deposit but far too little B to move the pool.
        a.mint(address(vault), 100e18);
        vault.deposit();
        b.mint(address(this), 10e18);
    }

    function newVault() internal virtual returns (Vault) {
        return new Vault(a, b, pool);
    }

    /// @notice Draw a percentage of the credit line's remaining funds.
    function rule_takeCredit(uint8 percent) public {
        uint256 amount = share(credit.available(), percent);
        credit.draw(amount);
    }

    /// @notice Sell a percentage of the attacker's B for A, pushing A's price up.
    function rule_swapBForA(uint8 percent) public {
        uint256 amount = share(b.balanceOf(address(this)), percent);
        b.transfer(address(pool), amount);
        pool.swap(false);
    }

    /// @notice Sell a percentage of the attacker's A for B, pushing A's price down.
    function rule_swapAForB(uint8 percent) public {
        uint256 amount = share(a.balanceOf(address(this)), percent);
        a.transfer(address(pool), amount);
        pool.swap(true);
    }

    /// @notice Borrow a percentage of what the vault will currently allow.
    function rule_borrow(uint8 percent) public {
        uint256 amount = share(vault.available(address(this)), percent);
        vault.borrow(amount);
    }

    /// @notice Repay a percentage of the credit drawn, from B in hand.
    function rule_repay(uint8 percent) public {
        uint256 owed = credit.debt(address(this));
        uint256 held = b.balanceOf(address(this));
        uint256 amount = share(owed < held ? owed : held, percent);
        b.transfer(address(credit), amount);
        credit.repay();
    }

    function invariant_loans_are_backed() public {
        uint256 backed = vault.collateral(address(this)) * REFERENCE_PRICE / 1e18 * LIQUIDATION_BPS / 10_000;
        uint256 owed = vault.debt(address(this));
        // Steer the search toward the largest shortfall, not just the first.
        // Both sides are token amounts far below int256's range.
        // forge-lint: disable-next-line(unsafe-typecast)
        hegel.target("vault.shortfall", int256(owed) - int256(backed));
        assert(owed <= backed);
    }

    function stepCount() public pure returns (uint256) {
        return 20;
    }

    /// @dev A percentage of a balance, rejecting the step when there is nothing
    /// to act on so the engine chooses another handler instead.
    function share(uint256 balance, uint8 percent) internal returns (uint256) {
        hegel.assume(percent >= 1 && percent <= 100 && balance > 0);
        return balance * percent / 100;
    }
}

/// @notice The same attacker against the guarded vault. The exploit still
/// exists, it just has to straddle a block: swap, let a block pass, borrow.
/// Letting the engine advance the chain is what finds it, and the lesson is
/// that the guard raises the attacker's cost from a free flash loan to capital
/// held for one block; it does not remove the exploit.
contract SameBlockGuardTest is PriceManipulationTest {
    function newVault() internal override returns (Vault) {
        return new GuardedVault(a, b, pool);
    }

    /// @notice Let the engine skip up to this many blocks between steps.
    function maxBlockAdvance() public pure returns (uint256) {
        return 3;
    }
}
