use std::path::{Path, PathBuf};

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256, hex, keccak256};
use anyhow::{Result, anyhow, bail};
use revm::{Context, InspectCommitEvm, MainBuilder, MainContext, context::result::ExecutionResult};
use serde::Serialize;

use crate::{
    artifacts::{self, Contract},
    engine::{Context as HegelContext, Error, Settings, Status, TestCase},
    evm::{self, Database, Harness},
    inspector::{Abort, Draw, HegelInspector},
};

#[derive(Debug, Clone)]
pub struct Options {
    pub root: PathBuf,
    pub match_contract: Option<String>,
    pub match_test: Option<String>,
    pub test_cases: u64,
    pub seed: Option<u64>,
    pub database: Option<String>,
    pub profile: Option<String>,
    pub reproduce: Option<String>,
    pub phases: u32,
    pub verbosity: u32,
    pub step_count: i64,
    pub show_statistics: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            root: PathBuf::from("contracts"),
            match_contract: None,
            match_test: None,
            test_cases: 100,
            seed: None,
            database: None,
            profile: None,
            reproduce: None,
            step_count: 50,
            show_statistics: false,
            phases: 31,
            verbosity: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Invalid,
    Overrun,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestReport {
    pub test: String,
    pub status: TestStatus,
    pub origin: Option<String>,
    pub blob: Option<String>,
    pub trace: Vec<Draw>,
    pub console: Vec<String>,
}

pub fn run_project(options: &Options) -> Result<Vec<TestReport>> {
    let contracts = artifacts::build(&options.root)?;
    let selected: Vec<_> = contracts
        .iter()
        .flat_map(|contract| {
            contract.test_functions().filter_map(move |function| {
                let contract_matches = options
                    .match_contract
                    .as_ref()
                    .is_none_or(|pattern| contract.name.contains(pattern));
                let test_matches = options
                    .match_test
                    .as_ref()
                    .is_none_or(|pattern| function.name.contains(pattern));
                (contract_matches && test_matches).then_some((contract, function))
            })
        })
        .collect();
    let stateful: Vec<_> = contracts
        .iter()
        .filter(|contract| {
            let has_rules = contract.rule_functions().next().is_some();
            let has_invariants = contract.invariant_functions().next().is_some();
            let contract_matches = options
                .match_contract
                .as_ref()
                .is_none_or(|pattern| contract.name.contains(pattern));
            let test_matches = options.match_test.as_ref().is_none_or(|pattern| {
                "stateful".contains(pattern)
                    || contract
                        .rule_functions()
                        .any(|function| function.name.contains(pattern))
                    || contract
                        .invariant_functions()
                        .any(|function| function.name.contains(pattern))
            });
            has_rules && has_invariants && contract_matches && test_matches
        })
        .collect();
    if selected.is_empty() && stateful.is_empty() {
        bail!("no matching Solidity tests found");
    }

    let mut reports = Vec::new();
    for (contract, function) in selected {
        reports.extend(run_test(contract, function, options)?);
    }
    for contract in stateful {
        reports.extend(run_stateful(contract, options)?);
    }
    Ok(reports)
}

fn run_test(
    contract: &Contract,
    function: &Function,
    options: &Options,
) -> Result<Vec<TestReport>> {
    let test_name = format!("{}.{}", contract.name, function.name);
    let mut harness = Harness::new();
    let address = harness.deploy(contract.bytecode.clone())?;
    let base = harness.snapshot();
    let setup = contract
        .abi
        .function("setUp")
        .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
        .map(|function| Bytes::copy_from_slice(function.selector().as_slice()));

    let ctx = HegelContext::new();
    let mut settings = match options.profile.as_deref() {
        Some(profile) => Settings::for_profile(&ctx, profile)?,
        None => Settings::new(&ctx)?,
    };
    settings
        .set_test_cases(options.test_cases)?
        .set_seed(options.seed)?
        .set_database(options.database.as_deref())?
        .set_database_key(&test_name)?
        .set_phases(options.phases)?
        .set_verbosity(options.verbosity)?
        .set_print_blob(false)?;

    if let Some(blob) = &options.reproduce {
        let mut tc = settings.test_case_from_blob(blob)?;
        let execution = execute_case(&base, address, setup.as_ref(), contract, function, &tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute_case(&base, address, setup.as_ref(), contract, function, &tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
    }
    let result = run.result()?;
    if let Some(error) = result.error()? {
        bail!("Hegel run failed: {error}");
    }
    let mut reports = Vec::new();
    for index in 0..result.failure_count()? {
        let failure = result.failure(index)?;
        let blob = failure.reproduction_blob()?;
        let Some(blob_value) = blob.as_deref() else {
            reports.push(TestReport {
                test: test_name.clone(),
                status: TestStatus::Failed,
                origin: Some(failure.origin()?),
                blob: None,
                console: Vec::new(),
                trace: Vec::new(),
            });
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute_case(&base, address, setup.as_ref(), contract, function, &tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        reports.push(execution.report(test_name.clone(), blob));
    }
    if reports.is_empty() {
        reports.push(TestReport {
            test: test_name,
            status: TestStatus::Passed,
            origin: None,
            console: Vec::new(),
            blob: None,
            trace: Vec::new(),
        });
    }
    Ok(reports)
}

struct CaseExecution {
    console: Vec<String>,
    status: Status,
    origin: Option<String>,
    trace: Vec<Draw>,
}

impl CaseExecution {
    fn report(self, test: String, blob: Option<String>) -> TestReport {
        let status = match self.status {
            Status::Valid => TestStatus::Passed,
            Status::Invalid => TestStatus::Invalid,
            Status::Overrun => TestStatus::Overrun,
            Status::Interesting => TestStatus::Failed,
        };
        TestReport {
            test,
            console: self.console,
            status,
            origin: self.origin,
            blob,
            trace: self.trace,
        }
    }
}

fn execute_case(
    base: &Database,
    address: Address,
    setup: Option<&Bytes>,
    contract: &Contract,
    function: &Function,
    tc: &TestCase,
) -> Result<CaseExecution> {
    let (calldata, mut auto_trace) = match crate::autodraw::calldata(function, tc) {
        Ok(prepared) => prepared,
        Err(Error::Stop(_)) => {
            return Ok(CaseExecution {
                console: Vec::new(),
                status: Status::Overrun,
                origin: None,
                trace: Vec::new(),
            });
        }
        Err(Error::Assume) => {
            return Ok(CaseExecution {
                console: Vec::new(),
                status: Status::Invalid,
                origin: None,
                trace: Vec::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let mut db = base.clone();
    let mut inspector = HegelInspector::new(tc);
    let mut result = None;
    if let Some(setup) = setup {
        let setup_result = inspected_call(&mut db, &mut inspector, address, setup.clone())?;
        if !setup_result.is_success() {
            result = Some(setup_result);
        }
    }
    if result.is_none() && inspector.outcome.is_none() {
        result = Some(inspected_call(&mut db, &mut inspector, address, calldata)?);
    }

    let (status, origin) = match inspector.outcome.take() {
        Some(Abort::Invalid) => (Status::Invalid, None),
        Some(Abort::Overrun) => (Status::Overrun, None),
        Some(Abort::Error(message)) => return Err(anyhow!(message)),
        None => classify_with_abi(
            result.expect("a setup or test transaction was executed"),
            contract,
        ),
    };
    auto_trace.extend(inspector.trace);
    Ok(CaseExecution {
        console: inspector.console,
        status,
        origin,
        trace: auto_trace,
    })
}

fn inspected_call(
    db: &mut Database,
    inspector: &mut HegelInspector<'_>,
    address: Address,
    calldata: Bytes,
) -> Result<ExecutionResult> {
    let context = Context::mainnet()
        .with_db(db)
        .with_cfg(evm::config())
        .modify_block_chained(|block| inspector.configure_block(block));
    let mut evm = context.build_mainnet_with_inspector(inspector);
    evm.inspect_tx_commit(evm::transaction(
        evm::DEFAULT_CALLER,
        evm::GAS_LIMIT,
        address,
        calldata,
    ))
    .map_err(|error| anyhow!("EVM transaction failed: {error:?}"))
}

fn classify(result: ExecutionResult) -> (Status, Option<String>) {
    match result {
        ExecutionResult::Success { .. } => (Status::Valid, None),
        ExecutionResult::Revert { output, .. } => {
            let reason = decode_revert(&output);
            (Status::Interesting, Some(reason))
        }
        ExecutionResult::Halt { reason, .. } => {
            (Status::Interesting, Some(format!("EVM halt: {reason:?}")))
        }
    }
}

fn classify_with_abi(result: ExecutionResult, contract: &Contract) -> (Status, Option<String>) {
    match result {
        ExecutionResult::Success { .. } => (Status::Valid, None),
        ExecutionResult::Revert { output, .. } => {
            let reason =
                decode_custom_error(&output, contract).unwrap_or_else(|| decode_revert(&output));
            (Status::Interesting, Some(reason))
        }
        ExecutionResult::Halt { reason, .. } => {
            (Status::Interesting, Some(format!("EVM halt: {reason:?}")))
        }
    }
}

fn decode_custom_error(output: &[u8], contract: &Contract) -> Option<String> {
    if output.len() < 4 {
        return None;
    }
    let error = contract
        .abi
        .errors()
        .find(|error| error.selector().as_slice() == &output[..4])?;
    let values = error.abi_decode_input(&output[4..]).ok()?;
    let rendered = values
        .iter()
        .map(crate::autodraw::format_value)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{}({rendered})", error.name))
}

pub fn decode_revert(output: &[u8]) -> String {
    if output.len() >= 4
        && output[..4] == keccak256("Error(string)")[..4]
        && let Some(message) = decode_abi_string(&output[4..])
    {
        return format!("revert: {message}");
    }
    if output.len() >= 36 && output[..4] == keccak256("Panic(uint256)")[..4] {
        return format!("panic code 0x{:x}", U256::from_be_slice(&output[4..36]));
    }
    if output.len() >= 4 {
        format!("revert selector 0x{}", hex::encode(&output[..4]))
    } else {
        "revert with no data".into()
    }
}

fn decode_abi_string(args: &[u8]) -> Option<String> {
    if args.len() < 64 {
        return None;
    }
    let offset = usize::try_from(U256::from_be_slice(&args[..32])).ok()?;
    let len_end = offset.checked_add(32)?;
    let len = usize::try_from(U256::from_be_slice(args.get(offset..len_end)?)).ok()?;
    let end = len_end.checked_add(len)?;
    Some(String::from_utf8_lossy(args.get(len_end..end)?).into_owned())
}

pub fn database_argument(root: &Path, value: &str) -> String {
    if value == "off" {
        String::new()
    } else {
        root.join(value).to_string_lossy().into_owned()
    }
}

fn run_stateful(contract: &Contract, options: &Options) -> Result<Vec<TestReport>> {
    let test_name = format!("{}.stateful", contract.name);
    let rules: Vec<_> = contract.rule_functions().collect();
    let invariants: Vec<_> = contract.invariant_functions().collect();
    let mut harness = Harness::new();
    let address = harness.deploy(contract.bytecode.clone())?;
    let mut step_count = options.step_count;
    if let Some(function) = contract
        .abi
        .function("stepCount")
        .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
    {
        let calldata = Bytes::copy_from_slice(function.selector().as_slice());
        let result = harness.call(address, calldata)?;
        if let Some(output) = result.output()
            && output.len() >= 32
            && let Ok(value) = i64::try_from(U256::from_be_slice(&output[..32]))
            && value > 0
        {
            step_count = value;
        }
    }
    let base = harness.snapshot();
    let setup = contract
        .abi
        .function("setUp")
        .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
        .map(|function| Bytes::copy_from_slice(function.selector().as_slice()));

    let ctx = HegelContext::new();
    let mut settings = match options.profile.as_deref() {
        Some(profile) => Settings::for_profile(&ctx, profile)?,
        None => Settings::new(&ctx)?,
    };
    settings
        .set_test_cases(options.test_cases)?
        .set_seed(options.seed)?
        .set_database(options.database.as_deref())?
        .set_database_key(&test_name)?
        .set_phases(options.phases)?
        .set_verbosity(options.verbosity)?
        .set_print_blob(false)?;

    if let Some(blob) = &options.reproduce {
        let mut tc = settings.test_case_from_blob(blob)?;
        let execution = execute_stateful_case(
            &base,
            address,
            setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            step_count,
        )?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute_stateful_case(
            &base,
            address,
            setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            step_count,
        )?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
    }
    let result = run.result()?;
    if let Some(error) = result.error()? {
        bail!("Hegel stateful run failed: {error}");
    }
    let mut reports = Vec::new();
    for index in 0..result.failure_count()? {
        let failure = result.failure(index)?;
        let blob = failure.reproduction_blob()?;
        let Some(blob_value) = blob.as_deref() else {
            reports.push(TestReport {
                test: test_name.clone(),
                status: TestStatus::Failed,
                origin: Some(failure.origin()?),
                blob: None,
                trace: Vec::new(),
                console: Vec::new(),
            });
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute_stateful_case(
            &base,
            address,
            setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            step_count,
        )?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        reports.push(execution.report(test_name.clone(), blob));
    }
    if reports.is_empty() {
        reports.push(TestReport {
            test: test_name,
            status: TestStatus::Passed,
            origin: None,
            blob: None,
            trace: Vec::new(),
            console: Vec::new(),
        });
    }
    Ok(reports)
}

fn execute_stateful_case(
    base: &Database,
    address: Address,
    setup: Option<&Bytes>,
    rules: &[&Function],
    invariants: &[&Function],
    tc: &TestCase,
    step_count: i64,
) -> Result<CaseExecution> {
    let mut db = base.clone();
    let mut inspector = HegelInspector::new(tc);
    let mut trace = Vec::new();

    if let Some(setup) = setup {
        let result = inspected_call(&mut db, &mut inspector, address, setup.clone())?;
        if let Some(execution) =
            stateful_abort_or_failure(&mut inspector, result, &mut trace, "setUp")?
        {
            return Ok(execution);
        }
    }
    if let Some(execution) = check_invariants(
        &mut db,
        &mut inspector,
        address,
        invariants,
        tc,
        &mut trace,
        None,
    )? {
        return Ok(execution);
    }

    let rule_names = rules
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    let invariant_names = invariants
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    let groups = vec![0; rules.len()];
    let always = vec![false; invariants.len()];
    let machine =
        match tc.new_state_machine(&rule_names, &groups, &invariant_names, &always, step_count) {
            Ok(machine) => machine,
            Err(error) => return engine_case(error, inspector, trace),
        };
    debug_assert_eq!(machine.concurrency(), 1);

    loop {
        let group = match machine.next_group(tc) {
            Ok(group) => group,
            Err(error) => return engine_case(error, inspector, trace),
        };
        if group.is_none() {
            if let Some(execution) = check_invariants(
                &mut db,
                &mut inspector,
                address,
                invariants,
                tc,
                &mut trace,
                None,
            )? {
                return Ok(execution);
            }
            return Ok(finish_case(inspector, trace, Status::Valid, None));
        }

        loop {
            let index = match machine.next_rule(tc, 0) {
                Ok(Some(index)) => index,
                Ok(None) => break,
                Err(error) => return engine_case(error, inspector, trace),
            };
            let function = rules
                .get(index)
                .ok_or_else(|| anyhow!("Hegel selected unknown rule index {index}"))?;
            tc.start_span(&format!("hegel.sol.rule.{}", function.name))?;
            let (calldata, drawn) = match generated_calldata(function, tc)? {
                Ok(prepared) => prepared,
                Err(status) => {
                    let _ = tc.stop_span(true);
                    return Ok(finish_case(inspector, trace, status, None));
                }
            };
            trace.push(Draw {
                name: function.name.clone(),
                kind: "rule".into(),
                value: "()".into(),
            });
            trace.extend(drawn);
            inspector.outcome = None;
            let result = inspected_call(&mut db, &mut inspector, address, calldata)?;
            match inspector.outcome.take() {
                Some(Abort::Invalid) => {
                    tc.stop_span(true)?;
                    machine.rule_rejected(tc, 0)?;
                    continue;
                }
                Some(Abort::Overrun) => {
                    let _ = tc.stop_span(true);
                    return Ok(finish_case(inspector, trace, Status::Overrun, None));
                }
                Some(Abort::Error(message)) => return Err(anyhow!(message)),
                None => tc.stop_span(false)?,
            }
            let (status, origin) = classify(result);
            if status != Status::Valid {
                return Ok(finish_case(
                    inspector,
                    trace,
                    status,
                    origin.map(|origin| format!("{}: {origin}", function.name)),
                ));
            }
        }

        for index in 0..invariants.len() {
            let should_check = match machine.should_check_invariant(tc, index) {
                Ok(value) => value,
                Err(error) => return engine_case(error, inspector, trace),
            };
            if should_check
                && let Some(execution) = check_invariants(
                    &mut db,
                    &mut inspector,
                    address,
                    &invariants[index..=index],
                    tc,
                    &mut trace,
                    Some("sampled"),
                )?
            {
                return Ok(execution);
            }
        }
    }
}

fn check_invariants(
    db: &mut Database,
    inspector: &mut HegelInspector<'_>,
    address: Address,
    invariants: &[&Function],
    tc: &TestCase,
    trace: &mut Vec<Draw>,
    phase: Option<&str>,
) -> Result<Option<CaseExecution>> {
    for function in invariants {
        let (calldata, drawn) = match generated_calldata(function, tc)? {
            Ok(prepared) => prepared,
            Err(status) => {
                return Ok(Some(finish_case_from(inspector, trace, status, None)));
            }
        };
        trace.extend(drawn);
        let result = inspected_call(db, inspector, address, calldata)?;
        if let Some(execution) = stateful_abort_or_failure(
            inspector,
            result,
            trace,
            &phase.map_or_else(
                || function.name.clone(),
                |phase| format!("{} ({phase})", function.name),
            ),
        )? {
            return Ok(Some(execution));
        }
    }
    Ok(None)
}

fn stateful_abort_or_failure(
    inspector: &mut HegelInspector<'_>,
    result: ExecutionResult,
    trace: &mut Vec<Draw>,
    label: &str,
) -> Result<Option<CaseExecution>> {
    match inspector.outcome.take() {
        Some(Abort::Invalid) => Ok(Some(finish_case_from(
            inspector,
            trace,
            Status::Invalid,
            None,
        ))),
        Some(Abort::Overrun) => Ok(Some(finish_case_from(
            inspector,
            trace,
            Status::Overrun,
            None,
        ))),
        Some(Abort::Error(message)) => Err(anyhow!(message)),
        None => {
            let (status, origin) = classify(result);
            if status == Status::Valid {
                Ok(None)
            } else {
                Ok(Some(finish_case_from(
                    inspector,
                    trace,
                    status,
                    origin.map(|origin| format!("{label}: {origin}")),
                )))
            }
        }
    }
}

fn generated_calldata(
    function: &Function,
    tc: &TestCase,
) -> Result<std::result::Result<(Bytes, Vec<Draw>), Status>> {
    match crate::autodraw::calldata(function, tc) {
        Ok(prepared) => Ok(Ok(prepared)),
        Err(Error::Stop(_)) => Ok(Err(Status::Overrun)),
        Err(Error::Assume) => Ok(Err(Status::Invalid)),
        Err(error) => Err(error.into()),
    }
}

fn engine_case(
    error: Error,
    inspector: HegelInspector<'_>,
    trace: Vec<Draw>,
) -> Result<CaseExecution> {
    match error {
        Error::Stop(_) => Ok(finish_case(inspector, trace, Status::Overrun, None)),
        Error::Assume => Ok(finish_case(inspector, trace, Status::Invalid, None)),
        error => Err(error.into()),
    }
}

fn finish_case(
    inspector: HegelInspector<'_>,
    mut trace: Vec<Draw>,
    status: Status,
    origin: Option<String>,
) -> CaseExecution {
    trace.extend(inspector.trace);
    CaseExecution {
        console: inspector.console,
        status,
        origin,
        trace,
    }
}

fn finish_case_from(
    inspector: &mut HegelInspector<'_>,
    trace: &mut Vec<Draw>,
    status: Status,
    origin: Option<String>,
) -> CaseExecution {
    trace.append(&mut inspector.trace);
    CaseExecution {
        console: std::mem::take(&mut inspector.console),
        status,
        origin,
        trace: std::mem::take(trace),
    }
}
