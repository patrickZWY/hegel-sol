//! Execution accounting and actionable diagnostics, independent of presentation.
//!
//! These count observed executions across all engine phases, not unique inputs.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::engine::Status;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExampleStatistics {
    pub executed: u64,
    pub valid: u64,
    pub rejected: u64,
    pub overrun: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HandlerStatistics {
    pub calls: u64,
    pub completed: u64,
    pub rejected: u64,
    pub skipped_revert: u64,
    pub failed: u64,
    pub overrun: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunStatistics {
    pub examples: ExampleStatistics,
    pub handlers: BTreeMap<String, HandlerStatistics>,
    pub invariant_checks: u64,
    pub block_advances: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
}

pub(super) enum HandlerOutcome {
    Completed,
    Rejected,
    SkippedRevert,
    Failed,
    Overrun,
}

impl RunStatistics {
    pub(super) fn handler(&mut self, name: &str, outcome: HandlerOutcome) {
        let counts = self.handlers.entry(name.to_owned()).or_default();
        counts.calls += 1;
        match outcome {
            HandlerOutcome::Completed => counts.completed += 1,
            HandlerOutcome::Rejected => counts.rejected += 1,
            HandlerOutcome::SkippedRevert => counts.skipped_revert += 1,
            HandlerOutcome::Failed => counts.failed += 1,
            HandlerOutcome::Overrun => counts.overrun += 1,
        }
    }

    pub(super) fn observe(&mut self, status: Status, activity: &Self) {
        self.examples.executed += 1;
        match status {
            Status::Valid => self.examples.valid += 1,
            Status::Invalid => self.examples.rejected += 1,
            Status::Overrun => self.examples.overrun += 1,
            Status::Interesting => self.examples.failed += 1,
        }
        self.invariant_checks += activity.invariant_checks;
        self.block_advances += activity.block_advances;
        for (name, other) in &activity.handlers {
            let counts = self.handlers.entry(name.clone()).or_default();
            counts.calls += other.calls;
            counts.completed += other.completed;
            counts.rejected += other.rejected;
            counts.skipped_revert += other.skipped_revert;
            counts.failed += other.failed;
            counts.overrun += other.overrun;
        }
    }

    pub(super) fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut add = |code, message| diagnostics.push(Diagnostic { code, message });
        let examples = &self.examples;
        if examples.valid == 0 && examples.failed == 0 {
            add(
                "no_completed_examples",
                format!(
                    "No example completed with a property verdict ({} executed, {} rejected, {} overrun). Check --test-cases, enabled --phases, and assume preconditions; draw valid inputs directly where possible.",
                    examples.executed, examples.rejected, examples.overrun
                ),
            );
        } else if mostly_rejected(examples.rejected, examples.executed) {
            add(
                "high_example_rejection",
                format!(
                    "{} of {} executions were rejected. Prefer bounded draws or pick over generating inputs and discarding them with assume.",
                    examples.rejected, examples.executed
                ),
            );
        }
        for (name, counts) in &self.handlers {
            if counts.calls == 0 {
                add(
                    "handler_not_called",
                    format!(
                        "{name} was never called. Check setup, invariant preconditions, --step-count, and argument generation."
                    ),
                );
            } else if counts.completed == 0 && counts.failed == 0 {
                add(
                    "handler_never_completed",
                    format!(
                        "{name} never completed normally ({} calls, {} assumption rejections, {} skipped reverts, {} overruns). Check handler preconditions and generate callable arguments; skipped steps do not exercise the intended transition.",
                        counts.calls, counts.rejected, counts.skipped_revert, counts.overrun
                    ),
                );
            } else if mostly_rejected(counts.rejected + counts.skipped_revert, counts.calls) {
                add(
                    "high_handler_rejection",
                    format!(
                        "{name} skipped {} of {} calls ({} assumption rejections, {} reverts). Generate arguments from current state or adjust the handler's preconditions.",
                        counts.rejected + counts.skipped_revert,
                        counts.calls,
                        counts.rejected,
                        counts.skipped_revert
                    ),
                );
            }
        }
        diagnostics
    }

    pub(super) fn error_context(&self) -> String {
        let examples = &self.examples;
        let mut context = format!(
            "executions: {} total, {} valid, {} rejected, {} overrun, {} failed (all phases)",
            examples.executed, examples.valid, examples.rejected, examples.overrun, examples.failed
        );
        for diagnostic in self.diagnostics() {
            context.push_str(&format!("\n{}: {}", diagnostic.code, diagnostic.message));
        }
        // Engine health checks can abort before our advisory thresholds apply.
        if examples.rejected > 0 {
            context.push_str("\nFor rejected inputs, prefer bounded draws or pick over broad generation followed by assume.");
        }
        context
    }
}

// Avoid alarming on a handful of draws or dividing by zero. Use a widened
// product so the threshold cannot overflow for long-running campaigns.
fn mostly_rejected(rejected: u64, total: u64) -> bool {
    total >= 100 && u128::from(rejected) * 10 >= u128::from(total) * 9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_advice_needs_both_enough_executions_and_a_high_rejection_rate() {
        for (executed, rejected, warns) in [(99, 98, false), (100, 89, false), (100, 90, true)] {
            let stats = RunStatistics {
                examples: ExampleStatistics {
                    executed,
                    rejected,
                    valid: executed - rejected,
                    ..ExampleStatistics::default()
                },
                ..RunStatistics::default()
            };
            assert_eq!(
                stats
                    .diagnostics()
                    .iter()
                    .any(|d| d.code == "high_example_rejection"),
                warns
            );
        }
    }

    #[test]
    fn a_found_failure_is_a_verdict_even_when_no_example_passes() {
        let stats = RunStatistics {
            examples: ExampleStatistics {
                executed: 1,
                failed: 1,
                ..ExampleStatistics::default()
            },
            ..RunStatistics::default()
        };
        assert!(stats.diagnostics().is_empty());
        assert_eq!(
            RunStatistics::default().diagnostics()[0].code,
            "no_completed_examples"
        );
    }
}
