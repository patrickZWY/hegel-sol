use std::path::PathBuf;

use hegel_sol::runner::{Options, TestStatus, run_project};

#[test]
fn auto_draws_scalars_arrays_and_structs() {
    let reports = run_project(&Options {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts"),
        match_contract: Some("AutoDrawTest".into()),
        match_test: None,
        test_cases: 10,
        seed: Some(7),
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
    })
    .unwrap();
    assert_eq!(reports.len(), 3);
    assert!(
        reports
            .iter()
            .all(|report| matches!(report.status, TestStatus::Passed))
    );
}
