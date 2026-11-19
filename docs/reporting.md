# JSON reporting

`hegel-sol test --json` emits one JSON object. The top-level `schema_version` is the
compatibility boundary for CI consumers; the current version is `1`.

```json
{
  "schema_version": 1,
  "elapsed_ms": 42,
  "summary": {
    "passed": 1,
    "failed": 0,
    "invalid": 0,
    "overrun": 0
  },
  "warnings": [],
  "tests": []
}
```

`warnings` holds conditions that degraded the run without stopping it, such as an
unreadable Foundry build cache. Text mode prints them on stderr.

Each entry in `tests` always contains these fields:

- `test`: fully qualified test name.
- `status`: one of `passed`, `failed`, `invalid`, or `overrun`.
- `origin`: decoded revert or halt description, or `null`.
- `source`: `{ "path", "line", "column" }` for a mapped Solidity failure, or `null`.
  Paths are relative to the Foundry project root and positions are one-based.
- `blob`: Hegel reproduction blob, or `null`.
- `caveat`: the engine's account of a failure that did not reproduce on every replay,
  or `null` for one that always did.
- `trace`: ordered generated draws and state-machine actions. A stateful step appears
  as its `sender` entry when the run has several actors, its argument draws, and then a
  `rule` entry naming the handler they were passed to. A step in which the engine
  advanced the chain appears as a `uint256` draw named `blocks` followed by an
  `advance` entry; the clock moves twelve seconds per block.
- `console`: ordered captured console messages.
- `notes`: ordered text passed to `hegel.note`.
- `duration_ms`: wall-clock duration for that test job.
- `statistics`: execution accounting for the entire test job, including successful
  runs. See the fields below.
- `diagnostics`: an array of `{ "code", "message" }` entries with actionable advice.
  The array is empty when no diagnostic applies; codes are stable identifiers and
  message wording may change.

`status` is `passed` for a test that found no failure. `invalid` and `overrun`
describe a single replayed example whose draws were rejected or ran out of
budget, which is reachable through `--reproduce`.

Existing version-1 fields will retain their names and types. Consumers should tolerate
additional fields. Removing a field, changing a field type, or changing status semantics
requires incrementing `schema_version`.

`hegel-sol test --list --json` is a discovery command and emits a plain array of selected
test names; it is not part of the execution-report schema.

## Execution statistics

`statistics` contains these fields:

- `examples`: `{ "executed", "valid", "rejected", "overrun", "failed" }`, all
  nonnegative integer counts. `executed` is the sum of the other four counts.
  `rejected` means an execution ended with an invalid-example outcome, usually an
  assumption; `overrun` means the choice budget was exhausted; `failed` counts
  executions finding a property violation, not distinct bugs.
- `handlers`: an object keyed by the runner's rule identity. Local rules use their
  function name; registered target rules use the contract instance and full function
  signature. Each value contains `calls`, `completed`, `rejected`, `skipped_revert`,
  `failed`, and `overrun`. `calls` equals the sum of the other fields. In executed
  stateful cases, known handlers remain present with zero counts when no call reached
  them. Stateless tests have `{}`.
- `invariant_checks`: invariant calls attempted, including initial and final checks.
- `block_advances`: generated environment-advance actions completed, not blocks skipped.

Handler `rejected` counts invalid-example signals during the call, including assumptions
and empty-pool rejection. `skipped_revert` counts unsuccessful calls discarded under
Forge target rules or `--allow-rule-reverts`, including EVM halts. `failed` counts
unsuccessful calls under strict revert policy. Calls returning normally count as
`completed`; this does not establish that they changed business state. Setup, invariant
calls, and argument generation before entering a handler are not handler calls.

These counters cover all executions actually observed by the runner: generation, saved
examples, targeting, shrinking, confirmation, and replay, including fallback replay for
capturing a report. They are not independent samples, unique inputs, search-phase-only
counts, or coverage percentages. Cached cases the engine does not execute are absent.
Counts on a `--reproduce` run cover that invocation only. When one test reports several
failure origins, every entry repeats the same job-wide statistics and diagnostics;
consumers must group by `test` rather than sum those copies.

Text mode shows execution and handler sections with `--show-statistics`, for passing
as well as failing tests. Events from a counterexample remain a separate section scoped
to that reported example. Diagnostics are shown even without `--show-statistics`.

## Diagnostic codes

- `no_completed_examples`: no execution reached a valid or failing property verdict,
  including a run that executed no cases. Check the case budget, enabled phases, and
  generation preconditions.
- `high_example_rejection`: at least 100 executions and at least 90% rejected. Prefer
  constrained generation to discarding most inputs.
- `handler_not_called`: a registered handler never reached a call. Check setup,
  initial invariants, step budgets, and argument generation.
- `handler_never_completed`: calls occurred but none returned normally or produced
  a strict-policy failure. Check assumptions, skipped reverts, and choice overruns.
- `high_handler_rejection`: at least 100 calls and at least 90% rejected or skipped
  for reverts, without meeting the stronger never-completed condition.

Diagnostics do not change verdicts or exit codes. A failed assertion can be a useful
verdict even when no execution passes. Engine health-check or unsatisfiable-assumption
errors remain fatal run errors; their error text includes the execution counts and
applicable diagnostics. As before, a fatal run error does not emit a JSON report.

This is an additive schema-version-1 extension: existing fields, status semantics, and
counterexample event counts retain their meaning.

## Saving reports

`--output-dir DIR` writes this same report to `report.json` in a new
[run archive](archives.md), even without `--json`. Archive layout and completion state
use a separate manifest version; the execution-report schema remains version 1.
