use hegel_sol::runner::{Options, run_project};
use tests_integration::{behaviour, examples, only, options, rules, status_name};

/// Spanning each handler call lets the shrinker delete whole steps, not just
/// simplify their arguments. Without that, a bug found deep in a long sequence
/// is reported as the long sequence.
#[test]
fn a_stateful_bug_shrinks_to_the_two_calls_that_cause_it() {
    // A burn of exactly one is a narrow target for a random handler sequence,
    // so the budget is generous enough that a reshuffle of the engine's
    // generator still reaches it under the pinned seed.
    let execution = run_project(&Options {
        test_cases: 300,
        ..options(examples(), "StatefulCounterTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed());
    // StatefulCounterTest forgets to account for a burn of exactly 1, which
    // needs one mint to make the balance available and one burn to trigger.
    assert_eq!(rules(report), ["rule_mint", "rule_burn"]);
}

/// A pool hands a handler a value some earlier handler created. The engine
/// tracks these by variable id while the runner holds the values, so the two
/// must stay in step across shrinking and replay.
#[test]
fn pooled_values_reach_handlers_that_pick_them() {
    let execution = run_project(&Options {
        test_cases: 20,
        ..options(examples(), "PoolStatefulTest")
    })
    .unwrap();

    assert_eq!(status_name(&only(&execution).status), "passed");
}

#[test]
fn a_custom_error_from_an_invariant_is_decoded_and_attributed() {
    let execution = run_project(&Options {
        test_cases: 1,
        ..options(behaviour(), "InvariantErrorTest")
    })
    .unwrap();

    assert_eq!(
        only(&execution).origin.as_deref(),
        Some("invariant_custom_error: BrokenInvariant(7)")
    );
}

/// Real handlers guard their preconditions by reverting. Whether that ends the
/// run is policy: Foundry's invariant runner ignores such reverts by default,
/// and a suite ported from it needs the same option.
#[test]
fn a_reverting_handler_fails_by_default_and_is_skippable_on_request() {
    let strict = run_project(&Options {
        test_cases: 10,
        ..options(behaviour(), "RevertingRuleTest")
    })
    .unwrap();

    assert!(strict.failed());
    assert_eq!(
        only(&strict).origin.as_deref(),
        Some("rule_add_even: revert: odd amount"),
        "the failure must name the handler that reverted"
    );

    let lenient = run_project(&Options {
        test_cases: 10,
        allow_rule_reverts: true,
        ..options(behaviour(), "RevertingRuleTest")
    })
    .unwrap();

    assert!(!lenient.failed());
}

/// Handlers called from a single account cannot reach any bug that needs two
/// parties, so who makes each call is part of the generated example.
#[test]
fn handler_calls_record_which_account_made_them() {
    let execution = run_project(&Options {
        test_cases: 10,
        actors: 4,
        ..options(behaviour(), "RevertingRuleTest")
    })
    .unwrap();
    let report = only(&execution);

    let senders: Vec<&str> = report
        .trace
        .iter()
        .filter(|draw| draw.kind == "sender")
        .map(|draw| draw.value.as_str())
        .collect();
    assert_eq!(
        senders.len(),
        rules(report).len(),
        "every reported call names its sender"
    );
}

/// With one actor the sender is fixed, so recording it would be noise in every
/// counterexample.
#[test]
fn a_single_actor_run_reports_no_sender() {
    let execution = run_project(&Options {
        test_cases: 10,
        actors: 1,
        ..options(behaviour(), "RevertingRuleTest")
    })
    .unwrap();

    assert!(
        !only(&execution)
            .trace
            .iter()
            .any(|draw| draw.kind == "sender")
    );
}

/// Forge suites register handler contracts rather than writing `rule_*`
/// functions. Their state-changing functions become the rules, reverts are
/// rejected steps as under Forge, and shrinking still deletes whole calls.
#[test]
fn forge_style_targets_are_fuzzed_and_shrink_like_handlers() {
    for contract in ["ExplicitTargetTest", "DefaultTargetTest"] {
        let execution = run_project(&Options {
            test_cases: 300,
            ..options(behaviour(), contract)
        })
        .unwrap();
        let report = only(&execution);

        assert!(execution.failed(), "{contract}: {:?}", report.origin);
        // One credit of at least three and one debit of exactly three is the
        // shortest sequence that double counts; the bystander is never called.
        assert_eq!(
            rules(report),
            ["Ledger.credit", "Ledger.debit"],
            "{contract}"
        );
    }
}
