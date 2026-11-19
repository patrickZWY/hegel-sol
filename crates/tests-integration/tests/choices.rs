use hegel_sol::runner::{Options, list_project, run_project};
use tests_integration::{behaviour, examples, only, options, rules, statuses};

fn choice_options(test: &str) -> Options {
    Options {
        test_cases: 50,
        match_test: Some(test.into()),
        ..options(behaviour(), "ChoiceHelpersTest")
    }
}

#[test]
fn choices_shrink_by_candidate_order_and_replay_named_values() {
    for (test, name, index, note, origin) in [
        (
            "test_address_order",
            "actor.index",
            "0",
            "actor = 0x000000000000000000000000000000000000cafe",
            "ChoiceReached()",
        ),
        (
            "test_integer_boundary",
            "id.index",
            "1",
            "id = 7",
            "panic code 0x1",
        ),
        (
            "test_other_removes_every_excluded_entry",
            "recipient.index",
            "0",
            "recipient = 0x0000000000000000000000000000000000000002",
            "ChoiceReached()",
        ),
        (
            "test_other_keeps_zero_when_not_excluded",
            "recipient.index",
            "0",
            "recipient = 0x0000000000000000000000000000000000000000",
            "ChoiceReached()",
        ),
    ] {
        let options = choice_options(test);
        let found = run_project(&options).unwrap();
        let report = only(&found);
        assert!(found.failed(), "{test}");
        assert_eq!(report.origin.as_deref(), Some(origin), "{test}");
        let draw = report.trace.iter().find(|draw| draw.name == name).unwrap();
        assert_eq!(draw.value, index, "{test}");
        assert_eq!(report.notes, [note], "{test}");
        let blob = report
            .blob
            .clone()
            .expect("a found choice failure is replayable");
        let replay = run_project(&Options {
            reproduce: Some(blob),
            ..options
        })
        .unwrap();
        assert!(replay.failed(), "{test}");
        assert_eq!(only(&replay).trace, report.trace, "{test}");
        assert_eq!(only(&replay).notes, report.notes, "{test}");
        assert_eq!(only(&replay).origin, report.origin, "{test}");
    }
}

#[test]
fn singleton_choices_preserve_zero_and_full_width_values_in_notes() {
    let found = run_project(&choice_options("test_singletons")).unwrap();
    let report = only(&found);
    assert_eq!(report.origin.as_deref(), Some("ChoiceReached()"));
    assert_eq!(
        report.notes,
        [
            "zero = 0",
            "maximum = 115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "zero address = 0x0000000000000000000000000000000000000000",
            "maximum address = 0xffffffffffffffffffffffffffffffffffffffff",
        ]
    );
    assert_eq!(report.trace.len(), 4);
    assert!(report.trace.iter().all(|draw| draw.value == "0"));
}

#[test]
fn empty_choices_fail_with_the_choice_name_before_any_draw() {
    for (test, name) in [
        ("test_empty_addresses", "actors"),
        ("test_empty_integers", "objects"),
        ("test_empty_index", "orders"),
        ("test_all_excluded", "recipient"),
    ] {
        let found = run_project(&choice_options(test)).unwrap();
        let report = only(&found);
        assert!(found.failed(), "{test}");
        assert_eq!(
            report.origin.as_deref(),
            Some(format!("HegelEmptyChoice(\"{name}\")").as_str())
        );
        assert!(report.trace.is_empty(), "{test}");
    }
}

#[test]
fn stateful_choices_replay_after_the_candidate_collection_changes() {
    let options = Options {
        test_cases: 100,
        actors: 1,
        ..options(behaviour(), "ChoiceStateReplayTest")
    };
    let found = run_project(&options).unwrap();
    let report = only(&found);
    assert_eq!(
        report.origin.as_deref(),
        Some("rule_choose: ChoiceReached()")
    );
    assert_eq!(rules(report), ["rule_open", "rule_remove", "rule_choose"]);
    assert_eq!(report.notes, ["live object = 11"]);
    let replay = run_project(&Options {
        reproduce: Some(report.blob.clone().expect("stateful failures carry a blob")),
        ..options
    })
    .unwrap();
    assert!(replay.failed());
    assert_eq!(only(&replay).trace, report.trace);
    assert_eq!(only(&replay).notes, report.notes);
}

#[test]
fn choice_examples_are_discovered_and_run_as_written() {
    let options = options(examples(), "Choice");
    let listed = list_project(&options).unwrap();
    assert_eq!(
        listed,
        [
            "ChoiceObjectsTest.stateful",
            "ChoiceOrderTest.test_affordable_order",
            "ChoiceTransferTest.test_transfer",
        ]
    );
    let execution = run_project(&options).unwrap();
    assert_eq!(
        statuses(&execution),
        listed
            .iter()
            .map(|name| (name.as_str(), "passed"))
            .collect::<Vec<_>>()
    );
}
