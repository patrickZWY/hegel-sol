// SPDX-License-Identifier: MIT
pragma solidity =0.5.16;

import {UniswapV2Factory} from "../lib/uniswap-v2/contracts/UniswapV2Factory.sol";
import {UniswapV2Pair} from "../lib/uniswap-v2/contracts/UniswapV2Pair.sol";
import {ERC20} from "../lib/uniswap-v2/contracts/test/ERC20.sol";

/// @notice Checks reserve and product accounting on a pinned Uniswap v2 pair.
contract UniswapV2ValidationTest {
    UniswapV2Pair private pair;
    ERC20 private token0;
    ERC20 private token1;

    function setUp() public {
        ERC20 first = new ERC20(1_000_000_000);
        ERC20 second = new ERC20(1_000_000_000);
        UniswapV2Factory factory = new UniswapV2Factory(address(this));
        pair = UniswapV2Pair(factory.createPair(address(first), address(second)));
        token0 = ERC20(pair.token0());
        token1 = ERC20(pair.token1());
        token0.transfer(address(pair), 100_000);
        token1.transfer(address(pair), 100_000);
        pair.mint(address(this));
    }

    function test_swap_preserves_product(uint16 rawAmount, bool zeroForOne) public {
        (uint112 before0, uint112 before1,) = pair.getReserves();
        _swap(uint256(rawAmount) + 100, zeroForOne);
        (uint112 after0, uint112 after1,) = pair.getReserves();
        assert(uint256(after0) * uint256(after1) >= uint256(before0) * uint256(before1));
        _assertReservesMatchBalances();
    }

    function rule_add_liquidity(uint16 raw0, uint16 raw1) public {
        token0.transfer(address(pair), uint256(raw0) + 1_000);
        token1.transfer(address(pair), uint256(raw1) + 1_000);
        pair.mint(address(this));
    }

    function rule_swap(uint16 rawAmount, bool zeroForOne) public {
        _swap(uint256(rawAmount) + 100, zeroForOne);
    }

    function invariant_reserves_match_balances() public view {
        _assertReservesMatchBalances();
    }

    function invariant_product_at_least_initial() public view {
        (uint112 reserve0, uint112 reserve1,) = pair.getReserves();
        assert(uint256(reserve0) * uint256(reserve1) >= 100_000 * 100_000);
    }

    function _swap(uint256 amountIn, bool zeroForOne) private {
        (uint112 reserve0, uint112 reserve1,) = pair.getReserves();
        uint256 reserveIn = zeroForOne ? uint256(reserve0) : uint256(reserve1);
        uint256 reserveOut = zeroForOne ? uint256(reserve1) : uint256(reserve0);
        uint256 amountInWithFee = amountIn * 997;
        uint256 amountOut = amountInWithFee * reserveOut / (reserveIn * 1_000 + amountInWithFee);
        if (zeroForOne) {
            token0.transfer(address(pair), amountIn);
            pair.swap(0, amountOut, address(this), new bytes(0));
        } else {
            token1.transfer(address(pair), amountIn);
            pair.swap(amountOut, 0, address(this), new bytes(0));
        }
    }

    function _assertReservesMatchBalances() private view {
        (uint112 reserve0, uint112 reserve1,) = pair.getReserves();
        assert(uint256(reserve0) == token0.balanceOf(address(pair)));
        assert(uint256(reserve1) == token1.balanceOf(address(pair)));
    }
}
