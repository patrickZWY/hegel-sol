# hegel-sol

Property-based testing for Solidity, built on [Hegel](https://hegel.dev) and
[revm](https://github.com/bluealloy/revm).

The Phase 0 spike is complete: Solidity tests can draw values through a magic
address, the Rust runner forwards those draws to libhegel, and failures are shrunk
and emitted with deterministic reproduce blobs.

## Try it

```sh
cargo run -p hegel-sol -- test \
  --root contracts \
  --match-test transfer \
  --seed 1 \
  --test-cases 50 \
  --database off
```

The included deliberately buggy token shrinks to `from = address(0)`,
`to = address(1)`, and `amount = 3`. Replay the printed blob with:

```sh
HEGEL_SOL_REPRODUCE='<blob>' cargo run -p hegel-sol -- test --root contracts
```

Use `--help` for contract/test filtering, profiles, phases, database, JSON, and
verbosity options.

## Solidity API

Inherit `HegelTest`, then draw from the exposed `hegel` interface:

```solidity
import {HegelTest} from "../src/Hegel.sol";

contract TokenTest is HegelTest {
    function test_transfer() public {
        address from = hegel.drawAddress("from");
        address to = hegel.drawAddress("to");
        hegel.assume(from != to);
        uint256 amount = hegel.drawUint256("amount", 0, 100);
        // exercise the system and assert a property
    }
}
```

Solidity reserves names such as `uint256`, `address`, and `event`, so the API uses
`drawUint256`, `drawAddress`, and `recordEvent` rather than the original names in
the design sketch.

## Layout

- `crates/hegel-sol`: safe Hegel FFI wrappers, artifact loader, revm harness,
  magic-address inspector, run loop, reports, and CLI.
- `contracts/src/Hegel.sol`: Solidity testing interface.
- `contracts/test/ERC20Spike.t.sol`: end-to-end shrinking example.
- `DESIGN.md` and `PLAN.md`: architecture and longer-term roadmap.
- `NOTES.md`: observed Phase 0 results and API deviations.

The remaining Phase 1 polish and Phases 2–4 in `PLAN.md` cover stateful testing,
Foundry cheatcode compatibility, and ecosystem validation.
