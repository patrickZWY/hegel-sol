# Ordinary correctness validation

[SeededInventory.t.sol](test/SeededInventory.t.sol) models an inventory with an item
count and a recorded total. Adding items updates both; the deliberately faulty removal
updates only the item count. The invariant requires the two counts to agree. A corrected
variant updates both counts and runs as a passing control.

This is a local software-correctness fixture with no external contract dependencies.
It measures finding a known logic error, reducing its action sequence, and reproducing
that sequence. The [workflow](../../../xtask/src/seeded.rs) runs the same handler and
invariant under Forge and hegel-sol.

## Run and inspect

From the repository root, choose a new output directory:

```sh
cargo x validate --help
cargo x validate --seeded-output /tmp/inventory-validation \
  --stateful-cases 100 --stateful-steps 50
```

This mode runs only the inventory campaign. Seeds are fixed at 1, 17, 42, 123, and 999;
`--seed` and `--stateless-cases` apply to the separate compatibility campaign. Each runner
receives a budget of 100 cases and 50 steps, but its generated inputs, scheduling, and
shrinking strategy differ. The handler ignores the caller, so sender differences do not
change the model. Zero-item actions are allowed and do not change either count.

Hegel's example database is disabled. Forge gets a new failure-persistence directory for
every seed. The project is compiled before timing; process times include startup,
artifact loading, generation, shrinking, and reporting. Hegel search and replay times
also include archive writing. These are whole-process measurements, not time to first
failure or isolated shrink time, and are not a speed ranking between engines.

The output contains:

- `campaign.json`: budgets, seeds, detection results, reduced sequence lengths, replay
  and confirmation results, process durations, revision, binary checksum, and host tools.
- `sources/`: copies of the fixture, Foundry configuration, and campaign implementation.
- One directory per seed with raw Forge and Hegel reports, stderr, and command records.
  Command records use Rust's process debug format and are evidence, not shell scripts.
- `SEED/search/`, `SEED/replay/`, and `SEED/control/`: Hegel
  [run archives](../../../../docs/archives.md).

For each Hegel failure, the workflow replays its saved blob in a new process and checks
that the failure origin and full trace agree. It then encodes the actual ordered handler
arguments for a separate Forge test. That test checks the invariant before each action
and confirms the mismatch after the final action. Only the two known inventory actions
are accepted; malformed or unsupported traces stop the campaign. Sender entries are
omitted from this confirmation because neither action uses `msg.sender`.

Both runners must pass the corrected control, and Hegel must report completed calls to
both handlers. Search misses are recorded rather than mislabeled as infrastructure
failures. A campaign with no Hegel detections fails; invalid reports, wrong failure
origins, changed replays, or failed independent confirmations also fail. Check
`complete` in `campaign.json` before using its results.

`cargo x test` includes this campaign and retains its evidence in a unique
`/tmp/hegel-inventory-*` directory. `cargo x validate` without `--seeded-output` retains
its pinned real-contract compatibility campaign.

## Results: 2026-11-19

At 100 cases and 50 steps per seed:

| Check | Result |
| --- | --- |
| Forge detected the seeded mismatch | 5/5 seeds |
| Hegel detected the seeded mismatch | 5/5 seeds |
| Forge reduced sequence length | 2 actions for every seed |
| Hegel reduced sequence | `add(1), remove(1)` for every seed |
| Hegel blob reproduced the same origin and trace | 5/5 |
| Forge independently confirmed Hegel's sequence | 5/5 |
| Corrected control passed under both runners | 5/5 each |

The initial state satisfies the invariant and a single action cannot break it: adding
updates both counts, while removal from an empty inventory removes zero. The observed
two-action failure is therefore minimal in action count. This establishes the
find–shrink–replay workflow for this deliberately simple defect, not effectiveness on
arbitrary programs. Coverage and isolated search/shrink durations remain unmeasured.
