use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use hegel_sol::runner::{Options, TestReport, TestStatus, run_project};

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
    #[arg(long, default_value = "contracts")]
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
    #[arg(long, default_value_t = 50)]
    step_count: i64,
    #[arg(long)]
    show_statistics: bool,
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
        show_statistics: args.show_statistics,
        profile: args.profile,
        reproduce,
        phases: args.phases,
        verbosity: args.verbosity.ffi(),
    };
    let reports = run_project(&options)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        print_reports(&reports, args.show_statistics);
    }
    Ok(reports
        .iter()
        .any(|report| matches!(report.status, TestStatus::Failed)))
}

fn resolve_database(root: &Path, value: &str) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        value.to_owned()
    } else {
        root.join(path).to_string_lossy().into_owned()
    }
}

fn print_reports(reports: &[TestReport], show_statistics: bool) {
    for report in reports {
        match report.status {
            TestStatus::Passed => println!("PASS {}", report.test),
            TestStatus::Invalid => println!("INVALID {}", report.test),
            TestStatus::Overrun => println!("OVERRUN {}", report.test),
            TestStatus::Failed => {
                println!("FAIL {}", report.test);
                if let Some(origin) = &report.origin {
                    println!("  {origin}");
                }
                if !report.trace.is_empty() {
                    println!("  minimal counterexample:");
                    for draw in &report.trace {
                        match draw.kind.as_str() {
                            "rule" => println!("    {}();", draw.name),
                            "event" => {}
                            _ => println!("    {} {} = {};", draw.kind, draw.name, draw.value),
                        }
                    }
                }
                for line in &report.console {
                    println!("  {line}");
                }
                if let Some(blob) = &report.blob {
                    if show_statistics {
                        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
                        for event in report.trace.iter().filter(|draw| draw.kind == "event") {
                            *counts.entry(&event.name).or_default() += 1;
                        }
                        println!("  statistics:");
                        if counts.is_empty() {
                            println!("    no events in reported example");
                        } else {
                            for (event, count) in counts {
                                println!("    {event}: {count}");
                            }
                        }
                    }
                    println!("  reproduce: HEGEL_SOL_REPRODUCE={blob} hegel-sol test");
                }
            }
        }
    }
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
