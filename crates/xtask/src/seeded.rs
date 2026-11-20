//! Evidence for a deliberately seeded local defect, separate from compatibility passes.
use super::*;
use serde_json::{Value, json};
use std::time::Instant;

const CORRECTNESS_ROOT: &str = "crates/tests-integration/projects/correctness";

pub(super) fn campaign(runner: &Path, args: &Validate, output: &Path) -> Result<()> {
    fs::create_dir(output).with_context(|| {
        format!(
            "--seeded-output {} must be a new directory",
            output.display()
        )
    })?;
    let output = output.canonicalize()?;
    let cases = args.stateful_cases.to_string();
    let steps = args.stateful_steps.to_string();
    let mut rows = Vec::new();
    let mut metadata = json!({
        "cases": args.stateful_cases, "steps": args.stateful_steps,
        "seeds": [1, 17, 42, 123, 999], "rows": [], "complete": false,
    });
    for (name, program, arguments) in [
        ("revision", "git", vec!["rev-parse", "HEAD"]),
        ("worktree", "git", vec!["diff", "--stat"]),
        ("forge", "forge", vec!["--version"]),
        ("rust", "rustc", vec!["--version"]),
        ("host", "uname", vec!["-a"]),
        ("cpu", "lscpu", vec![]),
    ] {
        let result = Command::new(program).args(arguments).output()?;
        ensure!(result.status.success(), "could not record {name}");
        metadata[name] = String::from_utf8_lossy(&result.stdout).trim().into();
    }
    metadata["memory"] = fs::read_to_string("/proc/meminfo")?.into();
    let binary_hash = Command::new("sha256sum").arg(runner).output()?;
    ensure!(binary_hash.status.success(), "could not hash runner");
    metadata["runner_sha256"] = String::from_utf8_lossy(&binary_hash.stdout).trim().into();
    let sources = output.join("sources");
    fs::create_dir(&sources)?;
    for source in [
        "crates/tests-integration/projects/correctness/test/SeededInventory.t.sol",
        "crates/tests-integration/projects/correctness/foundry.toml",
        "crates/xtask/src/seeded.rs",
    ] {
        fs::copy(
            workspace().join(source),
            sources.join(Path::new(source).file_name().unwrap()),
        )?;
    }
    write_json(&output.join("campaign.json"), &metadata)?;
    // Compile once before timing either runner; measured searches use warm artifacts.
    run(forge(["build", "--root"], Path::new(CORRECTNESS_ROOT)))?;
    for seed in [1, 17, 42, 123, 999] {
        section(&format!("Seeded inventory defect: seed {seed}"));
        let seed = seed.to_string();
        let dir = output.join(&seed);
        fs::create_dir(&dir)?;
        let mut baseline = forge(["test", "--root"], Path::new(CORRECTNESS_ROOT));
        baseline.args([
            "--match-contract",
            "^SeededInventoryInvariantTest$",
            "--json",
        ]);
        baseline
            .env("FOUNDRY_INVARIANT_RUNS", &cases)
            .env("FOUNDRY_INVARIANT_DEPTH", &steps)
            .env("FOUNDRY_FUZZ_SEED", &seed)
            .env(
                "FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR",
                dir.join("forge-failures"),
            );
        let (forge_report, forge_ms, forge_exit) = capture(baseline, &dir, "forge")?;
        let result = forge_report
            .as_object()
            .context("Forge suites missing")?
            .values()
            .find_map(|suite| suite["test_results"].get("invariant_total()"))
            .context("Forge did not run invariant_total()")?;
        let forge_found = result["status"] == "Failure";
        ensure!(
            (forge_found && forge_exit == Some(1))
                || (result["status"] == "Success" && forge_exit == Some(0)),
            "unexpected Forge result: {result}"
        );
        if forge_found {
            ensure!(
                result["reason"]
                    .as_str()
                    .is_some_and(|s| s.contains("assertion")),
                "Forge failed for an unexpected reason: {result}"
            );
        }
        let mut search = property_cmd(
            runner,
            CORRECTNESS_ROOT,
            &[
                "--match-contract",
                "SeededInventoryInvariantTest",
                "--match-test",
                "stateful",
            ],
            &cases,
            Some(&steps),
            &seed,
        );
        search
            .args(["--json", "--output-dir"])
            .arg(dir.join("search"));
        search.env_remove("HEGEL_SOL_REPRODUCE");
        let (report, search_ms, exit) = capture(search, &dir, "hegel")?;
        let tests = report["tests"].as_array().context("missing hegel tests")?;
        ensure!(tests.len() == 1, "expected one inventory property");
        let test = &tests[0];
        let found = test["status"] == "failed";
        ensure!(
            (found && exit == Some(1)) || (test["status"] == "passed" && exit == Some(0)),
            "unexpected hegel result: {test}"
        );
        let mut row = json!({"seed": seed, "forge_found": forge_found,
            "hegel_found": found, "forge_process_ms": forge_ms,
            "hegel_process_ms": search_ms,
            "forge_steps": result["counterexample"]["Sequence"][1].as_array().map(Vec::len)});
        if found {
            ensure!(
                test["origin"]
                    .as_str()
                    .is_some_and(|s| s.contains("invariant_total") && s.contains("panic code 0x1")),
                "unexpected failure origin: {}",
                test["origin"]
            );
            ensure!(test["caveat"].is_null(), "failure has replay caveat");
            let blob = test["blob"]
                .as_str()
                .context("failure has no replay blob")?;
            let actions = encode_actions(test)?;
            row["hegel_steps"] = (actions.len() / 4).into();
            row["actions"] = format!("0x{actions}").into();
            let mut replay = property_cmd(
                runner,
                CORRECTNESS_ROOT,
                &[
                    "--match-contract",
                    "SeededInventoryInvariantTest",
                    "--match-test",
                    "stateful",
                ],
                &cases,
                Some(&steps),
                &seed,
            );
            replay
                .args(["--json", "--reproduce", blob, "--output-dir"])
                .arg(dir.join("replay"));
            let (replayed, replay_ms, replay_exit) = capture(replay, &dir, "replay")?;
            ensure!(
                replay_exit == Some(1)
                    && replayed["tests"][0]["status"] == "failed"
                    && replayed["tests"][0]["origin"] == test["origin"]
                    && replayed["tests"][0]["trace"] == test["trace"],
                "replay changed the failure"
            );
            let mut confirm = forge_fuzz_cmd(
                CORRECTNESS_ROOT,
                &[
                    "--match-contract",
                    "^SeededInventoryReplayTest$",
                    "--match-test",
                    "^test_archived_sequence\\(\\)$",
                    "--json",
                ],
                "1",
                &seed,
            );
            confirm.env("HEGEL_INVENTORY_ACTIONS", format!("0x{actions}"));
            let (confirmed, _, confirm_exit) = capture(confirm, &dir, "confirmation")?;
            ensure!(
                confirm_exit == Some(0)
                    && confirmed
                        .as_object()
                        .is_some_and(|suites| suites.values().any(
                            |suite| suite["test_results"]["test_archived_sequence()"]["status"]
                                == "Success"
                        )),
                "Forge did not confirm the archived sequence"
            );
            row["replay_process_ms"] = json!(replay_ms);
            row["replayed"] = true.into();
            row["forge_confirmed"] = true.into();
        }
        let mut control = forge(["test", "--root"], Path::new(CORRECTNESS_ROOT));
        control
            .args([
                "--match-contract",
                "^CorrectInventoryInvariantTest$",
                "--json",
            ])
            .env("FOUNDRY_INVARIANT_RUNS", &cases)
            .env("FOUNDRY_INVARIANT_DEPTH", &steps)
            .env("FOUNDRY_FUZZ_SEED", &seed)
            .env(
                "FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR",
                dir.join("control-failures"),
            );
        let (control_report, _, control_exit) = capture(control, &dir, "forge-control")?;
        ensure!(
            control_exit == Some(0)
                && control_report.as_object().is_some_and(|suites| {
                    suites.values().any(|suite| {
                        suite["test_results"]["invariant_total()"]["status"] == "Success"
                    })
                }),
            "corrected inventory failed under Forge"
        );
        let mut control = property_cmd(
            runner,
            CORRECTNESS_ROOT,
            &[
                "--match-contract",
                "CorrectInventoryInvariantTest",
                "--match-test",
                "stateful",
            ],
            &cases,
            Some(&steps),
            &seed,
        );
        control
            .args(["--json", "--output-dir"])
            .arg(dir.join("control"));
        control.env_remove("HEGEL_SOL_REPRODUCE");
        let (control_report, _, control_exit) = capture(control, &dir, "hegel-control")?;
        ensure!(
            control_exit == Some(0)
                && control_report["tests"]
                    .as_array()
                    .is_some_and(|tests| tests.len() == 1 && tests[0]["status"] == "passed"),
            "corrected inventory failed under Hegel"
        );
        for action in [
            "SeededInventoryHandler.add(uint8)",
            "SeededInventoryHandler.remove(uint8)",
        ] {
            ensure!(
                control_report["tests"][0]["statistics"]["handlers"][action]["completed"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0,
                "corrected inventory never exercised {action}"
            );
        }
        row["corrected_passed_both"] = true.into();
        println!("{row}");
        rows.push(row);
        metadata["rows"] = json!(rows);
        write_json(&output.join("campaign.json"), &metadata)?;
    }
    ensure!(
        rows.iter().any(|r| r["hegel_found"] == true),
        "no Hegel failures found; inspect campaign.json and increase --stateful-cases or --stateful-steps"
    );
    metadata["complete"] = true.into();
    write_json(&output.join("campaign.json"), &metadata)?;
    Ok(())
}

fn encode_actions(test: &Value) -> Result<String> {
    let mut encoded = String::new();
    let mut argument = None;
    for draw in test["trace"].as_array().context("missing trace")? {
        match draw["kind"].as_str().context("missing draw kind")? {
            "sender" => {}
            "uint8" => {
                ensure!(argument.is_none(), "unexpected extra handler argument");
                argument = Some(
                    draw["value"]
                        .as_str()
                        .context("missing argument")?
                        .parse::<u8>()?,
                );
            }
            "rule" => {
                let name = draw["name"].as_str().context("missing rule name")?;
                let action = if name.ends_with(".add") {
                    0
                } else if name.ends_with(".remove") {
                    1
                } else {
                    bail!("unexpected rule {name}")
                };
                let amount = argument.take().context("rule missing argument")?;
                encoded.push_str(&format!("{action:02x}{amount:02x}"));
            }
            kind => bail!("unexpected trace kind {kind}"),
        }
    }
    ensure!(
        argument.is_none() && !encoded.is_empty(),
        "incomplete action trace"
    );
    Ok(encoded)
}

fn capture(mut cmd: Command, dir: &Path, name: &str) -> Result<(Value, u128, Option<i32>)> {
    let description = format!("{cmd:?}");
    println!("{description}");
    fs::write(dir.join(format!("{name}.command.txt")), description)?;
    let start = Instant::now();
    let result = cmd.output()?;
    let elapsed = start.elapsed().as_millis();
    fs::write(dir.join(format!("{name}.json")), &result.stdout)?;
    fs::write(dir.join(format!("{name}.stderr")), &result.stderr)?;
    let value = serde_json::from_slice(&result.stdout).with_context(|| {
        format!(
            "invalid JSON; inspect {}",
            dir.join(format!("{name}.stderr")).display()
        )
    })?;
    Ok((value, elapsed, result.status.code()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_preserves_action_order_and_arguments() {
        let report = json!({"trace": [
            {"kind": "sender", "value": "0x123"},
            {"kind": "uint8", "value": "255"},
            {"kind": "rule", "name": "SeededInventoryHandler.add"},
            {"kind": "uint8", "value": "7"},
            {"kind": "rule", "name": "SeededInventoryHandler.remove"}
        ]});
        assert_eq!(encode_actions(&report).unwrap(), "00ff0107");
    }

    #[test]
    fn confirmation_refuses_incomplete_or_unknown_actions() {
        for trace in [
            json!([]),
            json!([{"kind": "uint8", "value": "1"}]),
            json!([{"kind": "rule", "name": "SeededInventoryHandler.add"}]),
            json!([{"kind": "uint8", "value": "256"}]),
            json!([{"kind": "advance", "value": "1"}]),
            json!([{"kind": "uint8", "value": "1"},
                   {"kind": "rule", "name": "SeededInventoryHandler.reset"}]),
            json!([{"kind": "uint8", "value": "1"},
                   {"kind": "uint8", "value": "2"}]),
        ] {
            assert!(encode_actions(&json!({"trace": trace})).is_err());
        }
    }
}
