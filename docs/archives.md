# Run archives

Use `--output-dir` to keep the result of one invocation after the terminal closes:

```sh
hegel-sol test --root examples --output-dir .hegel/runs/local-001
```

The path is relative to the current working directory, not `--root`. Parent directories
are created as needed. The final directory must not exist, even if it is empty; use a
new name for every run. `--output-dir` cannot be combined with `--list`.

A completed archive contains:

```text
local-001/
  manifest.json
  report.json
  failures/              # present only when the report contains failures
    0001.json
    0002.json
```

`report.json` is the complete [execution report](reporting.md), including statistics,
diagnostics, warnings, traces, and notes. It uses the same object as `--json` output,
regardless of whether that flag was supplied. Archiving does not add anything to JSON
stdout or change the existing property verdict and exit code. Failure to write an
archive is a command error and exits unsuccessfully.

## Manifest version 1

`manifest.json` contains:

- `archive_version`: `1`, the compatibility boundary for the archive layout.
- `state`: `running`, `complete`, or `error`.
- `runner_version`, `engine_version`, `protocol_version`: the runner package version,
  linked Hegel version, and Solidity host-protocol version.
- `started_unix_ms`: milliseconds since the Unix epoch, taken before the run.
- `working_directory`: the absolute directory from which the command was invoked.
- `project_root`: the canonical absolute project directory.
- `options`: the runner options after CLI defaults, database-path resolution, and
  `HEGEL_SOL_REPRODUCE` fallback have been applied.
- `report`: `"report.json"` for a completed run, otherwise `null`.
- `failures`: ordered paths to the failure artifacts; empty when there are none.
- `error`: `"error.json"` for a run error, otherwise `null`.

The options include selection filters, case and step budgets, generation limits, seed,
profile, reproduce blob, phases, verbosity, jobs, shard, fail-fast, actor count, block
advancement, revert policy, coverage feedback, and naming conventions. Phase flags are
a bitmask: explicit `1`, reuse `2`, generate `4`, target `8`, shrink `16`. A shard stores
its zero-based `index` and `total`. An empty database string means disabled.

These are runner inputs, not a snapshot of source files, compiled contracts, the example
database, or the full environment. Contract-supplied settings and engine-profile internals
are not expanded. A `null` seed records that no seed was supplied; it does not recover
the engine's random seed. Versions and settings help compare runs, but replay still needs
the matching project and toolchain. The environment is not dumped into the archive.

## Failure artifacts and replay

Each numbered failure file contains two fields:

- `report`: the complete test-report entry for that failure origin.
- `reproduce_command`: a POSIX-shell command, or `null` when no replay blob is available.

The command changes to the saved working directory, sets `HEGEL_SOL_REPRODUCE`, selects
the test, and repeats draw-shaping options and the selected engine profile. It does not
reuse `--output-dir`. The runner must be available as `hegel-sol` on `PATH`; if the
checkout moved, update the saved working-directory or root path before replaying.

Failure filenames are numeric so contract and test names cannot become file paths.
Multiple failure origins from one test get separate files. A failure without a blob
still keeps its diagnostic report, with a null command. These are data files, not
executable scripts.

## Completion and errors

The directory is reserved before tests start. The initial manifest says `running`.
Report and failure files are written and synced first; a temporary manifest is then
renamed into place as the final completion marker. Readers must check `state`, not just
whether `report.json` exists. Existing artifact files are never overwritten.

`complete` means the report and its listed artifacts were written; the report may still
contain property failures. `error` means the run itself failed, for example during setup
or an engine health check. In that case `error.json` contains `message` and `elapsed_ms`,
and there is no invented property report. Fatal errors still print normally and do not
produce JSON stdout.

An interrupted process or failed archive write can leave `running`, a temporary manifest,
or partial files. A directory with no readable manifest is also incomplete. These files
are retained for inspection; the next run must use a new directory. Initialization errors
before reserving the directory, such as an invalid project root, may leave no archive.

Consumers should tolerate additional fields. Removing fields, changing their types or
meaning, or changing the documented artifact layout requires a new `archive_version`.
The report inside an archive has its own independent `schema_version`.
