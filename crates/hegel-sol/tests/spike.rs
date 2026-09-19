mod support;

use hegel_sol::runner::run_project;
use support::{contracts, only, options};

/// The end-to-end contract of a property runner: find the bug, reduce it to the
/// smallest input that still triggers it, and hand back something that replays.
#[test]
fn a_found_failure_shrinks_to_its_boundary_and_replays_from_the_blob() {
    let search = options(contracts(), "ERC20SpikeTest");
    let found = run_project(&search).unwrap();
    let report = only(&found);

    assert!(found.failed());
    // BuggyToken mishandles exactly `balance + 1`, and setUp mints 2, so 3 is
    // the only amount that triggers it. Anything larger means shrinking stopped
    // early.
    assert_eq!(
        report
            .trace
            .iter()
            .find(|draw| draw.name == "amount")
            .map(|draw| draw.value.as_str()),
        Some("3")
    );

    let replay = run_project(&hegel_sol::runner::Options {
        reproduce: report.blob.clone(),
        ..search
    })
    .unwrap();

    assert!(replay.failed(), "the blob must reproduce the failure");
    assert_eq!(only(&replay).trace, report.trace);
}

/// A blob records one example's choices, which mean nothing to another test.
/// Replaying it across a selection used to report whatever those draws happened
/// to produce, which read as passes.
#[test]
fn replaying_a_blob_across_several_tests_is_refused() {
    let error = run_project(&hegel_sol::runner::Options {
        reproduce: Some("AXicY2YAAi5GBjjFCKGYAQIZACk=".into()),
        ..options(contracts(), "AutoDrawTest")
    })
    .unwrap_err()
    .to_string();

    assert!(error.contains("single test"), "{error}");
}
