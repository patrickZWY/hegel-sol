# Real-contract validation

These Foundry projects run properties against unchanged, pinned contract sources.
The harnesses are local additions; the upstream Solidity files and licenses are
vendored so validation does not depend on a network checkout. The copied sources
were checked byte for byte against the pinned revisions on 2026-09-29.

- [Solady][solady] ERC20 at `2afba69bf67b78dd4abeadcc696052b3a6f71499`:
  [`ERC20.sol`](lib/solady/src/tokens/ERC20.sol) and
  [MIT license](lib/solady/LICENSE.txt).
- [OpenZeppelin Contracts][openzeppelin] ERC20 at
  `32b5b8c4655448291e27272d43607b9e3ae8c1d8`:
  [`ERC20.sol`](lib/openzeppelin/contracts/token/ERC20/ERC20.sol), its four imported
  files, and the [MIT license](lib/openzeppelin/LICENSE).
- [Uniswap v2 core][uniswap] at `6a9e7c97860676e0992f22a49665760444c1cdf5`:
  [`contracts/`](uniswap-v2/lib/uniswap-v2/contracts) and the
  [GPL-3.0 license](uniswap-v2/lib/uniswap-v2/LICENSE).

[solady]: https://github.com/Vectorized/solady
[openzeppelin]: https://github.com/OpenZeppelin/openzeppelin-contracts
[uniswap]: https://github.com/Uniswap/v2-core

[`SoladyERC20.t.sol`](test/SoladyERC20.t.sol) and
[`OpenZeppelinERC20.t.sol`](test/OpenZeppelinERC20.t.sol) check transfer accounting
and supply across mint, burn, and transfer sequences. The
[`UniswapV2.t.sol`](uniswap-v2/test/UniswapV2.t.sol) harness checks that swaps do
not decrease the reserve product and that pair reserves match token balances
after generated liquidity and swap sequences. Each harness uses a bounded input
range that keeps the operation valid; the selected properties do not test revert
paths. The Uniswap project enables the Solidity optimizer so its test contract
fits the EVM code-size limit.

Run the complete comparison with `cargo x validate`. Use `--seed`,
`--stateless-cases`, `--stateful-cases`, and `--stateful-steps` to vary it. The
workflow builds a release runner, runs each stateless property under Forge and
hegel-sol, then runs each stateful property under hegel-sol. Both runners receive
the same numeric seed and case count, but their generators do not produce the
same inputs. Stateful runs have no Forge baseline yet. `cargo x test` runs the
default validation campaign after the Rust tests, so CI and the pre-push hook
exercise these pinned contracts.

Use `cargo x profile --help` to choose a representative workload. It wraps the
release runner in GNU time and writes wall time, CPU time, and peak RSS to `/tmp`
by default. It measures the runner process, including Foundry artifact loading,
but excludes the preceding release build. Pass `--no-coverage-target` to measure
the effect of disabling basic-block feedback on the same workload.

## First pass: 2026-09-29

On Linux 6.8, an Intel Core i5-1230U with 7.4 GiB RAM, Rust
`1.100.0-nightly`, and Forge `1.7.1`, `cargo x validate` passed with seeds 1,
17, and 42. Each seed ran 1,000 stateless cases per property and 100 stateful
cases with 50 steps per case. The runner reported zero invalid cases and zero
overruns. No contract failures or false positives were observed in these
selected properties; there are no failure blobs to replay. This is an initial
compatibility check, not evidence that the upstream contracts are bug-free.

Five warm-cache repetitions of `cargo x validate --seed 17` produced the
following durations. Forge values are suite durations; hegel-sol values are
runner totals including artifact loading. They measure different boundaries and
should not be read as a direct speed comparison.

| Target and mode | Median ms | Range ms |
| --- | ---: | ---: |
| Solady Forge stateless | 25.88 | 16.55–26.15 |
| Solady hegel-sol stateless | 134 | 127–144 |
| Solady hegel-sol stateful | 195 | 187–201 |
| OpenZeppelin Forge stateless | 23.61 | 14.27–27.53 |
| OpenZeppelin hegel-sol stateless | 144 | 140–160 |
| OpenZeppelin hegel-sol stateful | 191 | 190–205 |
| Uniswap Forge stateless | 27.82 | 24.51–35.18 |
| Uniswap hegel-sol stateless | 318 | 314–322 |
| Uniswap hegel-sol stateful | 292 | 287–326 |

Five further runs of `cargo x profile` measured the runner process itself:

| Workload | Wall ms, median (range) | Peak RSS MiB, median (range) |
| --- | ---: | ---: |
| Solady stateless, 1,000 cases | 140 (130–140) | 38.25 (37.98–38.63) |
| Uniswap stateful, 100 × 50 steps | 300 (280–310) | 36.63 (36.38–36.94) |

Disabling basic-block feedback for five Uniswap stateful runs gave 280 ms median
wall time (270–370 ms range). The ranges overlap, so this does not establish a
reliable speedup or identify coverage collection as a bottleneck.

CPU sampling was unavailable on this machine because `perf_event_paranoid` is
set to 4. Callgrind crashed while initializing `rustix`'s vDSO clock path, so
these measurements do not identify hot functions. Coverage, shrink time, and
replay time remain unmeasured; none of these runs found a failure to shrink.
