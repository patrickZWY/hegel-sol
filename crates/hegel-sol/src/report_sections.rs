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

/// Both live execution and offline archives use the same counterexample renderer.
pub(crate) fn test(
    report: &hegel_sol::runner::TestReport,
    show_timings: bool,
    show_statistics: bool,
    reproduce_command: Option<&str>,
) -> String {
    use hegel_sol::runner::TestStatus;
    let timing = if show_timings {
        format!(" ({} ms)", report.duration_ms)
    } else {
        String::new()
    };
    let verdict = match report.status {
        TestStatus::Passed => "PASS",
        TestStatus::Failed => "FAIL",
        TestStatus::Invalid => "INVALID",
        TestStatus::Overrun => "OVERRUN",
    };
    let mut lines = vec![format!("{verdict} {}{timing}", report.test)];
    if matches!(report.status, TestStatus::Failed) {
        if let Some(origin) = &report.origin {
            lines.push(format!("  {origin}"));
        }
        if let Some(source) = &report.source {
            lines.push(format!(
                "  at {}:{}:{}",
                source.path, source.line, source.column
            ));
        }
        if let Some(caveat) = &report.caveat {
            lines.push(format!("  caveat: {caveat}"));
        }
        if !report.trace.is_empty() {
            lines.push(counterexample(report));
        }
        lines.extend(report.console.iter().map(|line| format!("  {line}")));
        lines.extend(report.notes.iter().map(|note| format!("  note: {note}")));
        if show_statistics {
            lines.push(events(report));
        }
        if let Some(command) = reproduce_command {
            lines.push(format!("  reproduce: {command}"));
        }
    }
    if show_statistics {
        lines.push(statistics(&report.statistics));
    }
    if !report.diagnostics.is_empty() {
        lines.push(diagnostics(&report.diagnostics));
    }
    lines.join("\n")
}

fn counterexample(report: &hegel_sol::runner::TestReport) -> String {
    use hegel_sol::{inspector::Draw, runner::SECONDS_PER_BLOCK};
    let mut lines = vec!["  minimal counterexample:".to_owned()];
    let mut sender = None;
    let mut pending: Vec<&Draw> = Vec::new();
    for draw in &report.trace {
        match draw.kind.as_str() {
            "event" => {}
            "sender" => sender = Some(draw.value.as_str()),
            "rule" => {
                if let Some(sender) = sender.take() {
                    lines.push(format!("    vm.prank({sender});"));
                }
                let arguments = pending
                    .drain(..)
                    .map(|draw| draw.value.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("    {}({arguments});", draw.name));
            }
            "advance" => {
                let blocks = pending
                    .drain(..)
                    .next_back()
                    .map_or_else(|| "1".to_owned(), |draw| draw.value.clone());
                lines.push(format!("    vm.roll(block.number + {blocks}); vm.warp(block.timestamp + {blocks} * {SECONDS_PER_BLOCK});"));
            }
            _ => pending.push(draw),
        }
    }
    for draw in pending {
        lines.push(format!("    {} {} = {};", draw.kind, draw.name, draw.value));
    }
    lines.join("\n")
}

fn events(report: &hegel_sol::runner::TestReport) -> String {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for event in report.trace.iter().filter(|draw| draw.kind == "event") {
        *counts.entry(&event.name).or_default() += 1;
    }
    let mut lines = vec!["  events (reported example only):".to_owned()];
    if counts.is_empty() {
        lines.push("    no events recorded".into());
    } else {
        lines.extend(
            counts
                .into_iter()
                .map(|(event, count)| format!("    {event}: {count}")),
        );
    }
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
            code: "handler_not_called".into(),
            message: "Check setup".into(),
        }]);
        assert!(advice.contains("diagnostics:"));
        assert!(advice.contains("[handler_not_called] Check setup"));
    }
}
