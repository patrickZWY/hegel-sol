//! Text presentation of execution accounting; thresholds live in the runner.

use hegel_sol::runner::{Diagnostic, RunStatistics};

pub(crate) fn statistics(stats: &RunStatistics) -> String {
    let examples = &stats.examples;
    let mut lines = vec![
        "  execution (all phases, including shrinking and replay):".to_owned(),
        format!(
            "    examples: {} executed; {} valid; {} rejected; {} overrun; {} failed",
            examples.executed, examples.valid, examples.rejected, examples.overrun, examples.failed
        ),
    ];
    if !stats.handlers.is_empty() {
        lines.push("  handlers (normal completion does not imply a state change):".into());
        for (name, counts) in &stats.handlers {
            lines.push(format!(
                "    {name}: {} calls; {} completed; {} rejected; {} skipped reverts; {} failed; {} overrun",
                counts.calls, counts.completed, counts.rejected, counts.skipped_revert, counts.failed, counts.overrun
            ));
        }
        lines.push(format!(
            "    invariant checks: {}; block advances: {}",
            stats.invariant_checks, stats.block_advances
        ));
    }
    lines.join("\n")
}

pub(crate) fn diagnostics(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }
    let mut lines = vec!["  diagnostics:".to_owned()];
    lines.extend(
        diagnostics
            .iter()
            .map(|diagnostic| format!("    [{}] {}", diagnostic.code, diagnostic.message)),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel_sol::runner::{ExampleStatistics, HandlerStatistics};

    #[test]
    fn sections_explain_scope_and_distinguish_completed_from_skipped_calls() {
        let stats = RunStatistics {
            examples: ExampleStatistics {
                executed: 10,
                valid: 10,
                ..ExampleStatistics::default()
            },
            handlers: [(
                "withdraw".into(),
                HandlerStatistics {
                    calls: 20,
                    completed: 3,
                    skipped_revert: 17,
                    ..HandlerStatistics::default()
                },
            )]
            .into(),
            invariant_checks: 12,
            block_advances: 0,
        };
        let text = statistics(&stats);
        assert!(text.contains("all phases, including shrinking and replay"));
        assert!(text.contains("10 executed; 10 valid"));
        assert!(text.contains("normal completion does not imply a state change"));
        assert!(text.contains("withdraw: 20 calls; 3 completed; 0 rejected; 17 skipped reverts"));
        assert!(diagnostics(&[]).is_empty());
        let advice = diagnostics(&[Diagnostic {
            code: "handler_not_called",
            message: "Check setup".into(),
        }]);
        assert!(advice.contains("diagnostics:"));
        assert!(advice.contains("[handler_not_called] Check setup"));
    }
}
