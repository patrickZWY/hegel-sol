//! Naming conventions that decide what the runner treats as a test.
//!
//! The mapping from Solidity function names to roles is a convention, not a
//! language feature, and different suites use different ones. Holding it in a
//! value rather than in scattered string literals keeps it configurable and gives
//! the conventions a single place to change.

/// Prefixes identifying each kind of function the runner looks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conventions {
    /// Stateless property tests, run once per generated example.
    pub test: String,
    /// Stateful handlers the state machine may call.
    pub rule: String,
    /// Properties checked between handler calls.
    pub invariant: String,
    /// Fixture run before each generated example.
    pub setup: String,
    /// Optional view returning a per-contract state-machine depth.
    pub step_count: String,
}

impl Default for Conventions {
    fn default() -> Self {
        Self {
            test: "test".into(),
            rule: "rule_".into(),
            invariant: "invariant_".into(),
            setup: "setUp".into(),
            step_count: "stepCount".into(),
        }
    }
}

/// Name the runner reports for a contract's state machine. Rules and invariants
/// are collective, so the machine is addressed by contract rather than by
/// function.
pub const STATEFUL_SUFFIX: &str = "stateful";
