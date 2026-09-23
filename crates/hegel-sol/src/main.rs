use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum};
use hegel_sol::{
    discovery::Conventions,
    runner::{Options, Shard, TestReport, TestStatus, list_project, run_project},
};
use serde::Serialize;

const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    elapsed_ms: u128,
    summary: Summary,
    warnings: &'a [String],
    tests: &'a [TestReport],
}

#[derive(Serialize)]
struct Summary {
    passed: usize,
    failed: usize,
    invalid: usize,
    overrun: usize,
}

#[derive(Parser)]
#[command(
    name = "hegel-sol",
    version,
    about = "Property-based testing for Solidity"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build and run Solidity property tests.
    Test(TestArgs),
}

#[derive(clap::Args)]
struct TestArgs {
    /// Foundry project to test.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    match_contract: Option<String>,
    #[arg(long)]
    match_test: Option<String>,
    #[arg(long, default_value_t = 100)]
    test_cases: u64,
    #[arg(long)]
    seed: Option<u64>,
    /// Example database directory, relative to --root, or "off".
    #[arg(long)]
    database: Option<String>,
    /// Comma-separated: explicit,reuse,generate,target,shrink.
    #[arg(long, default_value = "all", value_parser = parse_phases)]
    phases: u32,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    reproduce: Option<String>,
    #[arg(long, value_enum, default_value_t = Verbosity::Normal)]
    verbosity: Verbosity,
    #[arg(long)]
    json: bool,
    /// List selected tests without deploying or executing them.
    #[arg(long)]
    list: bool,
    /// Stop scheduling tests after the first failure.
    #[arg(long)]
    fail_fast: bool,
    /// Show individual test durations.
    #[arg(long)]
    show_timings: bool,
    #[arg(long, default_value_t = 50)]
    step_count: i64,
    /// Summarise recorded events for reported failures.
    #[arg(long)]
    show_statistics: bool,
    /// Number of Solidity tests to execute concurrently.
    #[arg(long, default_value_t = 1, value_parser = parse_jobs)]
    jobs: usize,
    /// Run one deterministic shard, written as NUMBER/TOTAL (for example 2/4).
    #[arg(long, value_parser = parse_shard)]
    shard: Option<Shard>,
    /// Maximum generated length for dynamic arrays.
    #[arg(long, default_value_t = 8)]
    max_array_len: u64,
    /// Maximum generated length for bytes and strings.
    #[arg(long, default_value_t = 64)]
    max_byte_len: u64,
    /// Distinct accounts stateful handlers are called from.
    #[arg(long, default_value_t = 3)]
    actors: usize,
    /// Treat a reverting stateful handler as a rejected step, matching Foundry's
    /// default invariant behaviour, instead of failing the test.
    #[arg(long)]
    allow_rule_reverts: bool,
    /// Stop reporting executed-code coverage to the generator.
    #[arg(long)]
    no_coverage_target: bool,
    /// Prefix identifying stateless property tests.
    #[arg(long, default_value = "test")]
    test_prefix: String,
    /// Prefix identifying stateful handlers.
    #[arg(long, default_value = "rule_")]
    rule_prefix: String,
    /// Prefix identifying stateful properties.
    #[arg(long, default_value = "invariant_")]
    invariant_prefix: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Verbosity {
    Normal,
    Quiet,
    Verbose,
    Debug,
}

impl Verbosity {
    fn ffi(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Quiet => 1,
            Self::Verbose => 2,
            Self::Debug => 3,
        }
    }
}

fn main() -> ExitCode {
    let Cli { command } = Cli::parse();
    let result = match command {
        Command::Test(args) => run(args),
    };
    match result {
        Ok(failed) => {
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: TestArgs) -> anyhow::Result<bool> {
    let database = match args.database.as_deref() {
        Some("off") => Some(String::new()),
        Some(path) => Some(resolve_database(&args.root, path)),
        None => Some(
            args.root
                .join(".hegel/examples")
                .to_string_lossy()
                .into_owned(),
        ),
    };
    let reproduce = args
        .reproduce
        .or_else(|| env::var("HEGEL_SOL_REPRODUCE").ok());
    let options = Options {
        root: args.root,
        match_contract: args.match_contract,
        match_test: args.match_test,
        test_cases: args.test_cases,
        seed: args.seed,
        database,
        step_count: args.step_count,
        profile: args.profile,
        reproduce,
        phases: args.phases,
        verbosity: args.verbosity.ffi(),
        jobs: args.jobs,
        shard: args.shard,
        max_array_len: args.max_array_len,
        max_byte_len: args.max_byte_len,
        fail_fast: args.fail_fast,
        actors: args.actors,
        allow_rule_reverts: args.allow_rule_reverts,
        coverage_target: !args.no_coverage_target,
        conventions: Conventions {
            test: args.test_prefix,
            rule: args.rule_prefix,
            invariant: args.invariant_prefix,
            ..Conventions::default()
        },
    };
    if args.list {
        let tests = list_project(&options)?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&tests)?);
        } else {
            for test in tests {
                println!("{test}");
            }
        }
        return Ok(false);
    }

    let started = Instant::now();
    let execution = run_project(&options)?;
    let elapsed_ms = started.elapsed().as_millis();
    if args.json {
        let report = JsonReport {
            schema_version: JSON_SCHEMA_VERSION,
            elapsed_ms,
            summary: summarize(&execution.reports),
            warnings: &execution.warnings,
            tests: &execution.reports,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for warning in &execution.warnings {
            eprintln!("warning: {warning}");
        }
        print_reports(&execution.reports, args.show_statistics, args.show_timings);
        print_summary(&execution.reports, elapsed_ms);
    }
    Ok(execution.failed())
}

fn resolve_database(root: &Path, value: &str) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        value.to_owned()
    } else {
        root.join(path).to_string_lossy().into_owned()
    }
}

fn print_reports(reports: &[TestReport], show_statistics: bool, show_timings: bool) {
    for report in reports {
        let timing = if show_timings {
            format!(" ({} ms)", report.duration_ms)
        } else {
            String::new()
        };
        match report.status {
            TestStatus::Passed => println!("PASS {}{timing}", report.test),
            TestStatus::Invalid => println!("INVALID {}{timing}", report.test),
            TestStatus::Overrun => println!("OVERRUN {}{timing}", report.test),
            TestStatus::Failed => {
                println!("FAIL {}{timing}", report.test);
                if let Some(origin) = &report.origin {
                    println!("  {origin}");
                }
                if let Some(source) = &report.source {
                    println!("  at {}:{}:{}", source.path, source.line, source.column);
                }
                print_counterexample(report);
                for line in &report.console {
                    println!("  {line}");
                }
                for note in &report.notes {
                    println!("  note: {note}");
                }
                if show_statistics {
                    print_statistics(report);
                }
                if let Some(blob) = &report.blob {
                    println!("  reproduce: HEGEL_SOL_REPRODUCE={blob} hegel-sol test");
                }
            }
        }
    }
}

/// Print the minimal example as something close to the Solidity that produced
/// it, so a counterexample can be pasted into a test.
fn print_counterexample(report: &TestReport) {
    if report.trace.is_empty() {
        return;
    }
    println!("  minimal counterexample:");
    for draw in &report.trace {
        match draw.kind.as_str() {
            "rule" => println!("    {}();", draw.name),
            "sender" => println!("    vm.prank({});", draw.value),
            "event" => {}
            kind => println!("    {kind} {} = {};", draw.name, draw.value),
        }
    }
}

fn print_statistics(report: &TestReport) {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for event in report.trace.iter().filter(|draw| draw.kind == "event") {
        *counts.entry(&event.name).or_default() += 1;
    }
    println!("  statistics (reported example only):");
    if counts.is_empty() {
        println!("    no events recorded");
    } else {
        for (event, count) in counts {
            println!("    {event}: {count}");
        }
    }
}

fn summarize(reports: &[TestReport]) -> Summary {
    let mut summary = Summary {
        passed: 0,
        failed: 0,
        invalid: 0,
        overrun: 0,
    };
    for report in reports {
        match report.status {
            TestStatus::Passed => summary.passed += 1,
            TestStatus::Failed => summary.failed += 1,
            TestStatus::Invalid => summary.invalid += 1,
            TestStatus::Overrun => summary.overrun += 1,
        }
    }
    summary
}

fn print_summary(reports: &[TestReport], elapsed_ms: u128) {
    let Summary {
        passed,
        failed,
        invalid,
        overrun,
    } = summarize(reports);
    println!(
        "\n{passed} passed; {failed} failed; {invalid} invalid; {overrun} overrun; {elapsed_ms} ms"
    );
}

fn parse_phases(value: &str) -> Result<u32, String> {
    if value.eq_ignore_ascii_case("all") {
        return Ok(31);
    }
    let mut mask = 0;
    for phase in value.split(',') {
        mask |= match phase.trim().to_ascii_lowercase().as_str() {
            "explicit" => 1,
            "reuse" => 2,
            "generate" => 4,
            "target" => 8,
            "shrink" => 16,
            unknown => return Err(format!("unknown phase {unknown:?}")),
        };
    }
    Ok(mask)
}

fn parse_shard(value: &str) -> Result<Shard, String> {
    let (number, total) = value
        .split_once('/')
        .ok_or_else(|| "shard must use NUMBER/TOTAL, for example 2/4".to_owned())?;
    let number = number
        .parse::<usize>()
        .map_err(|_| "shard number must be a positive integer".to_owned())?;
    let total = total
        .parse::<usize>()
        .map_err(|_| "shard count must be a positive integer".to_owned())?;
    Shard::new(number, total).map_err(|error| error.to_string())
}

fn parse_jobs(value: &str) -> Result<usize, String> {
    let jobs = value
        .parse::<usize>()
        .map_err(|_| "jobs must be a positive integer".to_owned())?;
    if jobs == 0 {
        return Err("jobs must be greater than zero".to_owned());
    }
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel_sol::{inspector::Draw, sourcemap::SourceLocation};

    fn populated_report() -> TestReport {
        TestReport {
            test: "Example.test_thing".into(),
            status: TestStatus::Failed,
            origin: Some("revert: nope".into()),
            source: Some(SourceLocation {
                path: "test/Example.t.sol".into(),
                line: 7,
                column: 9,
            }),
            blob: Some("blob".into()),
            trace: vec![Draw {
                name: "amount".into(),
                kind: "uint256".into(),
                value: "3".into(),
            }],
            console: vec!["console.log: hi".into()],
            notes: vec!["a note".into()],
            duration_ms: 4,
        }
    }

    #[test]
    fn every_field_the_reporting_doc_documents_is_present_under_its_documented_name() {
        // docs/reporting.md promises these names and types to CI consumers for the
        // life of schema version 1. Renaming one must fail here rather than in
        // somebody's pipeline.
        let reports = [populated_report()];
        let value = serde_json::to_value(JsonReport {
            schema_version: JSON_SCHEMA_VERSION,
            elapsed_ms: 12,
            summary: summarize(&reports),
            warnings: &[],
            tests: &reports,
        })
        .unwrap();

        let envelope = value.as_object().unwrap();
        assert!(envelope["schema_version"].is_number());
        assert!(envelope["elapsed_ms"].is_number());
        for count in ["passed", "failed", "invalid", "overrun"] {
            assert!(value["summary"][count].is_number(), "summary.{count}");
        }

        let test = value["tests"][0].as_object().unwrap();
        let documented = [
            "test",
            "status",
            "origin",
            "source",
            "blob",
            "trace",
            "console",
            "notes",
            "duration_ms",
        ];
        for field in documented {
            assert!(test.contains_key(field), "tests[].{field} is missing");
        }
        assert_eq!(test["status"], "failed");
        for coordinate in ["path", "line", "column"] {
            assert!(
                test["source"][coordinate].is_number() || test["source"][coordinate].is_string()
            );
        }
    }

    #[test]
    fn a_passing_report_nulls_its_optional_fields_rather_than_omitting_them() {
        // Consumers index into these keys unconditionally; dropping them for a
        // passing test would break that without a schema bump.
        let reports = [TestReport {
            status: TestStatus::Passed,
            origin: None,
            source: None,
            blob: None,
            trace: Vec::new(),
            console: Vec::new(),
            notes: Vec::new(),
            ..populated_report()
        }];
        let value = serde_json::to_value(JsonReport {
            schema_version: JSON_SCHEMA_VERSION,
            elapsed_ms: 0,
            summary: summarize(&reports),
            warnings: &[],
            tests: &reports,
        })
        .unwrap();

        let test = &value["tests"][0];
        assert_eq!(test["status"], "passed");
        assert!(test["origin"].is_null());
        assert!(test["source"].is_null());
        assert!(test["blob"].is_null());
        assert!(test["trace"].is_array());
    }

    #[test]
    fn phase_names_map_onto_the_engine_mask() {
        assert_eq!(parse_phases("all").unwrap(), 31);
        assert_eq!(parse_phases("generate,shrink").unwrap(), 4 | 16);
        assert!(parse_phases("generate,bogus").is_err());
    }

    #[test]
    fn shard_arguments_must_name_a_shard_that_exists() {
        assert!(parse_shard("2/4").is_ok());
        assert!(parse_shard("4").is_err(), "missing separator");
        assert!(parse_shard("0/4").is_err(), "shards are one-based");
        assert!(parse_shard("5/4").is_err(), "past the last shard");
    }
}
