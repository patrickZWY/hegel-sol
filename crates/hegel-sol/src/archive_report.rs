//! Read-only presentation of the archive contract, independent of project discovery.

use crate::{JSON_SCHEMA_VERSION, Summary, report_sections, summarize};
use anyhow::{Context as _, Result, bail, ensure};
use hegel_sol::runner::{TestReport, TestStatus};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(clap::Args)]
pub(crate) struct ReportArgs {
    /// Saved run directory (does not require the original project).
    directory: PathBuf,
    /// Show only failed test entries; the overview still describes the whole run.
    #[arg(long)]
    failed: bool,
    /// Show entries whose full contract.test name contains this text.
    #[arg(long)]
    match_test: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    state: String,
    runner_version: String,
    project_root: PathBuf,
    report: Option<String>,
    failures: Vec<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Report {
    elapsed_ms: u128,
    summary: Summary,
    warnings: Vec<String>,
    tests: Vec<TestReport>,
}

#[derive(Deserialize)]
struct Failure {
    report: Value,
    reproduce_command: Option<String>,
}

#[derive(Deserialize)]
struct RunError {
    message: String,
    elapsed_ms: u128,
}

pub(crate) fn run(args: ReportArgs) -> Result<()> {
    println!("{}", render(&args)?);
    Ok(())
}

fn read<T: DeserializeOwned>(directory: &Path, name: &str) -> Result<T> {
    let path = directory.join(name);
    ensure!(
        Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "invalid archive artifact path: {}",
        path.display()
    );
    let resolved = fs::canonicalize(&path)
        .with_context(|| format!("archive is incomplete: cannot read {}", path.display()))?;
    ensure!(
        resolved.starts_with(directory),
        "archive artifact escapes directory: {}",
        path.display()
    );
    let bytes = fs::read(&resolved).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid archive JSON in {}", path.display()))
}

fn version(value: &Value, field: &str, expected: u32, path: &Path) -> Result<()> {
    ensure!(
        value[field].as_u64() == Some(u64::from(expected)),
        "unsupported or missing {field} in {}: {}; expected {expected}",
        path.display(),
        value[field]
    );
    Ok(())
}

fn render(args: &ReportArgs) -> Result<String> {
    let directory = fs::canonicalize(&args.directory)
        .with_context(|| format!("cannot open archive {}", args.directory.display()))?;
    let manifest: Value = read(&directory, "manifest.json")?;
    version(
        &manifest,
        "archive_version",
        1,
        &directory.join("manifest.json"),
    )?;
    let manifest: Manifest = serde_json::from_value(manifest).with_context(|| {
        format!(
            "invalid archive manifest {}",
            directory.join("manifest.json").display()
        )
    })?;
    let mut lines = vec![
        format!("Archive: {}", directory.display()),
        format!("State: {}", manifest.state),
        format!("Project: {}", manifest.project_root.display()),
        format!("Runner: {}", manifest.runner_version),
    ];
    match manifest.state.as_str() {
        "running" => bail!(
            "archive {} is incomplete (state: running); no final report is available",
            directory.display()
        ),
        "error" => {
            ensure!(
                manifest.error.as_deref() == Some("error.json")
                    && manifest.report.is_none()
                    && manifest.failures.is_empty(),
                "inconsistent error state in {}",
                directory.join("manifest.json").display()
            );
            let error: RunError = read(&directory, "error.json")?;
            lines.push(format!(
                "Run error ({} ms): {}",
                error.elapsed_ms, error.message
            ));
        }
        "complete" => {
            ensure!(
                manifest.report.as_deref() == Some("report.json") && manifest.error.is_none(),
                "inconsistent complete state in {}",
                directory.join("manifest.json").display()
            );
            let value: Value = read(&directory, "report.json")?;
            version(
                &value,
                "schema_version",
                JSON_SCHEMA_VERSION,
                &directory.join("report.json"),
            )?;
            let report: Report = serde_json::from_value(value.clone()).with_context(|| {
                format!("invalid report {}", directory.join("report.json").display())
            })?;
            ensure!(
                report.summary == summarize(&report.tests),
                "summary does not match tests in {}",
                directory.join("report.json").display()
            );
            let mut commands = vec![None; report.tests.len()];
            let mut failures = 0;
            for (index, test) in report.tests.iter().enumerate() {
                if !matches!(test.status, TestStatus::Failed) {
                    continue;
                }
                let expected = format!("failures/{:04}.json", failures + 1);
                ensure!(
                    manifest.failures.get(failures) == Some(&expected),
                    "missing or out-of-order {expected} in {}",
                    directory.join("manifest.json").display()
                );
                let failure: Failure = read(&directory, &expected)?;
                ensure!(
                    failure.report == value["tests"][index],
                    "failure report disagrees with report.json: {}",
                    directory.join(&expected).display()
                );
                commands[index] = failure.reproduce_command;
                failures += 1;
            }
            ensure!(
                failures == manifest.failures.len(),
                "unexpected failure artifacts in {}",
                directory.join("manifest.json").display()
            );
            let Summary {
                passed,
                failed,
                invalid,
                overrun,
            } = report.summary;
            lines.push(format!("Overview: {passed} passed; {failed} failed; {invalid} invalid; {overrun} overrun; {} ms", report.elapsed_ms));
            for warning in report.warnings {
                lines.push(format!("Warning: {warning}"));
            }
            let selected: Vec<_> = report
                .tests
                .iter()
                .enumerate()
                .filter(|(_, test)| {
                    (!args.failed || matches!(test.status, TestStatus::Failed))
                        && args
                            .match_test
                            .as_ref()
                            .is_none_or(|name| test.test.contains(name))
                })
                .collect();
            lines.push(format!(
                "Showing {} of {} entries",
                selected.len(),
                report.tests.len()
            ));
            if selected.is_empty() {
                lines.push("No matching test entries.".into());
            }
            for (index, test) in selected {
                lines.push(String::new());
                lines.push(report_sections::test(
                    test,
                    true,
                    true,
                    commands[index].as_deref(),
                ));
            }
        }
        state => bail!(
            "unknown archive state {state:?} in {}",
            directory.join("manifest.json").display()
        ),
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "hegel-offline-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            let fixture = Self(path);
            fixture.write(
                "manifest.json",
                json!({
                    "archive_version": 1, "state": "complete", "runner_version": "0.2.0",
                    "project_root": "/project/no/longer/exists", "report": "report.json",
                    "failures": ["failures/0001.json"], "error": null,
                    "future_field": true
                }),
            );
            let failed = serde_json::to_value(crate::tests::populated_report()).unwrap();
            let mut passed = failed.clone();
            passed["test"] = json!("Other.test_pass");
            passed["status"] = json!("passed");
            fixture.write(
                "report.json",
                json!({
                    "schema_version": 1, "elapsed_ms": 15,
                    "summary": {"passed": 1, "failed": 1, "invalid": 0, "overrun": 0},
                    "warnings": ["saved warning"], "tests": [failed, passed]
                }),
            );
            fixture.write("failures/0001.json", json!({"report": failed, "reproduce_command": "cd /saved/project && saved-command"}));
            fixture
        }
        fn write(&self, name: &str, value: Value) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        fn edit(&self, name: &str, edit: impl FnOnce(&mut Value)) {
            let mut value: Value =
                serde_json::from_slice(&fs::read(self.0.join(name)).unwrap()).unwrap();
            edit(&mut value);
            self.write(name, value);
        }
        fn args(&self) -> ReportArgs {
            ReportArgs {
                directory: self.0.clone(),
                failed: false,
                match_test: None,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn offline_view_preserves_overview_and_saved_replay_when_filtered() {
        let fixture = Fixture::new();
        let before = fs::read(fixture.0.join("manifest.json")).unwrap();
        let mut args = fixture.args();
        args.failed = true;
        let text = render(&args).unwrap();
        assert!(text.contains("State: complete"));
        assert!(text.contains("1 passed; 1 failed"));
        assert!(text.contains("Showing 1 of 2 entries"));
        assert!(text.contains("uint256 amount = 3;"));
        assert!(text.contains("cd /saved/project && saved-command"));
        assert!(text.contains("execution (all phases"));
        assert!(text.contains("handler_not_called"));
        assert!(text.contains("saved warning"));
        assert!(!text.contains("Other.test_pass"));
        args.match_test = Some("missing".into());
        assert!(render(&args).unwrap().contains("No matching test entries"));
        assert_eq!(before, fs::read(fixture.0.join("manifest.json")).unwrap());
    }

    #[test]
    fn incomplete_and_future_archives_are_not_presented_as_complete() {
        let fixture = Fixture::new();
        fixture.edit("manifest.json", |m| m["state"] = json!("running"));
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("incomplete (state: running)")
        );
        fixture.edit("manifest.json", |m| m["archive_version"] = json!(2));
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("archive_version")
        );
        fixture.edit("manifest.json", |m| {
            m["archive_version"] = json!(1);
            m["state"] = json!("complete");
        });
        fixture.edit("report.json", |r| r["schema_version"] = json!(2));
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("schema_version")
        );
    }

    #[test]
    fn all_artifacts_are_checked_even_when_no_tests_match() {
        let fixture = Fixture::new();
        let mut args = fixture.args();
        args.match_test = Some("missing".into());
        fixture.edit("failures/0001.json", |f| {
            f["report"]["test"] = json!("wrong")
        });
        assert!(render(&args).unwrap_err().to_string().contains("disagrees"));
        fs::remove_file(fixture.0.join("failures/0001.json")).unwrap();
        assert!(
            render(&args)
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
    }

    #[test]
    fn saved_run_errors_are_successfully_inspected_without_a_property_report() {
        let fixture = Fixture::new();
        fixture.edit("manifest.json", |m| {
            m["state"] = json!("error");
            m["report"] = Value::Null;
            m["failures"] = json!([]);
            m["error"] = json!("error.json");
        });
        fixture.write(
            "error.json",
            json!({"message": "setup failed", "elapsed_ms": 10}),
        );
        let text = render(&fixture.args()).unwrap();
        assert!(text.contains("Run error (10 ms): setup failed"));
        assert!(!text.contains("passed"));
    }

    #[test]
    fn inconsistent_summary_and_truncated_json_name_the_broken_file() {
        let fixture = Fixture::new();
        fixture.edit("report.json", |r| r["summary"]["passed"] = json!(999));
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("summary does not match tests")
        );
        fs::write(fixture.0.join("report.json"), "{").unwrap();
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("invalid archive JSON in")
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifacts_cannot_escape_the_archive_through_symlinks() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        fs::remove_file(fixture.0.join("report.json")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("report.json"), fixture.0.join("report.json"))
            .unwrap();
        assert!(
            render(&fixture.args())
                .unwrap_err()
                .to_string()
                .contains("escapes directory")
        );
    }
}
