use hegel_sol::runner::{Options, list_project, run_project};
use tests_integration::{behaviour, examples, only, options, statuses};

/// Unmodified forge-std tests reach the runner through Foundry's cheatcode
/// address, so the emulated subset has to behave as those tests expect.
#[test]
fn the_emulated_foundry_cheatcodes_behave_as_forge_tests_expect() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(examples(), "CompatibilityTest")
    })
    .unwrap();

    assert!(
        !execution.failed(),
        "{:?}",
        only(&execution).origin.as_deref()
    );
}

/// A failure report is only actionable if it names the error and points at the
/// line that raised it.
#[test]
fn a_failure_is_reported_with_its_decoded_error_and_source_line() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(examples(), "CustomErrorTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed());
    assert!(
        report
            .origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("WrongValue(7,")),
        "custom error arguments must be decoded, got {:?}",
        report.origin
    );
    assert_eq!(report.console, ["console.log: about to fail"]);

    // Assert the reported position by reading it back, so the test tracks the
    // failing statement rather than a line number that moves whenever anything
    // above it is edited.
    let source = report.source.as_ref().expect("a mapped source location");
    assert_eq!(source.path, "test/ForgeCompatibility.t.sol");
    let line = std::fs::read_to_string(examples().join(&source.path))
        .unwrap()
        .lines()
        .nth(source.line - 1)
        .expect("the reported line exists")
        .to_owned();
    assert!(
        line.contains("revert WrongValue"),
        "reported {}:{} but that line reads {line:?}",
        source.path,
        source.line
    );
}

/// Forge suites decode the return value of a call they expected to revert, so a
/// matched expectation has to hand back data that decodes.
#[test]
fn a_matched_expected_revert_leaves_the_return_value_decodable() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(behaviour(), "ExpectRevertReturnTest")
    })
    .unwrap();

    assert!(!execution.failed(), "{:?}", statuses(&execution));
}

#[test]
fn expected_events_pass_when_emitted_and_fail_the_example_when_not() {
    let execution = run_project(&Options {
        test_cases: 20,
        ..options(behaviour(), "ExpectEmitTest")
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [
            ("ExpectEmitTest.test_matching_event_passes", "passed"),
            ("ExpectEmitTest.test_missing_event_fails", "failed"),
            ("ExpectEmitTest.test_unchecked_topics_are_ignored", "passed"),
        ]
    );
    let failed = &execution.reports[1];
    assert!(
        failed
            .origin
            .as_deref()
            .is_some_and(|origin| origin.contains("was not emitted")),
        "{:?}",
        failed.origin
    );
}

/// Permit-style tests sign with a generated key and expect the contract's
/// `ecrecover` to agree with `vm.addr`.
#[test]
fn signatures_recover_to_the_address_derived_from_the_key() {
    let execution = run_project(&Options {
        test_cases: 20,
        ..options(behaviour(), "SigningTest")
    })
    .unwrap();

    assert!(!execution.failed(), "{:?}", statuses(&execution));
}

/// A fixed test and its fuzzed overload would otherwise share a name in reports,
/// in the example database, and in a replay.
#[test]
fn overloaded_tests_are_named_by_their_signatures() {
    let listed = list_project(&options(behaviour(), "OverloadedTest")).unwrap();
    assert_eq!(
        listed,
        [
            "OverloadedTest.test_same()",
            "OverloadedTest.test_same(uint8)"
        ]
    );

    let narrowed = list_project(&Options {
        match_test: Some("test_same(uint8)".into()),
        ..options(behaviour(), "OverloadedTest")
    })
    .unwrap();
    assert_eq!(narrowed, ["OverloadedTest.test_same(uint8)"]);
}

/// forge-std's assertions call `vm.assert*` cheatcodes when a comparison fails.
#[test]
fn assertion_cheatcodes_fail_the_test_with_a_readable_message() {
    let execution = run_project(&Options {
        test_cases: 50,
        ..options(behaviour(), "VmAssertTest")
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [
            (
                "VmAssertTest.test_failing_assertion_reverts_with_its_message",
                "failed"
            ),
            ("VmAssertTest.test_passing_assertions_are_silent", "passed"),
        ]
    );
    assert_eq!(
        execution.reports[0].origin.as_deref(),
        Some("revert: assertion failed: 0 != 7")
    );
}

/// Classic DSTest marks a failure in storage and returns normally, so the runner
/// has to ask `failed()` afterwards and surface the logged explanation.
#[test]
fn a_recorded_dstest_failure_is_reported_with_its_logs() {
    let execution = run_project(&Options {
        test_cases: 50,
        ..options(behaviour(), "DSTestStyleTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed());
    assert!(
        report
            .origin
            .as_deref()
            .is_some_and(|origin| origin.contains("without reverting")),
        "{:?}",
        report.origin
    );
    assert_eq!(
        report.console,
        ["log: Error: value == 7 not satisfied", "log:   Actual: 0"]
    );
}

/// Suites read the block back through the cheatcode interface as well as from
/// the EVM, and the two must agree after `roll` and `warp`.
#[test]
fn block_reads_through_the_cheatcode_interface_follow_roll_and_warp() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(behaviour(), "BlockReadTest")
    })
    .unwrap();

    assert!(
        !execution.failed(),
        "{:?}",
        only(&execution).origin.as_deref()
    );
}
