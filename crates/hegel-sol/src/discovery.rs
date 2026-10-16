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
    /// Properties checked between handler calls. Forge's bare `invariant`
    /// prefix also covers the `invariant_` spelling.
    pub invariant: String,
    /// Fixture run before each generated example.
    pub setup: String,
    /// Optional view returning a per-contract state-machine depth.
    pub step_count: String,
    /// Optional view returning how many blocks the engine may skip at once
    /// between stateful steps; zero or absent keeps time still.
    pub max_block_advance: String,
}

impl Default for Conventions {
    fn default() -> Self {
        Self {
            test: "test".into(),
            rule: "rule_".into(),
            invariant: "invariant".into(),
            setup: "setUp".into(),
            step_count: "stepCount".into(),
            max_block_advance: "maxBlockAdvance".into(),
        }
    }
}

/// Name the runner reports for a contract's state machine. Rules and invariants
/// are collective, so the machine is addressed by contract rather than by
/// function.
pub const STATEFUL_SUFFIX: &str = "stateful";

/// Forge's view listing the contracts a stateful test wants fuzzed, and the one
/// listing contracts to leave alone.
pub const TARGET_CONTRACTS: &str = "targetContracts";
pub const EXCLUDE_CONTRACTS: &str = "excludeContracts";

/// Forge's views listing the accounts handler calls may come from, and the
/// ones to leave out. When neither is declared the runner's synthetic actors
/// are used.
pub const TARGET_SENDERS: &str = "targetSenders";
pub const EXCLUDE_SENDERS: &str = "excludeSenders";

/// DSTest's flag, read after a test that returned normally: a classic assertion
/// records its failure here instead of reverting.
pub const FAILED_FLAG: &str = "failed";
