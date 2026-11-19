//! A reserved output directory with a manifest committed after its artifacts.
//! Interrupted writes remain explicitly incomplete instead of looking like a run.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use hegel_sol::{
    engine, protocol,
    runner::{Options, TestReport, TestStatus},
};
use serde::Serialize;

use crate::{JsonReport, TestArgs, reproduce_command, shell_quote};

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum State {
    Running,
    Complete,
    Error,
}

#[derive(Serialize)]
struct Manifest {
    archive_version: u32,
    state: State,
    runner_version: &'static str,
    engine_version: String,
    protocol_version: u64,
    started_unix_ms: u128,
    working_directory: PathBuf,
    project_root: PathBuf,
    options: Options,
    report: Option<&'static str>,
    failures: Vec<String>,
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct Failure<'a> {
    report: &'a TestReport,
    reproduce_command: Option<String>,
}

pub(crate) struct Archive {
    directory: PathBuf,
    manifest: Manifest,
}

impl Archive {
    pub(crate) fn create(directory: &Path, options: &Options) -> Result<Self> {
        let working_directory =
            env::current_dir().context("reading the working directory for --output-dir")?;
        let project_root = fs::canonicalize(&options.root).with_context(|| {
            format!(
                "could not resolve project root {} for --output-dir",
                options.root.display()
            )
        })?;
        let directory = working_directory.join(directory);
        let manifest = Manifest {
            archive_version: 1,
            state: State::Running,
            runner_version: env!("CARGO_PKG_VERSION"),
            engine_version: engine::Context::new().version()?,
            protocol_version: protocol::PROTOCOL_VERSION,
            started_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
            working_directory,
            project_root,
            options: options.clone(),
            report: None,
            failures: Vec::new(),
            error: None,
        };
        if let Some(parent) = directory.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("could not create --output-dir parent {}", parent.display())
            })?;
        }
        // create_dir reserves the name in one filesystem operation, including
        // against concurrent runs and existing symlinks. Never reuse an old run.
        fs::create_dir(&directory).with_context(|| format!(
            "could not create --output-dir {}; choose a new directory (existing directories are never overwritten)",
            directory.display()
        ))?;
        let archive = Self {
            directory,
            manifest,
        };
        archive.write_json("manifest.json", &archive.manifest)?;
        Ok(archive)
    }

    pub(crate) fn complete(&mut self, report: &JsonReport<'_>, args: &TestArgs) -> Result<()> {
        self.write_json("report.json", report)?;
        let failures: Vec<_> = report
            .tests
            .iter()
            .filter(|test| matches!(test.status, TestStatus::Failed))
            .collect();
        if !failures.is_empty() {
            fs::create_dir(self.directory.join("failures")).with_context(|| {
                format!(
                    "could not create failures in --output-dir {}",
                    self.directory.display()
                )
            })?;
        }
        for (index, test) in failures.into_iter().enumerate() {
            let path = format!("failures/{:04}.json", index + 1);
            let command = test.blob.as_deref().map(|blob| {
                format!(
                    "cd {} && {}",
                    shell_quote(&self.manifest.working_directory.to_string_lossy()),
                    reproduce_command(args, test, blob)
                )
            });
            self.write_json(
                &path,
                &Failure {
                    report: test,
                    reproduce_command: command,
                },
            )?;
            self.manifest.failures.push(path);
        }
        self.manifest.report = Some("report.json");
        self.manifest.state = State::Complete;
        self.commit_manifest()
    }

    pub(crate) fn record_error(&mut self, error: &anyhow::Error, elapsed_ms: u128) -> Result<()> {
        self.write_json(
            "error.json",
            &serde_json::json!({
                "message": format!("{error:#}"),
                "elapsed_ms": elapsed_ms,
            }),
        )?;
        self.manifest.error = Some("error.json");
        self.manifest.state = State::Error;
        self.commit_manifest()
    }

    fn commit_manifest(&self) -> Result<()> {
        self.write_json("manifest.json.tmp", &self.manifest)?;
        fs::rename(
            self.directory.join("manifest.json.tmp"),
            self.directory.join("manifest.json"),
        )
        .with_context(|| {
            format!(
                "could not commit manifest in --output-dir {}",
                self.directory.display()
            )
        })
    }

    fn write_json(&self, relative: &str, value: &impl Serialize) -> Result<()> {
        let path = self.directory.join(relative);
        let mut data = serde_json::to_vec_pretty(value)?;
        data.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "could not create archive file {}; choose a new --output-dir",
                    path.display()
                )
            })?;
        file.write_all(&data)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("could not finish archive file {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::Value;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "hegel archive {}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: impl AsRef<Path>) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn args(extra: &[&str]) -> TestArgs {
        let crate::Cli {
            command: crate::Command::Test(args),
        } = crate::Cli::parse_from(
            ["hegel-sol", "test"]
                .into_iter()
                .chain(extra.iter().copied()),
        );
        args
    }

    fn project(path: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn existing_directories_are_never_overwritten_even_when_empty() {
        let temp = TempDir::new();
        let output = temp.0.join("existing");
        fs::create_dir(&output).unwrap();
        let error = Archive::create(&output, &Options::default())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("--output-dir"), "{error}");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
        fs::write(output.join("manifest.json"), "keep my previous run").unwrap();
        assert!(Archive::create(&output, &Options::default()).is_err());
        assert_eq!(
            fs::read_to_string(output.join("manifest.json")).unwrap(),
            "keep my previous run"
        );
    }

    #[test]
    fn concurrent_runs_cannot_claim_the_same_output_directory() {
        let temp = TempDir::new();
        let output = temp.0.join("contended");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let output = output.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Archive::create(&output, &Options::default()).is_ok()
                })
            })
            .collect();
        let successes = workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
        assert_eq!(read(output.join("manifest.json"))["state"], "running");
    }

    #[cfg(unix)]
    #[test]
    fn output_symlinks_cannot_redirect_archive_writes() {
        let temp = TempDir::new();
        let output = temp.0.join("link");
        let original = temp.0.join("original");
        fs::create_dir(&original).unwrap();
        std::os::unix::fs::symlink(&original, &output).unwrap();
        assert!(Archive::create(&output, &Options::default()).is_err());
        assert_eq!(fs::read_dir(&original).unwrap().count(), 0);
    }

    #[test]
    fn an_interrupted_or_failed_write_never_marks_a_run_complete() {
        let temp = TempDir::new();
        let output = temp.0.join("incomplete");
        let mut archive = Archive::create(&output, &Options::default()).unwrap();
        assert_eq!(read(output.join("manifest.json"))["state"], "running");
        // Simulate a failed final manifest write after all report data is ready.
        fs::write(output.join("manifest.json.tmp"), "reserved").unwrap();
        let report = JsonReport {
            schema_version: crate::JSON_SCHEMA_VERSION,
            elapsed_ms: 0,
            summary: crate::summarize(&[]),
            warnings: &[],
            tests: &[],
        };
        let error = archive
            .complete(&report, &args(&[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("manifest.json.tmp"), "{error}");
        drop(archive);
        assert_eq!(read(output.join("manifest.json"))["state"], "running");
        assert!(read(output.join("report.json"))["tests"].is_array());
        assert_eq!(
            fs::read_to_string(output.join("manifest.json.tmp")).unwrap(),
            "reserved"
        );
    }

    #[test]
    fn a_passing_run_archives_its_report_without_failure_artifacts() {
        let temp = TempDir::new();
        let output = temp.0.join("passed");
        let root = project("examples");
        assert!(
            !crate::run(args(&[
                "--root",
                &root,
                "--match-contract",
                "ChoiceOrderTest",
                "--database",
                "off",
                "--seed",
                "17",
                "--test-cases",
                "10",
                "--output-dir",
                output.to_str().unwrap(),
                "--json",
            ]))
            .unwrap()
        );
        let manifest = read(output.join("manifest.json"));
        let report = read(output.join("report.json"));
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["archive_version"], 1);
        assert_eq!(manifest["runner_version"], env!("CARGO_PKG_VERSION"));
        assert!(!manifest["engine_version"].as_str().unwrap().is_empty());
        assert_eq!(manifest["protocol_version"], protocol::PROTOCOL_VERSION);
        assert_eq!(manifest["project_root"], root);
        assert_eq!(manifest["options"]["seed"], 17);
        assert_eq!(manifest["options"]["database"], "");
        assert_eq!(manifest["report"], "report.json");
        assert_eq!(manifest["failures"], serde_json::json!([]));
        assert!(!output.join("failures").exists());
        assert_eq!(report["summary"]["passed"], 1);
        assert_eq!(
            report["tests"][0]["test"],
            "ChoiceOrderTest.test_affordable_order"
        );
        assert!(
            report["tests"][0]["statistics"]["examples"]["valid"]
                .as_u64()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn archived_failures_keep_the_configuration_and_replay_from_the_saved_blob() {
        let temp = TempDir::new();
        let output = temp.0.join("failed");
        let replay_output = temp.0.join("replayed");
        let root = project("examples");
        let common = [
            "--root",
            &root,
            "--match-contract",
            "ERC20SpikeTest",
            "--database",
            "off",
            "--seed",
            "17",
            "--actors",
            "5",
            "--max-array-len",
            "3",
            "--max-byte-len",
            "12",
            "--step-count",
            "7",
        ];
        let mut first = args(&common);
        first.output_dir = Some(output.clone());
        assert!(crate::run(first).unwrap());
        let manifest = read(output.join("manifest.json"));
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["options"]["actors"], 5);
        assert_eq!(manifest["options"]["max_array_len"], 3);
        assert_eq!(manifest["options"]["max_byte_len"], 12);
        assert_eq!(manifest["options"]["step_count"], 7);
        assert_eq!(
            manifest["failures"],
            serde_json::json!(["failures/0001.json"])
        );
        let failure = read(output.join("failures/0001.json"));
        let report = read(output.join("report.json"));
        assert_eq!(failure["report"], report["tests"][0]);
        let command = failure["reproduce_command"].as_str().unwrap();
        assert!(command.starts_with(&format!(
            "cd {} && ",
            shell_quote(&env::current_dir().unwrap().to_string_lossy())
        )));
        for flag in [
            "--actors 5",
            "--max-array-len 3",
            "--max-byte-len 12",
            "--step-count 7",
            "--match-test test_transfer_preserves_supply",
        ] {
            assert!(command.contains(flag), "{command}");
        }
        assert!(!command.contains("--output-dir"));
        let mut replay = args(&common);
        replay.reproduce = Some(failure["report"]["blob"].as_str().unwrap().to_owned());
        replay.output_dir = Some(replay_output.clone());
        assert!(crate::run(replay).unwrap());
        let replayed = read(replay_output.join("report.json"));
        assert_eq!(replayed["tests"][0]["trace"], report["tests"][0]["trace"]);
        assert_eq!(replayed["tests"][0]["origin"], report["tests"][0]["origin"]);
    }

    #[test]
    fn a_run_error_is_archived_without_inventing_a_property_report() {
        let temp = TempDir::new();
        let output = temp.0.join("error");
        let root = project("crates/tests-integration/projects/broken");
        let error = crate::run(args(&[
            "--root",
            &root,
            "--match-contract",
            "BrokenSetupTest",
            "--database",
            "off",
            "--output-dir",
            output.to_str().unwrap(),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("setUp() reverted"));
        let manifest = read(output.join("manifest.json"));
        assert_eq!(manifest["state"], "error");
        assert_eq!(manifest["error"], "error.json");
        assert!(manifest["report"].is_null());
        assert!(!output.join("report.json").exists());
        let saved = read(output.join("error.json"));
        assert!(
            saved["message"]
                .as_str()
                .unwrap()
                .contains("fixture setUp is broken")
        );
        assert!(saved["elapsed_ms"].is_number());
    }

    #[test]
    fn listing_tests_cannot_accidentally_create_a_run_archive() {
        assert!(
            crate::Cli::try_parse_from(["hegel-sol", "test", "--list", "--output-dir", "unused",])
                .is_err()
        );
    }
}
