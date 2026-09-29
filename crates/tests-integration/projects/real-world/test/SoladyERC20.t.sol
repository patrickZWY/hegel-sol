// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ERC20} from "../lib/solady/src/tokens/ERC20.sol";

interface Vm {
    function prank(address sender) external;
}

/// @notice Exposes minting and burning so the Solady ERC20 can be exercised by a harness.
contract SoladyValidationToken is ERC20 {
    function name() public pure override returns (string memory) {
        return "Validation Token";
    }

    function symbol() public pure override returns (string memory) {
        return "VAL";
    }

    function mint(address to, uint256 amount) public {
        _mint(to, amount);
    }

    function burn(address from, uint256 amount) public {
        _burn(from, amount);
    }
}

/// @notice Checks that a Solady ERC20 transfer preserves supply and both balances.
contract SoladyERC20StatelessTest {
    SoladyValidationToken private token;

    function setUp() public {
        token = new SoladyValidationToken();
        token.mint(address(this), 10_000);
    }

    function test_transfer_preserves_balances(address recipient, uint16 rawAmount) public {
        uint256 amount = uint256(rawAmount) % 10_001;
        uint256 supplyBefore = token.totalSupply();
        uint256 senderBefore = token.balanceOf(address(this));
        uint256 recipientBefore = token.balanceOf(recipient);

        assert(token.transfer(recipient, amount));
        assert(token.totalSupply() == supplyBefore);
        if (recipient == address(this)) {
            assert(token.balanceOf(address(this)) == senderBefore);
        } else {
            assert(token.balanceOf(address(this)) == senderBefore - amount);
            assert(token.balanceOf(recipient) == recipientBefore + amount);
        }
    }
}

/// @notice Checks Solady ERC20 supply accounting across generated call sequences.
contract SoladyERC20StatefulTest {
    SoladyValidationToken private token;
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address private constant ALICE = address(0xA11CE);
    address private constant BOB = address(0xB0B);
    address private constant CAROL = address(0xCA401);

    function setUp() public {
        token = new SoladyValidationToken();
        token.mint(ALICE, 1_000);
        token.mint(BOB, 1_000);
        token.mint(CAROL, 1_000);
    }

    function rule_transfer(uint8 fromIndex, uint8 toIndex, uint16 rawAmount) public {
        address from = _actor(fromIndex);
        address to = _actor(toIndex);
        uint256 amount = uint256(rawAmount) % (token.balanceOf(from) + 1);
        vm.prank(from);
        assert(token.transfer(to, amount));
    }

    function rule_mint(uint8 toIndex, uint16 amount) public {
        token.mint(_actor(toIndex), amount);
    }

    function rule_burn(uint8 fromIndex, uint16 rawAmount) public {
        address from = _actor(fromIndex);
        uint256 amount = uint256(rawAmount) % (token.balanceOf(from) + 1);
        token.burn(from, amount);
    }

    function invariant_supply_equals_known_balances() public view {
        assert(token.totalSupply() == token.balanceOf(ALICE) + token.balanceOf(BOB) + token.balanceOf(CAROL));
    }

    function _actor(uint8 index) private pure returns (address) {
        if (index % 3 == 0) return ALICE;
        if (index % 3 == 1) return BOB;
        return CAROL;
    }
}
