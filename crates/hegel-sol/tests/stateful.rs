use std::path::PathBuf;

use hegel_sol::runner::{Options, TestStatus, run_project};

fn options(contract: &str) -> Options {
    Options {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts"),
        match_contract: Some(contract.into()),
        match_test: None,
        test_cases: 100,
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
fn stateful_bug_shrinks_to_two_rules() {
    let reports = run_project(&options("StatefulCounterTest")).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Failed));
    let rules = reports[0]
        .trace
        .iter()
        .filter(|draw| draw.kind == "rule")
        .map(|draw| draw.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rules, ["rule_mint", "rule_burn"]);
}

#[test]
fn stateful_pool_reuses_runtime_values() {
    let mut options = options("PoolStatefulTest");
    options.test_cases = 20;
    let reports = run_project(&options).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Passed));
}

#[test]
fn stateful_custom_errors_are_decoded() {
    let mut options = options("InvariantErrorTest");
    options.test_cases = 1;
    let reports = run_project(&options).unwrap();

    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Failed));
    assert_eq!(
        reports[0].origin.as_deref(),
        Some("invariant_custom_error: BrokenInvariant(7)")
    );
}
