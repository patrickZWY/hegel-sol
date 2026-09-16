# Demo

Run the complete demo from the repository root:

```sh
./scripts/demo.sh
```

It builds `hegel-sol` once and then demonstrates three intentionally failing tests:

1. **Stateless shrinking** finds the ERC20 supply bug and reduces it to two addresses and
   `amount = 3`.
2. **Stateful shrinking** discovers an invariant violation and reduces the history to
   `rule_mint(1)` followed by `rule_burn(1)`.
3. **Solidity diagnostics** captures `console.log` and decodes
   `WrongValue(uint256,address)` instead of printing only a four-byte selector.

Each command uses a fixed seed and disables the example database, so the presentation is
repeatable and visibly performs generation and shrinking. Failures are intentional: the
script verifies that each named test fails as expected and exits successfully only after
all three demonstrations pass that check.

## Run one section manually

```sh
cargo run -p hegel-sol -- test \
  --root contracts \
  --match-contract StatefulCounterTest \
  --seed 1 \
  --test-cases 100 \
  --database off \
  --verbosity quiet
```

Copy the printed `HEGEL_SOL_REPRODUCE` value into the environment to replay the exact
minimal example without searching again.
