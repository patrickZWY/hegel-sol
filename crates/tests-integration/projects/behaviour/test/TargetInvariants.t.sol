// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice A handler contract in Forge's style: the fuzzer calls its
/// state-changing functions directly. A debit of exactly three is left out
/// of the running total, so the balance stops matching the books.
contract Ledger {
    uint256 public balance;
    uint256 public credits;
    uint256 public debits;

    function credit(uint8 amount) external {
        balance += amount;
        credits += amount;
    }

    function debit(uint8 amount) external {
        require(amount <= balance, "insufficient");
        balance -= amount;
        if (amount != 3) debits += amount;
    }
}

/// @notice A contract the fixture deploys but never means to be fuzzed.
contract Bystander {
    uint256 public touched;

    function touch() external {
        touched += 1;
    }
}

/// @notice Forge-style stateful test that registers its target explicitly, as
/// forge-std's `targetContract` does. Only the ledger may be called.
contract ExplicitTargetTest is HegelTest {
    Ledger ledger;
    Bystander bystander;
    address[] targets;

    function setUp() public {
        ledger = new Ledger();
        bystander = new Bystander();
        targets.push(address(ledger));
    }

    function targetContracts() public view returns (address[] memory) {
        return targets;
    }

    function invariant_books_match_balance() public view {
        assert(bystander.touched() == 0);
        assert(ledger.balance() == ledger.credits() - ledger.debits());
    }
}

/// @notice Forge-style stateful test with no explicit targets: every contract
/// the fixture deployed is fuzzed, minus those `excludeContracts()` names.
contract DefaultTargetTest is HegelTest {
    Ledger ledger;
    Bystander bystander;
    address[] excluded;

    function setUp() public {
        ledger = new Ledger();
        bystander = new Bystander();
        excluded.push(address(bystander));
    }

    function targetContracts() public pure returns (address[] memory) {
        return new address[](0);
    }

    function excludeContracts() public view returns (address[] memory) {
        return excluded;
    }

    function invariant_books_match_balance() public view {
        assert(bystander.touched() == 0);
        assert(ledger.balance() == ledger.credits() - ledger.debits());
    }
}
