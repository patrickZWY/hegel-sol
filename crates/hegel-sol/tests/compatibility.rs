use std::path::PathBuf;

use hegel_sol::runner::{Options, TestStatus, run_project};

fn options(contract: &str) -> Options {
    Options {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts"),
        match_contract: Some(contract.into()),
        match_test: None,
        test_cases: 1,
        seed: Some(1),
        database: Some(String::new()),
        profile: None,
        reproduce: None,
        phases: 31,
        verbosity: 1,
        step_count: 50,
        show_statistics: false,
        jobs: 1,
        shard: None,
        max_array_len: 8,
        max_byte_len: 64,
        fail_fast: false,
    }
}

#[test]
fn foundry_cheatcodes_execute() {
    let reports = run_project(&options("CompatibilityTest")).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Passed));
}

#[test]
fn custom_errors_and_console_are_reported() {
    let reports = run_project(&options("CustomErrorTest")).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Failed));
    assert!(
        reports[0]
            .origin
            .as_deref()
            .unwrap()
            .starts_with("WrongValue(7,")
    );
    assert_eq!(reports[0].console, ["console.log: about to fail"]);
}
