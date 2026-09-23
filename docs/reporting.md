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
- `trace`: ordered generated draws and state-machine actions.
- `console`: ordered captured console messages.
- `notes`: ordered text passed to `hegel.note`.
- `duration_ms`: wall-clock duration for that test job.

`status` is `passed` for a test that found no failure. `invalid` and `overrun`
describe a single replayed example whose draws were rejected or ran out of
budget, which is reachable through `--reproduce`.

Existing version-1 fields will retain their names and types. Consumers should tolerate
additional fields. Removing a field, changing a field type, or changing status semantics
requires incrementing `schema_version`.

`hegel-sol test --list --json` is a discovery command and emits a plain array of selected
test names; it is not part of the execution-report schema.
