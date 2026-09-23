use std::collections::BTreeSet;

use hegel_sol::runner::{Options, Shard, list_project, run_project};
use tests_integration::{behaviour, options, statuses};

/// Sharding is a discovery-time decision, so it is observable without running
/// anything. Listing also has to agree with what a real run would select.
#[test]
fn shards_partition_the_same_tests_a_run_would_select() {
    let base = options(behaviour(), "ScalingTest");
    let all: BTreeSet<String> = list_project(&base).unwrap().into_iter().collect();

    let shards: Vec<BTreeSet<String>> = (1..=2)
        .map(|number| {
            list_project(&Options {
                shard: Some(Shard::new(number, 2).unwrap()),
                ..base.clone()
            })
            .unwrap()
            .into_iter()
            .collect()
        })
        .collect();

    assert!(
        shards[0].is_disjoint(&shards[1]),
        "a test must run in exactly one shard"
    );
    assert_eq!(
        &shards[0] | &shards[1],
        all,
        "the shards together must leave nothing unrun"
    );

    let executed: BTreeSet<String> = run_project(&base)
        .unwrap()
        .reports
        .into_iter()
        .map(|report| report.test)
        .collect();
    assert_eq!(executed, all, "listing must agree with execution");
}

/// The report is the runner's output, and it has to read the same whether the
/// work was done on one thread or several.
#[test]
fn parallel_execution_reports_the_same_results_in_the_same_order() {
    let sequential = run_project(&options(behaviour(), "ScalingTest")).unwrap();
    let parallel = run_project(&Options {
        jobs: 3,
        ..options(behaviour(), "ScalingTest")
    })
    .unwrap();

    assert_eq!(statuses(&sequential), statuses(&parallel));
    assert!(!sequential.failed());
}

/// Dynamic arguments are generated within bounds the run configures; the
/// contract asserts the same bounds from the inside.
#[test]
fn generation_stays_within_the_configured_limits() {
    let execution = run_project(&Options {
        test_cases: 50,
        max_array_len: 2,
        max_byte_len: 4,
        ..options(behaviour(), "GenerationLimitsTest")
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [("GenerationLimitsTest.testFuzz_configured_limits", "passed")]
    );
}

/// Fail-fast has to stop the run, not merely mark it failed, or a long suite
/// keeps burning time after the answer is already known.
#[test]
fn fail_fast_stops_before_running_the_tests_after_the_failure() {
    let base = options(behaviour(), "FailFastTest");
    assert_eq!(
        list_project(&base).unwrap().len(),
        3,
        "the fixture must have tests left to skip"
    );

    let execution = run_project(&Options {
        test_cases: 1,
        fail_fast: true,
        ..base
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [("FailFastTest.test_00_fails", "failed")]
    );
}

/// `stepCount()` is contract-supplied and need not be a view, so probing it must
/// not leave state behind for the tests that follow.
#[test]
fn probing_step_count_does_not_mutate_the_shared_snapshot() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(behaviour(), "StepCountIsolationTest")
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [
            (
                "StepCountIsolationTest.test_probe_does_not_mutate_base",
                "passed"
            ),
            ("StepCountIsolationTest.stateful", "passed"),
        ]
    );
}
