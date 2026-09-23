use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256, hex};
use anyhow::{Result, anyhow, bail};
use revm::{Context, InspectCommitEvm, MainBuilder, MainContext, context::result::ExecutionResult};
use serde::Serialize;

use crate::{
    artifacts::{Contract, FoundryProject, Project},
    autodraw::Limits,
    discovery::{Conventions, STATEFUL_SUFFIX},
    engine::{Context as HegelContext, Error, Settings, Status, TestCase},
    evm::{self, Database, Harness},
    inspector::{Abort, Draw, HegelInspector},
    protocol,
    sourcemap::SourceLocation,
    values::format_value,
};

/// Objective name for the basic-block coverage score. All tests report under one
/// label so the engine treats coverage as a single quantity to maximise.
const COVERAGE_LABEL: &str = "hegel.sol.coverage";

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
    pub jobs: usize,
    pub shard: Option<Shard>,
    pub max_array_len: u64,
    pub max_byte_len: u64,
    pub fail_fast: bool,
    /// Function-name prefixes that decide what counts as a test.
    pub conventions: Conventions,
    /// Distinct `msg.sender` values the state machine may call handlers from.
    pub actors: usize,
    /// Treat a reverting handler as a rejected step instead of a failure.
    pub allow_rule_reverts: bool,
    /// Report basic-block coverage to the engine as a target score.
    pub coverage_target: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            match_contract: None,
            match_test: None,
            test_cases: 100,
            seed: None,
            database: None,
            profile: None,
            reproduce: None,
            step_count: 50,
            phases: 31,
            verbosity: 0,
            jobs: 1,
            shard: None,
            max_array_len: 8,
            max_byte_len: 64,
            fail_fast: false,
            conventions: Conventions::default(),
            actors: 3,
            allow_rule_reverts: false,
            coverage_target: true,
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

    /// Addresses handler calls may originate from.
    ///
    /// Many invariants only break when two accounts interact, so a single caller
    /// leaves a whole class of bugs unreachable. Addresses start at `0xac..` to
    /// stay clear of the precompiles at the bottom of the address space.
    fn actor_addresses(&self) -> Vec<Address> {
        let count = self.actors.clamp(1, 256);
        (0..count)
            .map(|index| {
                let mut bytes = [0; 20];
                bytes[0] = 0xac;
                bytes[19] = index as u8;
                Address::new(bytes)
            })
            .collect()
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

    /// Assign a test to a shard by hashing its name, so every machine in a CI
    /// matrix reaches the same partition without exchanging a manifest.
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
    pub source: Option<SourceLocation>,
    pub blob: Option<String>,
    pub trace: Vec<Draw>,
    pub console: Vec<String>,
    pub notes: Vec<String>,
    pub duration_ms: u64,
}

/// Everything one invocation produced.
#[derive(Debug)]
pub struct Execution {
    pub reports: Vec<TestReport>,
    /// Non-fatal problems from loading the project, for the caller to surface.
    pub warnings: Vec<String>,
}

impl Execution {
    pub fn failed(&self) -> bool {
        self.reports
            .iter()
            .any(|report| matches!(report.status, TestStatus::Failed))
    }
}

impl From<Status> for TestStatus {
    fn from(status: Status) -> Self {
        match status {
            Status::Valid => Self::Passed,
            Status::Invalid => Self::Invalid,
            Status::Overrun => Self::Overrun,
            Status::Interesting => Self::Failed,
        }
    }
}

impl TestReport {
    fn empty(test: String, status: TestStatus, origin: Option<String>) -> Self {
        Self {
            test,
            status,
            origin,
            source: None,
            blob: None,
            trace: Vec::new(),
            console: Vec::new(),
            notes: Vec::new(),
            duration_ms: 0,
        }
    }
}

pub fn run_project(options: &Options) -> Result<Execution> {
    let loaded = FoundryProject::new(&options.root).compile()?;
    let discovered = select(&loaded.contracts, options)?;

    let mut prepared = BTreeMap::new();
    for test in &discovered {
        let contract = test.contract();
        if !prepared.contains_key(&contract.path) {
            prepared.insert(
                contract.path.clone(),
                PreparedContract::new(contract, options)?,
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
    Ok(Execution {
        reports: run_jobs(&jobs, options)?,
        warnings: loaded.warnings,
    })
}

pub fn list_project(options: &Options) -> Result<Vec<String>> {
    let loaded = FoundryProject::new(&options.root).compile()?;
    Ok(select(&loaded.contracts, options)?
        .into_iter()
        .map(|test| test.name())
        .collect())
}

fn select<'a>(contracts: &'a [Contract], options: &'a Options) -> Result<Vec<DiscoveredTest<'a>>> {
    let mut discovered = discover_tests(contracts, options);
    if let Some(shard) = options.shard {
        discovered.retain(|test| shard.contains(&test.id()));
    }
    if discovered.is_empty() {
        bail!("no matching Solidity tests found");
    }
    // A blob encodes one example's choices, which only mean anything to the test
    // that made them. Replaying it against others silently reports whatever those
    // draws happen to produce, usually a vacuous pass, so refuse the ambiguity.
    if options.reproduce.is_some() && discovered.len() > 1 {
        bail!(
            "a reproduce blob applies to a single test, but {} were selected; narrow the run \
             with --match-contract or --match-test",
            discovered.len()
        );
    }
    Ok(discovered)
}

fn discover_tests<'a>(contracts: &'a [Contract], options: &'a Options) -> Vec<DiscoveredTest<'a>> {
    let conventions = &options.conventions;
    let mut jobs = Vec::new();
    for contract in contracts {
        let contract_matches = options
            .match_contract
            .as_ref()
            .is_none_or(|pattern| contract.name.contains(pattern));
        if !contract_matches {
            continue;
        }
        for function in contract.test_functions(conventions) {
            let test_matches = options
                .match_test
                .as_ref()
                .is_none_or(|pattern| function.name.contains(pattern));
            if test_matches {
                jobs.push(DiscoveredTest::Stateless(contract, function));
            }
        }

        let has_rules = contract.rule_functions(conventions).next().is_some();
        let has_invariants = contract.invariant_functions(conventions).next().is_some();
        if !has_rules || !has_invariants {
            continue;
        }
        // A state machine has no function name of its own, so `--match-test`
        // selects it by naming one of its members or by asking for state
        // machines outright. Matching the literal "stateful" as a substring
        // would select every machine for a pattern as short as "a".
        let stateful_matches = options.match_test.as_ref().is_none_or(|pattern| {
            pattern.eq_ignore_ascii_case(STATEFUL_SUFFIX)
                || contract
                    .rule_functions(conventions)
                    .chain(contract.invariant_functions(conventions))
                    .any(|function| function.name.contains(pattern))
        });
        if stateful_matches {
            jobs.push(DiscoveredTest::Stateful(contract));
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
    /// Sharding key. The selector is included so that renaming a contract's
    /// unrelated functions does not reshuffle which shard a test lands in.
    fn id(self) -> String {
        match self {
            Self::Stateless(contract, function) => format!(
                "{}.{}:0x{}",
                contract.name,
                function.name,
                hex::encode(function.selector())
            ),
            Self::Stateful(contract) => format!("{}.{STATEFUL_SUFFIX}", contract.name),
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
            Self::Stateful(contract) => format!("{}.{STATEFUL_SUFFIX}", contract.name),
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
    fn new(contract: &'a Contract, options: &Options) -> Result<Self> {
        let conventions = &options.conventions;
        let mut harness = Harness::new(&options.actor_addresses());
        let address = harness.deploy(contract.bytecode.clone())?;
        check_protocol_version(contract, &harness, address)?;

        let mut step_count = options.step_count;
        if let Some(function) = contract.nullary(&conventions.step_count) {
            // Probe on a clone so even a non-view implementation cannot mutate
            // the snapshot shared by the actual tests.
            let mut probe = harness.clone();
            let result = probe.call(address, selector_calldata(function))?;
            if let Some(output) = result.output()
                && output.len() >= 32
                && let Ok(value) = i64::try_from(U256::from_be_slice(&output[..32]))
                && value > 0
            {
                step_count = value;
            }
        }
        Ok(Self {
            contract,
            address,
            base: harness.snapshot(),
            setup: contract.nullary(&conventions.setup).map(selector_calldata),
            step_count,
        })
    }
}

/// Reject a `Hegel.sol` built against a different release of the protocol.
///
/// Test projects hold their own copy of the Solidity surface, so it can lag the
/// runner. Detecting that here turns a confusing unknown-selector failure, raised
/// mid-test, into one actionable message before any test runs. Contracts that do
/// not extend `HegelTest` have no version to report and are left alone.
fn check_protocol_version(contract: &Contract, harness: &Harness, address: Address) -> Result<()> {
    let Some(function) = contract.nullary(protocol::PROTOCOL_VERSION_FUNCTION) else {
        return Ok(());
    };
    let mut probe = harness.clone();
    let result = probe.call(address, selector_calldata(function))?;
    let version = result
        .output()
        .filter(|output| output.len() >= 32)
        .map(|output| U256::from_be_slice(&output[..32]))
        .unwrap_or(U256::ZERO);
    if version != U256::from(protocol::PROTOCOL_VERSION) {
        bail!(
            "{} was compiled against Hegel.sol protocol version {version}, but this runner \
             speaks version {}. Update the project's copy of Hegel.sol to match.",
            contract.name,
            protocol::PROTOCOL_VERSION
        );
    }
    Ok(())
}

fn selector_calldata(function: &Function) -> Bytes {
    Bytes::copy_from_slice(function.selector().as_slice())
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

    // Each job builds its own engine context inside the worker thread that runs
    // it, so no engine handle is ever shared across threads. Results carry their
    // job index and are sorted afterwards, which keeps report order independent
    // of how the work happened to be scheduled.
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

fn configured_settings(
    ctx: &HegelContext,
    options: &Options,
    database_key: &str,
) -> Result<Settings> {
    let mut settings = match options.profile.as_deref() {
        Some(profile) => Settings::for_profile(ctx, profile)?,
        None => Settings::new(ctx)?,
    };
    settings
        .set_test_cases(options.test_cases)?
        .set_seed(options.seed)?
        .set_database(options.database.as_deref())?
        .set_database_key(database_key)?
        .set_phases(options.phases)?
        .set_verbosity(options.verbosity)?
        .set_print_blob(false)?;
    Ok(settings)
}

/// Drive one test through the engine: search, then replay each failure's blob to
/// recover the values behind it.
fn drive(
    test_name: String,
    options: &Options,
    mut execute: impl FnMut(&TestCase) -> Result<CaseExecution>,
) -> Result<Vec<TestReport>> {
    let ctx = HegelContext::new();
    let settings = configured_settings(&ctx, options, &test_name)?;

    if let Some(blob) = &options.reproduce {
        let mut tc = settings.test_case_from_blob(blob)?;
        let execution = execute(&tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        return Ok(vec![execution.report(test_name, Some(blob.clone()))]);
    }

    let mut run = settings.start_run()?;
    while let Some(mut tc) = run.next_test_case()? {
        let execution = execute(&tc)?;
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
            reports.push(TestReport::empty(
                test_name.clone(),
                TestStatus::Failed,
                Some(failure.origin()?),
            ));
            continue;
        };
        let mut tc = settings.test_case_from_blob(blob_value)?;
        let execution = execute(&tc)?;
        tc.mark_complete(execution.status, execution.origin.as_deref())?;
        reports.push(execution.report(test_name.clone(), blob));
    }
    if reports.is_empty() {
        reports.push(TestReport::empty(test_name, TestStatus::Passed, None));
    }
    Ok(reports)
}

fn run_test(
    prepared: &PreparedContract<'_>,
    function: &Function,
    options: &Options,
) -> Result<Vec<TestReport>> {
    let test_name = format!("{}.{}", prepared.contract.name, function.name);
    drive(test_name, options, |tc| {
        execute_case(prepared, function, tc, options)
    })
}

struct CaseExecution {
    console: Vec<String>,
    notes: Vec<String>,
    status: Status,
    origin: Option<String>,
    source: Option<SourceLocation>,
    trace: Vec<Draw>,
}

impl CaseExecution {
    fn report(self, test: String, blob: Option<String>) -> TestReport {
        TestReport {
            test,
            console: self.console,
            notes: self.notes,
            status: self.status.into(),
            origin: self.origin,
            source: self.source,
            blob,
            trace: self.trace,
            duration_ms: 0,
        }
    }
}

fn new_inspector<'a>(tc: &'a TestCase, address: Address, options: &Options) -> HegelInspector<'a> {
    let inspector = HegelInspector::new(tc, address);
    if options.coverage_target {
        inspector.collect_coverage()
    } else {
        inspector
    }
}

/// Report how much of the contract this example reached, so the engine can steer
/// generation toward examples that reach more. Targeting records an observation
/// rather than consuming choices, so a budget signal here is not a case failure.
fn report_coverage(tc: &TestCase, inspector: &HegelInspector<'_>) -> Result<()> {
    let Some(score) = inspector.coverage_score() else {
        return Ok(());
    };
    match tc.target(COVERAGE_LABEL, score as f64) {
        Ok(()) | Err(Error::Stop(_)) | Err(Error::Assume) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn execute_case(
    prepared: &PreparedContract<'_>,
    function: &Function,
    tc: &TestCase,
    options: &Options,
) -> Result<CaseExecution> {
    let (calldata, mut auto_trace) = match crate::autodraw::calldata(function, tc, options.limits())
    {
        Ok(prepared) => prepared,
        Err(Error::Stop(_)) => return Ok(aborted(Status::Overrun)),
        Err(Error::Assume) => return Ok(aborted(Status::Invalid)),
        Err(error) => return Err(error.into()),
    };
    let mut db = prepared.base.clone();
    let mut inspector = new_inspector(tc, prepared.address, options);
    let caller = evm::DEFAULT_CALLER;

    if let Some(setup) = &prepared.setup {
        let result = inspected_call(
            &mut db,
            &mut inspector,
            caller,
            prepared.address,
            setup.clone(),
        )?;
        if let Some(execution) = setup_failure(&mut inspector, result, prepared)? {
            return Ok(execution);
        }
    }
    let result = inspected_call(&mut db, &mut inspector, caller, prepared.address, calldata)?;

    let (status, origin, source) = match inspector.outcome.take() {
        Some(Abort::Invalid) => (Status::Invalid, None, None),
        Some(Abort::Overrun) => (Status::Overrun, None, None),
        Some(Abort::Error(message)) => return Err(anyhow!(message)),
        None => classify_with_abi(result, prepared.contract, inspector.source_pc()),
    };
    report_coverage(tc, &inspector)?;
    auto_trace.extend(inspector.trace);
    Ok(CaseExecution {
        console: inspector.console,
        notes: inspector.notes,
        status,
        origin,
        source,
        trace: auto_trace,
    })
}

fn aborted(status: Status) -> CaseExecution {
    CaseExecution {
        console: Vec::new(),
        notes: Vec::new(),
        status,
        origin: None,
        source: None,
        trace: Vec::new(),
    }
}

/// Classify the outcome of the fixture that runs before every example.
///
/// A generator signal (an assumption or an exhausted budget) is an ordinary
/// verdict on the example. A genuine revert is not: the fixture is the same for
/// every example, so it is a broken test rather than a discovered property
/// violation, and reporting it as a failure would attribute the fixture's revert
/// to the property under test.
fn setup_failure(
    inspector: &mut HegelInspector<'_>,
    result: ExecutionResult,
    prepared: &PreparedContract<'_>,
) -> Result<Option<CaseExecution>> {
    match inspector.outcome.take() {
        Some(Abort::Invalid) => Ok(Some(aborted(Status::Invalid))),
        Some(Abort::Overrun) => Ok(Some(aborted(Status::Overrun))),
        Some(Abort::Error(message)) => Err(anyhow!(message)),
        None if result.is_success() => Ok(None),
        None => {
            let (_, origin, source) =
                classify_with_abi(result, prepared.contract, inspector.source_pc());
            let where_ = source.map_or_else(String::new, |source| {
                format!(" at {}:{}:{}", source.path, source.line, source.column)
            });
            Err(anyhow!(
                "{}.setUp() reverted{where_}: {}",
                prepared.contract.name,
                origin.unwrap_or_else(|| "no reason given".into())
            ))
        }
    }
}

fn inspected_call(
    db: &mut Database,
    inspector: &mut HegelInspector<'_>,
    caller: Address,
    address: Address,
    calldata: Bytes,
) -> Result<ExecutionResult> {
    inspector.reset_source_trace();
    let context = Context::mainnet()
        .with_db(db)
        .with_cfg(evm::config())
        .modify_block_chained(|block| inspector.configure_block(block));
    let mut evm = context.build_mainnet_with_inspector(inspector);
    evm.inspect_tx_commit(evm::transaction(caller, evm::GAS_LIMIT, address, calldata))
        .map_err(|error| anyhow!("EVM transaction failed: {error:?}"))
}

fn classify_with_abi(
    result: ExecutionResult,
    contract: &Contract,
    pc: Option<usize>,
) -> (Status, Option<String>, Option<SourceLocation>) {
    let source = pc.and_then(|pc| contract.source_location(pc));
    match result {
        ExecutionResult::Success { .. } => (Status::Valid, None, None),
        ExecutionResult::Revert { output, .. } => {
            let reason =
                decode_custom_error(&output, contract).unwrap_or_else(|| decode_revert(&output));
            (Status::Interesting, Some(reason), source)
        }
        ExecutionResult::Halt { reason, .. } => (
            Status::Interesting,
            Some(format!("EVM halt: {reason:?}")),
            source,
        ),
    }
}

fn decode_custom_error(output: &[u8], contract: &Contract) -> Option<String> {
    use alloy_dyn_abi::JsonAbiExt;
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
        .map(format_value)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{}({rendered})", error.name))
}

pub fn decode_revert(output: &[u8]) -> String {
    use alloy_primitives::keccak256;
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

fn run_stateful(prepared: &PreparedContract<'_>, options: &Options) -> Result<Vec<TestReport>> {
    let conventions = &options.conventions;
    let contract = prepared.contract;
    let test_name = format!("{}.{STATEFUL_SUFFIX}", contract.name);
    let rules: Vec<_> = contract.rule_functions(conventions).collect();
    let invariants: Vec<_> = contract.invariant_functions(conventions).collect();
    drive(test_name, options, |tc| {
        execute_stateful_case(prepared, &rules, &invariants, tc, options)
    })
}

/// State carried across one stateful example: the database, the inspector, and
/// the trace built up as rules run.
struct Machine<'a, 'tc> {
    db: Database,
    inspector: HegelInspector<'tc>,
    trace: Vec<Draw>,
    tc: &'tc TestCase,
    prepared: &'a PreparedContract<'a>,
    options: &'a Options,
}

impl Machine<'_, '_> {
    fn call(&mut self, caller: Address, calldata: Bytes) -> Result<ExecutionResult> {
        inspected_call(
            &mut self.db,
            &mut self.inspector,
            caller,
            self.prepared.address,
            calldata,
        )
    }

    /// End the example, reporting what it reached before handing back the result.
    fn finish(
        mut self,
        status: Status,
        origin: Option<String>,
        source: Option<SourceLocation>,
    ) -> Result<CaseExecution> {
        report_coverage(self.tc, &self.inspector)?;
        self.trace.append(&mut self.inspector.trace);
        Ok(CaseExecution {
            console: self.inspector.console,
            notes: self.inspector.notes,
            status,
            origin,
            source,
            trace: self.trace,
        })
    }
}

fn execute_stateful_case(
    prepared: &PreparedContract<'_>,
    rules: &[&Function],
    invariants: &[&Function],
    tc: &TestCase,
    options: &Options,
) -> Result<CaseExecution> {
    let actors = options.actor_addresses();
    let mut machine = Machine {
        db: prepared.base.clone(),
        inspector: new_inspector(tc, prepared.address, options),
        trace: Vec::new(),
        tc,
        prepared,
        options,
    };

    if let Some(setup) = &prepared.setup {
        let result = machine.call(evm::DEFAULT_CALLER, setup.clone())?;
        if let Some(execution) = setup_failure(&mut machine.inspector, result, prepared)? {
            return Ok(execution);
        }
    }
    if let Some(execution) = check_invariants(&mut machine, invariants, tc, None)? {
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
    let sm = match tc.new_state_machine(
        &rule_names,
        &groups,
        &invariant_names,
        &always,
        prepared.step_count,
    ) {
        Ok(sm) => sm,
        Err(error) => return engine_case(error, machine),
    };
    debug_assert_eq!(sm.concurrency(), 1, "the EVM executes one call at a time");

    loop {
        let group = match sm.next_group(tc) {
            Ok(group) => group,
            Err(error) => return engine_case(error, machine),
        };
        if group.is_none() {
            if let Some(execution) = check_invariants(&mut machine, invariants, tc, None)? {
                return Ok(execution);
            }
            return machine.finish(Status::Valid, None, None);
        }

        loop {
            let index = match sm.next_rule(tc, 0) {
                Ok(Some(index)) => index,
                Ok(None) => break,
                Err(error) => return engine_case(error, machine),
            };
            let function = rules
                .get(index)
                .ok_or_else(|| anyhow!("Hegel selected unknown rule index {index}"))?;

            // Span the whole call, including who made it, so the shrinker can
            // drop a step outright rather than only simplifying its arguments.
            tc.start_span(&format!("hegel.sol.rule.{}", function.name))?;
            let caller = match draw_actor(tc, &actors) {
                Ok(caller) => caller,
                Err(status) => {
                    let _ = tc.stop_span(true);
                    return machine.finish(status, None, None);
                }
            };
            let (calldata, drawn) = match generated_calldata(function, tc, options.limits())? {
                Ok(prepared) => prepared,
                Err(status) => {
                    let _ = tc.stop_span(true);
                    return machine.finish(status, None, None);
                }
            };
            machine.trace.push(Draw {
                name: function.name.clone(),
                kind: "rule".into(),
                value: "()".into(),
            });
            if actors.len() > 1 {
                machine.trace.push(Draw {
                    name: "sender".into(),
                    kind: "sender".into(),
                    value: caller.to_string(),
                });
            }
            machine.trace.extend(drawn);

            machine.inspector.outcome = None;
            let result = machine.call(caller, calldata)?;
            match machine.inspector.outcome.take() {
                Some(Abort::Invalid) => {
                    tc.stop_span(true)?;
                    sm.rule_rejected(tc, 0)?;
                    continue;
                }
                Some(Abort::Overrun) => {
                    let _ = tc.stop_span(true);
                    return machine.finish(Status::Overrun, None, None);
                }
                Some(Abort::Error(message)) => return Err(anyhow!(message)),
                None => {}
            }

            let source_pc = machine.inspector.source_pc();
            let (status, origin, source) = classify_with_abi(result, prepared.contract, source_pc);
            if status == Status::Valid {
                tc.stop_span(false)?;
                continue;
            }
            if options.allow_rule_reverts {
                // A handler that reverts left no state behind, so the step never
                // happened: drop it and let the machine choose again rather than
                // spending the budget on it.
                tc.stop_span(true)?;
                sm.rule_rejected(tc, 0)?;
                continue;
            }
            tc.stop_span(false)?;
            return machine.finish(
                status,
                origin.map(|origin| format!("{}: {origin}", function.name)),
                source,
            );
        }

        for index in 0..invariants.len() {
            let should_check = match sm.should_check_invariant(tc, index) {
                Ok(value) => value,
                Err(error) => return engine_case(error, machine),
            };
            if should_check
                && let Some(execution) = check_invariants(
                    &mut machine,
                    &invariants[index..=index],
                    tc,
                    Some("sampled"),
                )?
            {
                return Ok(execution);
            }
        }
    }
}

/// Choose which account makes the next handler call.
fn draw_actor(tc: &TestCase, actors: &[Address]) -> std::result::Result<Address, Status> {
    if actors.len() <= 1 {
        return Ok(actors.first().copied().unwrap_or(evm::DEFAULT_CALLER));
    }
    match tc.integer(0, actors.len() as i64 - 1) {
        Ok(index) => Ok(actors[index as usize]),
        Err(Error::Stop(_)) => Err(Status::Overrun),
        Err(_) => Err(Status::Invalid),
    }
}

fn check_invariants(
    machine: &mut Machine<'_, '_>,
    invariants: &[&Function],
    tc: &TestCase,
    phase: Option<&str>,
) -> Result<Option<CaseExecution>> {
    for function in invariants {
        let (calldata, drawn) = match generated_calldata(function, tc, machine.options.limits())? {
            Ok(prepared) => prepared,
            Err(status) => return Ok(Some(take_case(machine, status, None, None)?)),
        };
        machine.trace.extend(drawn);
        let result = machine.call(evm::DEFAULT_CALLER, calldata)?;
        let label = phase.map_or_else(
            || function.name.clone(),
            |phase| format!("{} ({phase})", function.name),
        );
        match machine.inspector.outcome.take() {
            Some(Abort::Invalid) => {
                return Ok(Some(take_case(machine, Status::Invalid, None, None)?));
            }
            Some(Abort::Overrun) => {
                return Ok(Some(take_case(machine, Status::Overrun, None, None)?));
            }
            Some(Abort::Error(message)) => return Err(anyhow!(message)),
            None => {
                let source_pc = machine.inspector.source_pc();
                let (status, origin, source) =
                    classify_with_abi(result, machine.prepared.contract, source_pc);
                if status != Status::Valid {
                    return Ok(Some(take_case(
                        machine,
                        status,
                        origin.map(|origin| format!("{label}: {origin}")),
                        source,
                    )?));
                }
            }
        }
    }
    Ok(None)
}

/// Finish an example from a borrowed machine, draining its accumulated output.
fn take_case(
    machine: &mut Machine<'_, '_>,
    status: Status,
    origin: Option<String>,
    source: Option<SourceLocation>,
) -> Result<CaseExecution> {
    report_coverage(machine.tc, &machine.inspector)?;
    machine.trace.append(&mut machine.inspector.trace);
    Ok(CaseExecution {
        console: std::mem::take(&mut machine.inspector.console),
        notes: std::mem::take(&mut machine.inspector.notes),
        status,
        origin,
        source,
        trace: std::mem::take(&mut machine.trace),
    })
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

fn engine_case(error: Error, machine: Machine<'_, '_>) -> Result<CaseExecution> {
    match error {
        Error::Stop(_) => machine.finish(Status::Overrun, None, None),
        Error::Assume => machine.finish(Status::Invalid, None, None),
        error => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_addresses_avoid_the_precompile_range() {
        // Calling from a precompile address would put the sender and a built-in
        // contract at the same address, so handlers must never be given one.
        let actors = Options {
            actors: 8,
            ..Options::default()
        }
        .actor_addresses();
        assert_eq!(actors.len(), 8);
        assert!(actors.iter().all(|actor| actor[0] == 0xac));
        assert_eq!(
            actors
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            8
        );
    }

    #[test]
    fn a_single_actor_run_keeps_the_default_caller_semantics() {
        let actors = Options {
            actors: 0,
            ..Options::default()
        }
        .actor_addresses();
        assert_eq!(actors.len(), 1, "a run always has somewhere to call from");
    }

    #[test]
    fn shards_are_disjoint_and_together_cover_every_test() {
        let names: Vec<String> = (0..200)
            .map(|index| format!("Contract.test_{index}"))
            .collect();
        let total = 4;
        let mut seen = std::collections::BTreeSet::new();
        for number in 1..=total {
            let shard = Shard::new(number, total).unwrap();
            for name in &names {
                if shard.contains(name) {
                    assert!(seen.insert(name.clone()), "{name} landed in two shards");
                }
            }
        }
        assert_eq!(seen.len(), names.len());
    }

    #[test]
    fn shard_bounds_are_rejected_rather_than_silently_clamped() {
        assert!(Shard::new(0, 4).is_err());
        assert!(Shard::new(5, 4).is_err());
        assert!(Shard::new(1, 0).is_err());
    }
}
