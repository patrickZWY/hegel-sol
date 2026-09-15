use std::path::PathBuf;

use hegel_sol::runner::{Options, TestStatus, run_project};

#[test]
fn solidity_spike_shrinks_and_replays() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let options = Options {
        root,
        match_contract: Some("ERC20SpikeTest".into()),
        match_test: Some("transfer".into()),
        test_cases: 50,
        seed: Some(1),
        database: Some(String::new()),
        profile: None,
        reproduce: None,
        phases: 31,
        verbosity: 1,
    };
    let reports = run_project(&options).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(matches!(reports[0].status, TestStatus::Failed));
    assert_eq!(reports[0].trace.last().unwrap().value, "3");
    let blob = reports[0]
        .blob
        .clone()
        .expect("failure must have a reproduce blob");

    let replay = run_project(&Options {
        reproduce: Some(blob),
        ..options
    })
    .unwrap();
    assert!(matches!(replay[0].status, TestStatus::Failed));
    assert_eq!(replay[0].trace.last().unwrap().value, "3");
}
