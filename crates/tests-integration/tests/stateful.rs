use hegel_sol::runner::{Options, run_project};
use tests_integration::{behaviour, examples, only, options, rules, status_name, steps};

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

/// Whether a block boundary falls between two steps is part of the generated
/// example, so a same-block guard can be tested without a hand-written time
/// handler. A suite that never asked for it keeps a still clock.
#[test]
fn the_engine_advances_blocks_only_when_asked_and_moves_the_clock_with_them() {
    let still = run_project(&Options {
        test_cases: 50,
        ..options(behaviour(), "BlockAdvanceTest")
    })
    .unwrap();
    assert_eq!(status_name(&only(&still).status), "passed");

    let moving = run_project(&Options {
        test_cases: 50,
        max_block_advance: 2,
        ..options(behaviour(), "BlockAdvanceTest")
    })
    .unwrap();
    let report = only(&moving);

    assert!(moving.failed());
    // A drifted clock fails the require with its own message instead.
    let origin = report.origin.as_deref().unwrap_or_default();
    assert!(
        origin.starts_with("invariant_time_stands_still") && origin.ends_with("panic code 0x1"),
        "{origin}"
    );
    assert_eq!(steps(report), ["env.advance"]);
    // The one draw the step makes is how far to skip, shrunk to the minimum.
    assert_eq!(
        report
            .trace
            .iter()
            .find(|draw| draw.name == "blocks")
            .map(|draw| draw.value.as_str()),
        Some("1")
    );
}

/// Forge suites name the accounts their handlers are called from. The
/// registered list replaces the synthetic actors, exclusions apply, and a
/// prank the fixture left open changes the calls a test makes, never the
/// transaction's own sender.
#[test]
fn registered_senders_replace_the_synthetic_actors() {
    let execution = run_project(&Options {
        test_cases: 50,
        ..options(behaviour(), "TargetSendersTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed(), "{:?}", report.origin);
    let used: Vec<String> = tests_integration::senders(report)
        .iter()
        .map(|sender| sender.to_ascii_lowercase())
        .collect();
    let alice = "0x00000000000000000000000000000000000a11ce";
    let bob = "0x0000000000000000000000000000000000000b0b";
    assert!(
        used.iter().all(|sender| sender == alice || sender == bob),
        "{used:?}"
    );
    assert!(
        used.iter().any(|sender| sender == alice) && used.iter().any(|sender| sender == bob),
        "{used:?}"
    );
}

/// The README's attack example: an attacker's options as handlers, the vault's
/// solvency as the invariant, and the engine finding the manipulation without
/// being told its shape.
#[test]
fn a_price_manipulation_shrinks_to_credit_swap_and_borrow() {
    let execution = run_project(&Options {
        test_cases: 300,
        ..options(examples(), "PriceManipulationTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed(), "{:?}", report.origin);
    assert!(
        report
            .origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("invariant_loans_are_backed")),
        "{:?}",
        report.origin
    );
    // Capital, a swap to move the price, and a loan against the moved price:
    // nothing shorter breaks the threshold, and repaying or swapping back is
    // irrelevant to the vault's exposure.
    assert_eq!(
        steps(report),
        ["rule_takeCredit", "rule_swapBForA", "rule_borrow"]
    );
}

/// A same-block guard forces the manipulation to straddle a block. The engine
/// decides where a block boundary falls, so the exploit is found with the
/// boundary in the one place it has to be.
#[test]
fn a_same_block_guard_is_bypassed_by_a_block_the_engine_inserts() {
    let execution = run_project(&Options {
        test_cases: 300,
        ..options(examples(), "SameBlockGuardTest")
    })
    .unwrap();
    let report = only(&execution);

    assert!(execution.failed(), "{:?}", report.origin);
    assert_eq!(
        steps(report),
        [
            "rule_takeCredit",
            "rule_swapBForA",
            "env.advance",
            "rule_borrow"
        ]
    );
}
