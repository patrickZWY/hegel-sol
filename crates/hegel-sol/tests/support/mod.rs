//! Shared helpers. Each integration-test binary compiles this module in full,
//! so anything one binary does not use is dead code there.
#![allow(dead_code)]

use std::path::PathBuf;

use hegel_sol::runner::{Execution, Options, TestReport, TestStatus};

/// The showcase project: contracts a reader is meant to look at.
pub fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

/// Contracts exercising how the runner reacts to broken projects. Several abort
/// a run by design, so they are kept out of the showcase suite.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Options for a deterministic run against one contract.
///
/// The example database is disabled so a run never reads a failure an earlier
/// run stored, which would make results depend on test order.
pub fn options(root: PathBuf, contract: &str) -> Options {
    Options {
        root,
        match_contract: Some(contract.into()),
        seed: Some(1),
        database: Some(String::new()),
        verbosity: 1,
        ..Options::default()
    }
}

pub fn statuses(execution: &Execution) -> Vec<(&str, &'static str)> {
    execution
        .reports
        .iter()
        .map(|report| (report.test.as_str(), status_name(&report.status)))
        .collect()
}

pub fn status_name(status: &TestStatus) -> &'static str {
    match status {
        TestStatus::Passed => "passed",
        TestStatus::Failed => "failed",
        TestStatus::Invalid => "invalid",
        TestStatus::Overrun => "overrun",
    }
}

pub fn only(execution: &Execution) -> &TestReport {
    assert_eq!(
        execution.reports.len(),
        1,
        "expected one report, got {:?}",
        statuses(execution)
    );
    &execution.reports[0]
}

/// Rule names in the order the reported counterexample calls them.
pub fn rules(report: &TestReport) -> Vec<&str> {
    report
        .trace
        .iter()
        .filter(|draw| draw.kind == "rule")
        .map(|draw| draw.name.as_str())
        .collect()
}
