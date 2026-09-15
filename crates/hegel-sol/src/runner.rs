use std::path::{Path, PathBuf};

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
    if selected.is_empty() {
        bail!("no matching Solidity tests found");
    }

    let mut reports = Vec::new();
    for (contract, function) in selected {
        reports.extend(run_test(contract, function, options)?);
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
        let execution = execute_case(&base, address, setup.as_ref(), function, &tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute_case(&base, address, setup.as_ref(), function, &tc)?;
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
                trace: Vec::new(),
            });
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute_case(&base, address, setup.as_ref(), function, &tc)?;
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
        });
    }
    Ok(reports)
}

struct CaseExecution {
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
    function: &Function,
    tc: &TestCase,
) -> Result<CaseExecution> {
    let (calldata, mut auto_trace) = match crate::autodraw::calldata(function, tc) {
        Ok(prepared) => prepared,
        Err(Error::Stop(_)) => {
            return Ok(CaseExecution {
                status: Status::Overrun,
                origin: None,
                trace: Vec::new(),
            });
        }
        Err(Error::Assume) => {
            return Ok(CaseExecution {
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
        None => classify(result.expect("a setup or test transaction was executed")),
    };
    auto_trace.extend(inspector.trace);
    Ok(CaseExecution {
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
    let context = Context::mainnet().with_db(db).with_cfg(evm::config());
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
