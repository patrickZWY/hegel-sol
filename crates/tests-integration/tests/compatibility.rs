use hegel_sol::runner::{Options, run_project};
use tests_integration::{examples, only, options};

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
