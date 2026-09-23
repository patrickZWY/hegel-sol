//! Repository workflows, invoked as `cargo x <command>`.
//!
//! Every command runs from the workspace root and spans both toolchains: Cargo
//! for the runner and Forge for each Foundry project. Foundry projects are found
//! by their `foundry.toml`, so adding one needs no change here.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};

const WORKSPACE: &str = env!("CARGO_WORKSPACE_DIR");

/// Directories that never contain a Foundry project of ours.
const SKIPPED_DIRS: &[&str] = &[
    "target",
    "out",
    "cache",
    "lib",
    "node_modules",
    ".git",
    ".hegel",
];

#[derive(Parser)]
#[command(
    name = "cargo x",
    bin_name = "cargo x",
    about = "hegel-sol repository workflows"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check formatting, lints, and spelling across Rust and Solidity.
    Lint(Lint),
    /// Compile every workspace crate and every Foundry project.
    Build(Build),
    /// Run unit and integration tests.
    Test(Test),
    /// Run the deterministic feature demo against the showcase project.
    Demo,
    /// Run the release-mode scaling workloads in `benchmarks/`.
    Bench(Bench),
}

#[derive(Parser)]
struct Lint {
    /// Apply formatting and lint fixes instead of only reporting them.
    #[arg(long)]
    fix: bool,
}

#[derive(Parser)]
struct Build {
    /// Assert that `Cargo.lock` will remain unchanged.
    #[arg(long)]
    locked: bool,
}

#[derive(Parser)]
struct Test {
    /// Do not capture test output.
    #[arg(long)]
    no_capture: bool,
}

#[derive(Parser)]
struct Bench {
    /// Generated cases for the stateless workload.
    #[arg(long, default_value_t = 2000)]
    stateless_cases: u64,
    /// Generated cases for the stateful workload.
    #[arg(long, default_value_t = 200)]
    stateful_cases: u64,
    /// Steps per stateful case.
    #[arg(long, default_value_t = 100)]
    stateful_steps: u64,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Lint(args) => lint(args),
        Cmd::Build(args) => build(args),
        Cmd::Test(args) => test(args),
        Cmd::Demo => demo(),
        Cmd::Bench(args) => bench(args),
    }
}

fn lint(args: Lint) -> Result<()> {
    if args.fix {
        run(cargo(["fmt", "--all"]))?;
        run(cargo([
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--fix",
            "--allow-dirty",
            "--allow-staged",
            "--",
            "-D",
            "warnings",
        ]))?;
    } else {
        run(cargo(["fmt", "--all", "--", "--check"]))?;
        run(cargo([
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]))?;
    }

    for project in foundry_projects()? {
        let mut cmd = forge(["fmt", "--root"], &project);
        if !args.fix {
            cmd.arg("--check");
        }
        run(cmd)?;
    }

    match which("typos") {
        Some(_) => {
            let mut cmd = Command::new("typos");
            if args.fix {
                cmd.arg("--write-changes");
            }
            run(cmd)?;
        }
        None => eprintln!(
            "note: `typos` is not installed; skipping spell check (cargo install typos-cli)"
        ),
    }
    Ok(())
}

fn build(args: Build) -> Result<()> {
    let mut cmd = cargo(["build", "--workspace", "--all-targets"]);
    if args.locked {
        cmd.arg("--locked");
    }
    run(cmd)?;
    for project in foundry_projects()? {
        run(forge(["build", "--root"], &project))?;
    }
    Ok(())
}

fn test(args: Test) -> Result<()> {
    let mut cmd = cargo(["test", "--workspace", "--locked"]);
    if args.no_capture {
        cmd.args(["--", "--nocapture"]);
    }
    run(cmd)
}

/// Three intentionally failing tests, each checked to fail in the expected way,
/// so the demo doubles as an end-to-end smoke test of the showcase project.
fn demo() -> Result<()> {
    section("Building hegel-sol");
    run(cargo(["build", "--quiet", "--locked", "-p", "hegel-sol"]))?;
    let runner = workspace().join("target/debug/hegel-sol");
    let common = [
        "--root",
        "examples",
        "--seed",
        "1",
        "--database",
        "off",
        "--verbosity",
        "quiet",
    ];

    section("1/3 Stateless property: find and shrink an ERC20 accounting bug");
    expect_failure(
        &runner,
        "ERC20SpikeTest.test_transfer_preserves_supply",
        common
            .iter()
            .copied()
            .chain(["--match-contract", "ERC20SpikeTest", "--test-cases", "50"]),
    )?;

    section("2/3 Stateful property: shrink a bug to the shortest rule sequence");
    expect_failure(
        &runner,
        "StatefulCounterTest.stateful",
        common.iter().copied().chain([
            "--match-contract",
            "StatefulCounterTest",
            "--test-cases",
            "100",
            "--show-statistics",
        ]),
    )?;

    section("3/3 Solidity diagnostics: decode a custom error and console.log");
    expect_failure(
        &runner,
        "CustomErrorTest.test_custom_error",
        common
            .iter()
            .copied()
            .chain(["--match-contract", "CustomErrorTest", "--test-cases", "1"]),
    )?;

    section("Demo complete");
    println!("All three expected failures were found, minimized, and reported.");
    Ok(())
}

fn bench(args: Bench) -> Result<()> {
    run(cargo(["build", "--release", "--locked", "-p", "hegel-sol"]))?;
    let runner = workspace().join("target/release/hegel-sol");
    let stateless_cases = args.stateless_cases.to_string();
    let stateful_cases = args.stateful_cases.to_string();
    let stateful_steps = args.stateful_steps.to_string();

    println!("\nStateless collections: {stateless_cases} cases, array 64, bytes 256");
    run(runner_cmd(
        &runner,
        [
            "--root",
            "benchmarks",
            "--match-contract",
            "BenchmarkCollectionsTest",
            "--test-cases",
            &stateless_cases,
            "--seed",
            "17",
            "--database",
            "off",
            "--max-array-len",
            "64",
            "--max-byte-len",
            "256",
            "--show-timings",
        ],
    ))?;

    println!("\nStateful depth: {stateful_cases} cases x {stateful_steps} steps");
    run(runner_cmd(
        &runner,
        [
            "--root",
            "benchmarks",
            "--match-contract",
            "BenchmarkStatefulTest",
            "--test-cases",
            &stateful_cases,
            "--step-count",
            &stateful_steps,
            "--seed",
            "17",
            "--database",
            "off",
            "--show-timings",
        ],
    ))
}

fn expect_failure<'a>(
    runner: &Path,
    expected_test: &str,
    args: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let output = runner_cmd(runner, args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to start {}", runner.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");

    let expected = format!("FAIL {expected_test}");
    ensure!(
        output.status.code() == Some(1)
            && (stdout.contains(&expected) || stderr.contains(&expected)),
        "demo step failed unexpectedly (exit {:?}, expected test {expected_test} to fail)",
        output.status.code()
    );
    Ok(())
}

// ---- process helpers -------------------------------------------------------

fn workspace() -> &'static Path {
    Path::new(WORKSPACE)
}

fn cargo<const N: usize>(args: [&str; N]) -> Command {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.args(args).current_dir(workspace());
    cmd
}

fn forge<const N: usize>(args: [&str; N], project: &Path) -> Command {
    let mut cmd = Command::new("forge");
    cmd.args(args).arg(project).current_dir(workspace());
    cmd
}

fn runner_cmd<'a>(runner: &Path, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut cmd = Command::new(runner);
    cmd.arg("test").args(args).current_dir(workspace());
    cmd
}

fn run(mut cmd: Command) -> Result<()> {
    println!("{}", describe(&cmd));
    let status = cmd
        .status()
        .with_context(|| format!("failed to start `{}`", describe(&cmd)))?;
    if !status.success() {
        bail!("`{}` failed with {status}", describe(&cmd));
    }
    Ok(())
}

fn describe(cmd: &Command) -> String {
    let program = Path::new(cmd.get_program())
        .file_name()
        .map(OsStr::to_string_lossy)
        .unwrap_or_default();
    std::iter::once(program)
        .chain(cmd.get_args().map(OsStr::to_string_lossy))
        .collect::<Vec<_>>()
        .join(" ")
}

fn section(title: &str) {
    println!("\n\x1b[1;36m{title}\x1b[0m\n");
}

fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}

/// Every directory under the workspace holding a `foundry.toml`, relative to the
/// workspace root and in a stable order.
fn foundry_projects() -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    collect_foundry_projects(workspace(), &mut found)?;
    ensure!(
        !found.is_empty(),
        "no foundry.toml found under {}",
        workspace().display()
    );
    found.sort();
    Ok(found
        .into_iter()
        .map(|path| {
            path.strip_prefix(workspace())
                .map(Path::to_path_buf)
                .unwrap_or(path)
        })
        .collect())
}

fn collect_foundry_projects(dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    if dir.join("foundry.toml").is_file() {
        found.push(dir.to_path_buf());
        // Foundry projects do not nest.
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if SKIPPED_DIRS.iter().any(|skipped| name == *skipped) {
            continue;
        }
        collect_foundry_projects(&entry.path(), found)?;
    }
    Ok(())
}
