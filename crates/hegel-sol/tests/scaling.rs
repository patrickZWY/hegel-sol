use std::{collections::BTreeSet, path::PathBuf};

use hegel_sol::runner::{Options, Shard, TestStatus, list_project, run_project};

fn options(contract: &str) -> Options {
    Options {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts"),
        match_contract: Some(contract.into()),
        match_test: None,
        test_cases: 10,
        seed: Some(11),
        database: Some(String::new()),
        profile: None,
        reproduce: None,
        phases: 31,
        verbosity: 1,
        step_count: 10,
        show_statistics: false,
        jobs: 1,
        shard: None,
        max_array_len: 8,
        max_byte_len: 64,
        fail_fast: false,
    }
}

fn names(reports: &[hegel_sol::runner::TestReport]) -> BTreeSet<String> {
    reports.iter().map(|report| report.test.clone()).collect()
}

#[test]
fn list_mode_reports_selected_tests_without_execution() {
    let tests = list_project(&options("ScalingTest")).unwrap();
    assert_eq!(
        tests,
        [
            "ScalingTest.testFuzz_configured_limits",
            "ScalingTest.test_parallel_a",
            "ScalingTest.test_parallel_b",
            "ScalingTest.test_parallel_c",
        ]
    );
}

#[test]
fn configured_generation_limits_are_enforced() {
    let reports = run_project(&Options {
        match_test: Some("configured_limits".into()),
        max_array_len: 2,
        max_byte_len: 4,
        ..options("ScalingTest")
    })
    .unwrap();

    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Passed));
}

#[test]
fn parallel_execution_preserves_results_and_order() {
    let mut sequential_options = options("ScalingTest");
    sequential_options.max_array_len = 2;
    sequential_options.max_byte_len = 4;
    let mut parallel_options = sequential_options.clone();
    parallel_options.jobs = 3;

    let sequential = run_project(&sequential_options).unwrap();
    let parallel = run_project(&parallel_options).unwrap();

    assert_eq!(
        sequential
            .iter()
            .map(|report| &report.test)
            .collect::<Vec<_>>(),
        parallel
            .iter()
            .map(|report| &report.test)
            .collect::<Vec<_>>()
    );
    assert!(
        parallel
            .iter()
            .all(|report| matches!(report.status, TestStatus::Passed))
    );
}

#[test]
fn fail_fast_stops_before_the_next_sequential_test() {
    let selected = list_project(&options("FailFastTest")).unwrap();
    assert_eq!(selected.len(), 3);

    let reports = run_project(&Options {
        test_cases: 1,
        fail_fast: true,
        ..options("FailFastTest")
    })
    .unwrap();

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].test, "FailFastTest.test_00_fails");
    assert!(matches!(reports[0].status, TestStatus::Failed));
}

#[test]
fn deterministic_shards_partition_the_selected_tests() {
    let all = run_project(&options("ScalingTest")).unwrap();
    let first = run_project(&Options {
        shard: Some(Shard::new(1, 2).unwrap()),
        ..options("ScalingTest")
    })
    .unwrap();
    let second = run_project(&Options {
        shard: Some(Shard::new(2, 2).unwrap()),
        ..options("ScalingTest")
    })
    .unwrap();

    assert!(names(&first).is_disjoint(&names(&second)));
    assert_eq!(
        names(&all),
        names(&first).union(&names(&second)).cloned().collect()
    );
}

#[test]
fn step_count_probe_does_not_mutate_shared_snapshot() {
    let reports = run_project(&Options {
        test_cases: 1,
        ..options("StepCountIsolationTest")
    })
    .unwrap();

    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|report| matches!(report.status, TestStatus::Passed))
    );
}
