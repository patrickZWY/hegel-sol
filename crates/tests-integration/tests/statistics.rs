use hegel_sol::runner::{Options, RunStatistics, run_project};
use tests_integration::{behaviour, broken, only, options, status_name};

fn check_totals(stats: &RunStatistics) {
    let e = &stats.examples;
    assert_eq!(e.executed, e.valid + e.rejected + e.overrun + e.failed);
    assert!(e.executed > 0);
    for h in stats.handlers.values() {
        assert_eq!(
            h.calls,
            h.completed + h.rejected + h.skipped_revert + h.failed + h.overrun
        );
    }
}

#[test]
fn execution_counts_include_rejections_and_replayed_failures() {
    for test in ["test_completed", "test_filtered", "test_failure"] {
        let options = Options {
            test_cases: 50,
            match_test: Some(test.into()),
            ..options(behaviour(), "ExampleStatisticsTest")
        };
        let execution = run_project(&options).unwrap();
        let report = only(&execution);
        check_totals(&report.statistics);
        assert!(report.statistics.handlers.is_empty());
        if test == "test_failure" {
            assert!(execution.failed());
            assert!(report.statistics.examples.failed > 0);
            let replay = run_project(&Options {
                reproduce: Some(report.blob.clone().unwrap()),
                ..options
            })
            .unwrap();
            let replayed = only(&replay);
            assert!(replay.failed());
            assert_eq!(replayed.trace, report.trace);
            assert!(replayed.statistics.examples.failed > 0);
            check_totals(&replayed.statistics);
        } else {
            assert_eq!(status_name(&report.status), "passed");
            assert!(report.statistics.examples.valid > 0);
            assert_eq!(report.statistics.examples.failed, 0);
            if test == "test_filtered" {
                assert!(report.statistics.examples.rejected > 0);
            } else {
                assert_eq!(report.statistics.examples.rejected, 0);
                assert!(report.diagnostics.is_empty());
            }
        }
    }
}

#[test]
fn handler_counts_distinguish_assumptions_reverts_and_normal_returns() {
    let execution = run_project(&Options {
        allow_rule_reverts: true,
        test_cases: 20,
        step_count: 20,
        ..options(behaviour(), "HandlerStatisticsTest")
    })
    .unwrap();
    let report = only(&execution);
    check_totals(&report.statistics);
    assert!(report.statistics.invariant_checks > 0);
    for (name, outcome) in [
        ("rule_complete", "completed"),
        ("rule_reject", "rejected"),
        ("rule_revert", "skipped"),
    ] {
        let counts = &report.statistics.handlers[name];
        assert!(counts.calls > 0, "{name}");
        let count = match outcome {
            "completed" => counts.completed,
            "rejected" => counts.rejected,
            _ => counts.skipped_revert,
        };
        assert_eq!(counts.calls, count, "{name}");
    }
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "handler_never_completed" && d.message.contains("rule_reject"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "handler_never_completed" && d.message.contains("rule_revert"))
    );
}

#[test]
fn skipped_handlers_warn_but_normal_returns_do_not_claim_state_changes() {
    for (contract, handler, skipped) in [
        ("SkippedActionStatisticsTest", "rule_skip", true),
        ("IdleStatisticsTest", "rule_idle", false),
    ] {
        let execution = run_project(&Options {
            allow_rule_reverts: true,
            test_cases: 10,
            ..options(behaviour(), contract)
        })
        .unwrap();
        let report = only(&execution);
        check_totals(&report.statistics);
        assert_eq!(status_name(&report.status), "passed");
        let counts = &report.statistics.handlers[handler];
        assert!(counts.calls > 0);
        if skipped {
            assert_eq!(counts.calls, counts.skipped_revert);
            assert_eq!(counts.completed, 0);
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "handler_never_completed")
            );
        } else {
            assert_eq!(counts.calls, counts.completed);
            assert!(report.diagnostics.is_empty());
        }
    }
}

#[test]
fn invariant_failures_leave_uncalled_handlers_in_the_report() {
    let execution = run_project(&options(behaviour(), "UncalledStatisticsTest")).unwrap();
    let report = only(&execution);
    assert!(execution.failed());
    assert_eq!(report.statistics.handlers["rule_unreached"].calls, 0);
    assert!(report.statistics.invariant_checks > 0);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "handler_not_called")
    );
}

#[test]
fn unsatisfiable_runs_keep_their_error_and_explain_rejected_execution() {
    let error = run_project(&Options {
        test_cases: 10,
        ..options(broken(), "RejectedExamplesTest")
    })
    .unwrap_err()
    .to_string();
    assert!(error.contains("Hegel run failed"), "{error}");
    assert!(
        error.contains("RejectedExamplesTest.test_reject"),
        "{error}"
    );
    assert!(error.contains("no_completed_examples"), "{error}");
    assert!(error.contains("rejected"), "{error}");
    assert!(error.contains("assume"), "{error}");
}

#[test]
fn early_engine_filter_errors_still_offer_direct_generation_advice() {
    let error = run_project(&Options {
        test_cases: 50,
        ..options(broken(), "OverfilteredExamplesTest")
    })
    .unwrap_err()
    .to_string();
    assert!(error.contains("Hegel run failed"), "{error}");
    assert!(error.contains("executions:"), "{error}");
    assert!(error.contains("bounded draws or pick"), "{error}");
}

#[test]
fn strict_handler_failures_are_not_counted_as_skipped_reverts() {
    let execution = run_project(&Options {
        test_cases: 10,
        ..options(behaviour(), "SkippedActionStatisticsTest")
    })
    .unwrap();
    let report = only(&execution);
    assert!(execution.failed());
    check_totals(&report.statistics);
    let counts = &report.statistics.handlers["rule_skip"];
    assert!(counts.failed > 0);
    assert_eq!(counts.calls, counts.failed);
    assert_eq!(counts.skipped_revert, 0);
}
