use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256, hex, keccak256};
use anyhow::{Result, anyhow, bail};
use revm::{Context, InspectCommitEvm, MainBuilder, MainContext, context::result::ExecutionResult};
use serde::Serialize;

use crate::{
    artifacts::{self, Contract},
    autodraw::Limits,
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
    pub jobs: usize,
    pub shard: Option<Shard>,
    pub max_array_len: u64,
    pub max_byte_len: u64,
    pub fail_fast: bool,
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
            jobs: 1,
            shard: None,
            max_array_len: 8,
            max_byte_len: 64,
            fail_fast: false,
        }
    }
}

impl Options {
    fn limits(&self) -> Limits {
        Limits {
            max_dynamic_length: self.max_array_len,
            max_byte_length: self.max_byte_len,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard {
    index: usize,
    total: usize,
}

impl Shard {
    pub fn new(number: usize, total: usize) -> Result<Self> {
        if total == 0 {
            bail!("shard count must be greater than zero");
        }
        if number == 0 || number > total {
            bail!("shard number must be between 1 and {total}");
        }
        Ok(Self {
            index: number - 1,
            total,
        })
    }

    fn contains(self, name: &str) -> bool {
        let hash = name.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        hash % self.total as u64 == self.index as u64
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
    pub duration_ms: u64,
}

pub fn run_project(options: &Options) -> Result<Vec<TestReport>> {
    let contracts = artifacts::build(&options.root)?;
    let mut discovered = discover_tests(&contracts, options);
    if discovered.is_empty() {
        bail!("no matching Solidity tests found");
    }
    if let Some(shard) = options.shard {
        discovered.retain(|test| shard.contains(&test.id()));
    }

    let mut prepared = BTreeMap::new();
    for test in &discovered {
        let contract = test.contract();
        if !prepared.contains_key(&contract.path) {
            prepared.insert(
                contract.path.clone(),
                PreparedContract::new(contract, options.step_count)?,
            );
        }
    }
    let jobs = discovered
        .iter()
        .map(|test| {
            let contract = prepared
                .get(&test.contract().path)
                .expect("every selected contract was prepared");
            match test {
                DiscoveredTest::Stateless(_, function) => TestJob::Stateless(contract, function),
                DiscoveredTest::Stateful(_) => TestJob::Stateful(contract),
            }
        })
        .collect::<Vec<_>>();
    run_jobs(&jobs, options)
}

pub fn list_project(options: &Options) -> Result<Vec<String>> {
    let contracts = artifacts::build(&options.root)?;
    let mut discovered = discover_tests(&contracts, options);
    if discovered.is_empty() {
        bail!("no matching Solidity tests found");
    }
    if let Some(shard) = options.shard {
        discovered.retain(|test| shard.contains(&test.id()));
    }
    Ok(discovered.into_iter().map(|test| test.name()).collect())
}

fn discover_tests<'a>(contracts: &'a [Contract], options: &Options) -> Vec<DiscoveredTest<'a>> {
    let mut jobs = Vec::new();
    for contract in contracts {
        let contract_matches = options
            .match_contract
            .as_ref()
            .is_none_or(|pattern| contract.name.contains(pattern));
        if !contract_matches {
            continue;
        }
        for function in contract.test_functions() {
            let test_matches = options
                .match_test
                .as_ref()
                .is_none_or(|pattern| function.name.contains(pattern));
            if test_matches {
                jobs.push(DiscoveredTest::Stateless(contract, function));
            }
        }
        let stateful_matches = options.match_test.as_ref().is_none_or(|pattern| {
            "stateful".contains(pattern)
                || contract
                    .rule_functions()
                    .any(|function| function.name.contains(pattern))
                || contract
                    .invariant_functions()
                    .any(|function| function.name.contains(pattern))
        });
        if stateful_matches {
            let has_rules = contract.rule_functions().next().is_some();
            let has_invariants = contract.invariant_functions().next().is_some();
            if has_rules && has_invariants {
                jobs.push(DiscoveredTest::Stateful(contract));
            }
        }
    }
    jobs
}

#[derive(Clone, Copy)]
enum DiscoveredTest<'a> {
    Stateless(&'a Contract, &'a Function),
    Stateful(&'a Contract),
}

impl<'a> DiscoveredTest<'a> {
    fn id(self) -> String {
        match self {
            Self::Stateless(contract, function) => format!(
                "{}.{}:0x{}",
                contract.name,
                function.name,
                hex::encode(function.selector())
            ),
            Self::Stateful(contract) => format!("{}.stateful", contract.name),
        }
    }

    fn contract(self) -> &'a Contract {
        match self {
            Self::Stateless(contract, _) | Self::Stateful(contract) => contract,
        }
    }

    fn name(self) -> String {
        match self {
            Self::Stateless(contract, function) => format!("{}.{}", contract.name, function.name),
            Self::Stateful(contract) => format!("{}.stateful", contract.name),
        }
    }
}

struct PreparedContract<'a> {
    contract: &'a Contract,
    address: Address,
    base: Database,
    setup: Option<Bytes>,
    step_count: i64,
}

impl<'a> PreparedContract<'a> {
    fn new(contract: &'a Contract, default_step_count: i64) -> Result<Self> {
        let mut harness = Harness::new();
        let address = harness.deploy(contract.bytecode.clone())?;
        let mut step_count = default_step_count;
        if let Some(function) = contract
            .abi
            .function("stepCount")
            .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
        {
            let calldata = Bytes::copy_from_slice(function.selector().as_slice());
            // Probe on a clone so even a non-view stepCount implementation cannot
            // mutate the snapshot shared by the actual tests.
            let mut probe = harness.clone();
            let result = probe.call(address, calldata)?;
            if let Some(output) = result.output()
                && output.len() >= 32
                && let Ok(value) = i64::try_from(U256::from_be_slice(&output[..32]))
                && value > 0
            {
                step_count = value;
            }
        }
        let setup = contract
            .abi
            .function("setUp")
            .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
            .map(|function| Bytes::copy_from_slice(function.selector().as_slice()));
        Ok(Self {
            contract,
            address,
            base: harness.snapshot(),
            setup,
            step_count,
        })
    }
}

#[derive(Clone, Copy)]
enum TestJob<'a> {
    Stateless(&'a PreparedContract<'a>, &'a Function),
    Stateful(&'a PreparedContract<'a>),
}

impl TestJob<'_> {
    fn run(self, options: &Options) -> Result<Vec<TestReport>> {
        let started = Instant::now();
        let result = match self {
            Self::Stateless(contract, function) => run_test(contract, function, options),
            Self::Stateful(contract) => run_stateful(contract, options),
        };
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        result.map(|mut reports| {
            for report in &mut reports {
                report.duration_ms = elapsed;
            }
            reports
        })
    }
}

fn run_jobs(jobs: &[TestJob<'_>], options: &Options) -> Result<Vec<TestReport>> {
    let workers = options.jobs.max(1).min(jobs.len().max(1));
    if workers == 1 {
        let mut reports = Vec::new();
        for job in jobs {
            let job_reports = job.run(options)?;
            let failed = job_reports
                .iter()
                .any(|report| matches!(report.status, TestStatus::Failed));
            reports.extend(job_reports);
            if options.fail_fast && failed {
                break;
            }
        }
        return Ok(reports);
    }

    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let completed = Mutex::new(Vec::with_capacity(jobs.len()));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else {
                        break;
                    };
                    let result = job.run(options);
                    let failed = match &result {
                        Err(_) => true,
                        Ok(reports) => reports
                            .iter()
                            .any(|report| matches!(report.status, TestStatus::Failed)),
                    };
                    if options.fail_fast && failed {
                        stopped.store(true, Ordering::Release);
                    }
                    completed
                        .lock()
                        .expect("test result lock poisoned")
                        .push((index, result));
                }
            });
        }
    });

    let mut completed = completed.into_inner().expect("test result lock poisoned");
    completed.sort_by_key(|(index, _)| *index);
    completed
        .into_iter()
        .try_fold(Vec::new(), |mut reports, (_, result)| {
            reports.extend(result?);
            Ok(reports)
        })
}

fn run_test(
    prepared: &PreparedContract<'_>,
    function: &Function,
    options: &Options,
) -> Result<Vec<TestReport>> {
    let contract = prepared.contract;
    let test_name = format!("{}.{}", contract.name, function.name);
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
        let execution = execute_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            contract,
            function,
            &tc,
            options.limits(),
        )?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            contract,
            function,
            &tc,
            options.limits(),
        )?;
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
                duration_ms: 0,
            });
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            contract,
            function,
            &tc,
            options.limits(),
        )?;
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
            duration_ms: 0,
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
            duration_ms: 0,
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
    limits: Limits,
) -> Result<CaseExecution> {
    let (calldata, mut auto_trace) = match crate::autodraw::calldata(function, tc, limits) {
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

#[derive(Clone, Copy)]
struct StatefulConfig<'a> {
    contract: &'a Contract,
    step_count: i64,
    limits: Limits,
}

impl<'a> StatefulConfig<'a> {
    fn invariant<'b>(self, phase: Option<&'b str>) -> InvariantConfig<'a, 'b> {
        InvariantConfig {
            contract: self.contract,
            limits: self.limits,
            phase,
        }
    }
}

#[derive(Clone, Copy)]
struct InvariantConfig<'a, 'b> {
    contract: &'a Contract,
    limits: Limits,
    phase: Option<&'b str>,
}

fn run_stateful(prepared: &PreparedContract<'_>, options: &Options) -> Result<Vec<TestReport>> {
    let contract = prepared.contract;
    let test_name = format!("{}.stateful", contract.name);
    let rules: Vec<_> = contract.rule_functions().collect();
    let invariants: Vec<_> = contract.invariant_functions().collect();
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

    let config = StatefulConfig {
        contract,
        step_count: prepared.step_count,
        limits: options.limits(),
    };

    if let Some(blob) = &options.reproduce {
        let mut tc = settings.test_case_from_blob(blob)?;
        let execution = execute_stateful_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            config,
        )?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute_stateful_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            config,
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
                duration_ms: 0,
            });
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute_stateful_case(
            &prepared.base,
            prepared.address,
            prepared.setup.as_ref(),
            &rules,
            &invariants,
            &tc,
            config,
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
            duration_ms: 0,
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
    config: StatefulConfig<'_>,
) -> Result<CaseExecution> {
    let mut db = base.clone();
    let mut inspector = HegelInspector::new(tc);
    let mut trace = Vec::new();

    if let Some(setup) = setup {
        let result = inspected_call(&mut db, &mut inspector, address, setup.clone())?;
        if let Some(execution) =
            stateful_abort_or_failure(&mut inspector, result, &mut trace, config.contract, "setUp")?
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
        config.invariant(None),
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
    let machine = match tc.new_state_machine(
        &rule_names,
        &groups,
        &invariant_names,
        &always,
        config.step_count,
    ) {
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
                config.invariant(None),
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
            let (calldata, drawn) = match generated_calldata(function, tc, config.limits)? {
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
            let (status, origin) = classify_with_abi(result, config.contract);
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
                    config.invariant(Some("sampled")),
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
    config: InvariantConfig<'_, '_>,
) -> Result<Option<CaseExecution>> {
    for function in invariants {
        let (calldata, drawn) = match generated_calldata(function, tc, config.limits)? {
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
            config.contract,
            &config.phase.map_or_else(
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
    contract: &Contract,
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
            let (status, origin) = classify_with_abi(result, contract);
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
    limits: Limits,
) -> Result<std::result::Result<(Bytes, Vec<Draw>), Status>> {
    match crate::autodraw::calldata(function, tc, limits) {
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
