use hegel_sol::runner::{Options, run_project};
use tests_integration::{behaviour, examples, only, options, statuses};

/// A Forge fuzz test declares its inputs in its signature and never calls Hegel.
/// The runner has to build calldata Solidity accepts for every ABI shape, or the
/// body is never reached at all.
#[test]
fn abi_declared_arguments_are_generated_for_every_supported_shape() {
    let execution = run_project(&Options {
        test_cases: 25,
        seed: Some(7),
        ..options(examples(), "AutoDrawTest")
    })
    .unwrap();

    assert_eq!(
        statuses(&execution),
        [
            ("AutoDrawTest.testFuzz_arrays", "passed"),
            ("AutoDrawTest.testFuzz_scalars", "passed"),
            ("AutoDrawTest.testFuzz_tuple", "passed"),
        ]
    );
}

/// Every generated argument is reported under its Solidity name and type, which
/// is what makes a counterexample something a reader can act on.
#[test]
fn generated_arguments_are_traced_under_their_declared_names_and_types() {
    let execution = run_project(&Options {
        test_cases: 5,
        ..options(behaviour(), "TracedArgumentsTest")
    })
    .unwrap();

    let traced: Vec<(&str, &str)> = only(&execution)
        .trace
        .iter()
        .map(|draw| (draw.name.as_str(), draw.kind.as_str()))
        .collect();
    assert_eq!(
        traced,
        [
            ("amount", "uint8"),
            ("account", "address"),
            ("salt", "bytes32"),
        ]
    );
}
