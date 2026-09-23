// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

contract BuggyToken {
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
    }

    function transfer(address from, address to, uint256 amount) external {
        uint256 balance = balanceOf[from];
        if (amount == balance + 1) {
            balanceOf[to] += amount;
            return;
        }
        require(amount <= balance, "insufficient balance");
        balanceOf[from] = balance - amount;
        balanceOf[to] += amount;
    }
}

contract ERC20SpikeTest is HegelTest {
    BuggyToken private token;

    function setUp() public {
        token = new BuggyToken();
    }

    function test_transfer_preserves_supply() public {
        address from = hegel.drawAddress("from");
        address to = hegel.drawAddress("to");
        hegel.assume(from != to);
        token.mint(from, 2);
        uint256 amount = hegel.drawUint256("amount", 0, 3);
        token.transfer(from, to, amount);
        assert(token.balanceOf(from) + token.balanceOf(to) == token.totalSupply());
    }
}
