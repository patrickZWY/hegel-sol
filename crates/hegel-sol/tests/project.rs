mod support;

use hegel_sol::runner::{Options, run_project};
use support::{fixtures, options};

/// setUp runs before every example and does not depend on the draws, so a revert
/// in it is a broken fixture. Reporting it as a test failure would blame the
/// property for the fixture's bug, and would do so once per generated example.
#[test]
fn a_reverting_setup_is_reported_as_a_broken_fixture_not_a_failed_property() {
    let error = run_project(&Options {
        test_cases: 5,
        ..options(fixtures(), "BrokenSetupTest")
    })
    .unwrap_err()
    .to_string();

    assert!(error.contains("setUp() reverted"), "{error}");
    assert!(
        error.contains("fixture setUp is broken"),
        "the revert reason must survive, got {error}"
    );
}

/// Projects keep their own copy of Hegel.sol, so it drifts. Catching the skew up
/// front turns an unknown-selector failure raised mid-test into one message
/// naming the file to update.
#[test]
fn a_contract_built_against_another_protocol_version_is_refused_before_running() {
    let error = run_project(&Options {
        test_cases: 5,
        ..options(fixtures(), "StaleProtocolTest")
    })
    .unwrap_err()
    .to_string();

    assert!(error.contains("protocol version 99"), "{error}");
    assert!(error.contains("Hegel.sol"), "{error}");
}
