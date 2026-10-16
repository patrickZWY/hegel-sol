use std::collections::{BTreeMap, HashMap, HashSet};

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, B256, I256, Log, U256};
use revm::{
    Context as RevmContext, Inspector,
    context::{
        BlockEnv, CfgEnv, Journal, JournalTr, TxEnv, journaled_state::account::JournaledAccountTr,
    },
    database_interface::Database,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, Interpreter, InterpreterResult,
        interpreter::EthInterpreter, interpreter_types::Jumps,
    },
    primitives::Bytes,
};
use serde::Serialize;

use crate::{
    engine::{
        Error, Pool, TestCase,
        integer::{i256_from_le_signed, i256_to_le_signed, u256_from_le_signed, u256_to_le_signed},
    },
    protocol::{self, Assertion, Op, VmOp},
    values::{self, format_value},
};

/// What an intercepted cheatcode hands back to the EVM.
enum VmOutcome {
    Return(Bytes),
    /// The cheatcode itself reverts with this data, as a failed assertion does,
    /// so the test sees an ordinary revert it can propagate or expect.
    Revert(Bytes),
}

/// JUMPDEST. Recording coverage only at basic-block entries keeps the per-opcode
/// `step` hook cheap while still distinguishing the paths a test took.
const JUMPDEST: u8 = 0x5b;
const REVERT: u8 = 0xfd;
const INVALID: u8 = 0xfe;

/// Return data handed back in place of a revert that `vm.expectRevert` matched.
/// The test goes on to ABI-decode whatever the call declares it returns, and an
/// empty buffer would make that decoding revert. Zeros decode as zero, false, or
/// empty for every type, which is what Foundry hands back too.
static MATCHED_REVERT_OUTPUT: [u8; 8192] = [0; 8192];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Draw {
    pub name: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Abort {
    Invalid,
    Overrun,
    Error(String),
}

pub struct HegelInspector<'a> {
    tc: &'a TestCase,
    pub trace: Vec<Draw>,
    pub console: Vec<String>,
    pub notes: Vec<String>,
    pub outcome: Option<Abort>,
    draw_index: usize,
    pools: HashMap<u64, PoolValues>,
    next_pool: u64,
    next_prank: Option<Address>,
    persistent_prank: Option<Address>,
    expected_revert: Option<Option<Vec<u8>>>,
    active_expect: Option<(Address, Option<Vec<u8>>)>,
    expected_emits: Vec<ExpectedEmit>,
    /// The call whose logs must satisfy the pending event expectations, as the
    /// depth it was made from and its target.
    emit_check: Option<(usize, Address)>,
    labels: HashMap<Address, String>,
    /// Best score observed under each `hegel.target` label. The engine accepts
    /// one observation per label per example, while an invariant observes on
    /// every check, so the runner keeps the maximum and reports it at the end.
    targets: BTreeMap<String, f64>,
    timestamp: Option<U256>,
    block_number: Option<U256>,
    /// Call depth of the transaction's own entry frame. Pranks apply to the
    /// calls a test makes, never to the transaction itself, so a prank left
    /// open by `setUp` must not replace the sender the runner chose.
    entry_depth: Option<usize>,
    trace_target: Address,
    last_pc: Option<usize>,
    failure_pc: Option<usize>,
    coverage: Option<HashSet<u64>>,
}

struct PoolValues {
    engine: Pool,
    values: HashMap<i64, B256>,
}

/// One `vm.expectEmit`, filled in by the event the test emits next and satisfied
/// by a matching event from the call after that.
struct ExpectedEmit {
    /// Call depth the expectation was declared at, so only the declaring frame's
    /// own event fills it and only calls made from that frame are checked.
    depth: usize,
    /// Whether topics 1 to 3 and the data must match; the event signature in
    /// topic 0 always must.
    checks: [bool; 4],
    emitter: Option<Address>,
    log: Option<Log>,
    found: bool,
}

impl ExpectedEmit {
    fn matches(&self, log: &Log) -> bool {
        let Some(expected) = &self.log else {
            return false;
        };
        if self.emitter.is_some_and(|emitter| emitter != log.address) {
            return false;
        }
        let topics = log.topics();
        let expected_topics = expected.topics();
        if topics.len() != expected_topics.len() || topics.first() != expected_topics.first() {
            return false;
        }
        for (index, check) in self.checks[..3].iter().enumerate() {
            if *check && topics.get(index + 1) != expected_topics.get(index + 1) {
                return false;
            }
        }
        !self.checks[3] || log.data.data == expected.data.data
    }
}

impl<'a> HegelInspector<'a> {
    pub fn new(tc: &'a TestCase, trace_target: Address) -> Self {
        Self {
            tc,
            trace: Vec::new(),
            console: Vec::new(),
            notes: Vec::new(),
            outcome: None,
            draw_index: 0,
            pools: HashMap::new(),
            next_pool: 0,
            next_prank: None,
            persistent_prank: None,
            expected_revert: None,
            active_expect: None,
            expected_emits: Vec::new(),
            emit_check: None,
            labels: HashMap::new(),
            targets: BTreeMap::new(),
            timestamp: None,
            block_number: None,
            entry_depth: None,
            trace_target,
            last_pc: None,
            failure_pc: None,
            coverage: None,
        }
    }

    /// Collect basic-block coverage so the runner can report it to the engine as
    /// a target score. Off by default because it costs work on every jump.
    pub fn collect_coverage(mut self) -> Self {
        self.coverage = Some(HashSet::new());
        self
    }

    /// The best score seen under each `hegel.target` label, for the engine.
    pub fn targets(&self) -> impl Iterator<Item = (&str, f64)> {
        self.targets
            .iter()
            .map(|(label, score)| (label.as_str(), *score))
    }

    /// Number of distinct basic blocks entered so far, or `None` when coverage
    /// collection is disabled.
    pub fn coverage_score(&self) -> Option<usize> {
        self.coverage.as_ref().map(HashSet::len)
    }

    /// Forget what the previous transaction left behind: its source trace and
    /// its entry depth.
    pub fn reset_source_trace(&mut self) {
        self.last_pc = None;
        self.failure_pc = None;
        self.entry_depth = None;
    }

    /// Move the chain forward between stateful steps. The overrides persist for
    /// the rest of the example, like a `roll` would, and start from revm's
    /// default block when nothing set them.
    pub fn advance_block(&mut self, blocks: u64, seconds: u64) {
        let default = BlockEnv::default();
        let number = self.block_number.unwrap_or(default.number);
        let timestamp = self.timestamp.unwrap_or(default.timestamp);
        self.block_number = Some(number + U256::from(blocks));
        self.timestamp = Some(timestamp + U256::from(seconds));
    }

    pub fn source_pc(&self) -> Option<usize> {
        self.failure_pc.or(self.last_pc)
    }

    fn auto_name(&mut self) -> String {
        let name = format!("draw_{}", self.draw_index);
        self.draw_index += 1;
        name
    }

    pub fn configure_block(&self, block: &mut BlockEnv) {
        if let Some(timestamp) = self.timestamp {
            block.timestamp = timestamp;
        }
        if let Some(number) = self.block_number {
            block.number = number;
        }
    }

    /// An expectation left unmet once the call under test has returned, as the
    /// failure to report. Expectations only make sense within the call that set
    /// them, so taking them clears the slate for the next one.
    pub fn take_unmet_expectation(&mut self) -> Option<String> {
        let unmet = self
            .expected_emits
            .iter()
            .find(|expected| !expected.found)
            .map(|expected| match &expected.log {
                Some(log) => format!(
                    "expected event {} from {} was not emitted",
                    log.topics()
                        .first()
                        .map_or_else(|| "<no topics>".to_owned(), ToString::to_string),
                    expected
                        .emitter
                        .map_or_else(|| "any emitter".to_owned(), |emitter| self.display(emitter))
                ),
                None => "vm.expectEmit was not followed by an event to expect".to_owned(),
            });
        self.expected_emits.clear();
        self.emit_check = None;
        unmet
    }

    /// Human-readable name for an address, honouring `vm.label`.
    fn display(&self, address: Address) -> String {
        match self.labels.get(&address) {
            Some(label) => format!("{label} ({address})"),
            None => address.to_string(),
        }
    }

    fn dispatch(&mut self, data: &[u8]) -> Result<Bytes, Abort> {
        let (selector, args) = split_selector(data).ok_or_else(|| {
            Abort::Error("Hegel call arrived without a four-byte selector".into())
        })?;
        let call = protocol::hegel_call(selector).ok_or_else(|| unknown_selector(selector))?;
        let values = call.decode(args).map_err(Abort::Error)?;
        let (declared_name, args) = call.split_name(&values).map_err(Abort::Error)?;
        let name = match declared_name {
            Some(name) => name.to_owned(),
            None => self.auto_name(),
        };

        match call.op {
            Op::DrawUint256 => {
                let value = self.draw_uint(arg_u256(args, 0)?, arg_u256(args, 1)?)?;
                self.record(name, "uint256", value.to_string());
                Ok(encode(DynSolValue::Uint(value, 256)))
            }
            Op::DrawInt256 => {
                let lo = arg_i256(args, 0)?;
                let hi = arg_i256(args, 1)?;
                let raw = self
                    .tc
                    .integer_big(&i256_to_le_signed(lo), &i256_to_le_signed(hi))
                    .map_err(map_engine)?;
                let value = i256_from_le_signed(&raw)
                    .ok_or_else(|| Abort::Error("invalid int256 from Hegel".into()))?;
                self.record(name, "int256", value.to_string());
                Ok(encode(DynSolValue::Int(value, 256)))
            }
            Op::DrawBool => {
                let value = self.tc.boolean(0.5).map_err(map_engine)?;
                self.record(name, "bool", value.to_string());
                Ok(encode(DynSolValue::Bool(value)))
            }
            Op::DrawAddress => {
                let value = self.draw_uint(U256::ZERO, values::address_max())?;
                let address = Address::from_slice(&value.to_be_bytes::<32>()[12..]);
                self.record(name, "address", address.to_string());
                Ok(encode(DynSolValue::Address(address)))
            }
            Op::DrawBytes32 => {
                let value = self.tc.bytes(32, 32).map_err(map_engine)?;
                let word = B256::from_slice(&value);
                self.record(name, "bytes32", word.to_string());
                Ok(encode(DynSolValue::FixedBytes(word, 32)))
            }
            Op::DrawBytes => {
                let min = arg_u64(args, 0)?;
                let max = arg_u64(args, 1)?;
                let value = self.tc.bytes(min, max).map_err(map_engine)?;
                let value = DynSolValue::Bytes(value);
                self.record(name, "bytes", format_value(&value));
                Ok(encode(value))
            }
            Op::DrawString => {
                let max = arg_u64(args, 0)?;
                let bytes = self.tc.bytes(0, max).map_err(map_engine)?;
                let value = DynSolValue::String(values::printable_string(bytes));
                self.record(name, "string", format_value(&value));
                Ok(encode(value))
            }
            Op::Assume => {
                if arg_bool(args, 0)? {
                    Ok(Bytes::new())
                } else {
                    Err(Abort::Invalid)
                }
            }
            Op::StartSpan => {
                self.tc.start_span(arg_str(args, 0)?).map_err(map_engine)?;
                Ok(Bytes::new())
            }
            Op::StopSpan => {
                self.tc.stop_span(arg_bool(args, 0)?).map_err(map_engine)?;
                Ok(Bytes::new())
            }
            Op::Note => {
                let text = arg_str(args, 0)?.to_owned();
                self.tc.note(&text).map_err(map_engine)?;
                self.notes.push(text);
                Ok(Bytes::new())
            }
            Op::Target => {
                // `target(int256)` carries no label of its own; give every such
                // call the same one so their scores form a single objective.
                let label = declared_name.unwrap_or("hegel.sol.target");
                let score = values::i256_as_f64(arg_i256(args, 0)?);
                self.targets
                    .entry(label.to_owned())
                    .and_modify(|best| *best = best.max(score))
                    .or_insert(score);
                Ok(Bytes::new())
            }
            Op::RecordEvent => {
                self.tc.event(&name).map_err(map_engine)?;
                self.record(name, "event", String::new());
                Ok(Bytes::new())
            }
            Op::PoolNew => {
                let id = self.next_pool;
                self.next_pool = self
                    .next_pool
                    .checked_add(1)
                    .ok_or_else(|| Abort::Error("pool id overflow".into()))?;
                let engine = self.tc.new_pool().map_err(map_engine)?;
                self.pools.insert(
                    id,
                    PoolValues {
                        engine,
                        values: HashMap::new(),
                    },
                );
                self.record(name, "pool", id.to_string());
                Ok(encode(DynSolValue::Uint(U256::from(id), 256)))
            }
            Op::PoolAdd => {
                let id = arg_u64(args, 0)?;
                let value = arg_b256(args, 1)?;
                let tc = self.tc;
                let pool = self.pool(id)?;
                let variable = pool.engine.add(tc).map_err(map_engine)?;
                pool.values.insert(variable, value);
                Ok(Bytes::new())
            }
            Op::PoolPick => {
                let id = arg_u64(args, 0)?;
                let consume = arg_bool(args, 1)?;
                let tc = self.tc;
                let pool = self.pool(id)?;
                let variable = pool.engine.generate(tc, consume).map_err(map_engine)?;
                let value = pool.values.get(&variable).copied().ok_or_else(|| {
                    Abort::Error(format!("pool {id} returned unknown variable {variable}"))
                })?;
                if consume {
                    pool.values.remove(&variable);
                }
                self.record(format!("pool_{id}"), "bytes32", value.to_string());
                Ok(encode(DynSolValue::FixedBytes(value, 32)))
            }
        }
    }

    fn draw_uint(&self, lo: U256, hi: U256) -> Result<U256, Abort> {
        let raw = self
            .tc
            .integer_big(&u256_to_le_signed(lo), &u256_to_le_signed(hi))
            .map_err(map_engine)?;
        u256_from_le_signed(&raw).ok_or_else(|| Abort::Error("invalid uint256 from Hegel".into()))
    }

    fn pool(&mut self, id: u64) -> Result<&mut PoolValues, Abort> {
        self.pools
            .get_mut(&id)
            .ok_or_else(|| Abort::Error(format!("unknown Hegel pool {id}")))
    }

    fn dispatch_vm<DB: Database>(
        &mut self,
        ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>,
        data: &[u8],
    ) -> Result<VmOutcome, Abort> {
        let (selector, args) = split_selector(data).ok_or_else(|| {
            Abort::Error("cheatcode call arrived without a four-byte selector".into())
        })?;
        let call = protocol::vm_call(selector).ok_or_else(|| {
            Abort::Error(format!(
                "unsupported Foundry cheatcode with selector 0x{}; hegel-sol emulates a subset of \
                 forge-std in-process",
                alloy_primitives::hex::encode(selector)
            ))
        })?;
        let args = call.decode(args).map_err(Abort::Error)?;
        let args = args.as_slice();

        match call.op {
            VmOp::Prank => self.next_prank = Some(arg_address(args, 0)?),
            VmOp::StartPrank => self.persistent_prank = Some(arg_address(args, 0)?),
            VmOp::StopPrank => {
                self.persistent_prank = None;
                self.next_prank = None;
            }
            VmOp::Deal => {
                let address = arg_address(args, 0)?;
                let balance = arg_u256(args, 1)?;
                let mut account = ctx
                    .journaled_state
                    .load_account_mut(address)
                    .map_err(|_| Abort::Error("failed to load account for vm.deal".into()))?;
                account.data.set_balance(balance);
            }
            VmOp::Warp => {
                let value = arg_u256(args, 0)?;
                ctx.block.timestamp = value;
                self.timestamp = Some(value);
            }
            VmOp::Roll => {
                let value = arg_u256(args, 0)?;
                ctx.block.number = value;
                self.block_number = Some(value);
            }
            VmOp::GetBlockNumber => {
                return Ok(VmOutcome::Return(encode(DynSolValue::Uint(
                    ctx.block.number,
                    256,
                ))));
            }
            VmOp::GetBlockTimestamp => {
                return Ok(VmOutcome::Return(encode(DynSolValue::Uint(
                    ctx.block.timestamp,
                    256,
                ))));
            }
            VmOp::ExpectRevertAny => self.expected_revert = Some(None),
            VmOp::ExpectRevertData => self.expected_revert = Some(Some(arg_data(args, 0)?)),
            VmOp::ExpectEmit => {
                // The overloads differ only in which parts they check and whether
                // they pin the emitter: (), (address), (bool x4), (bool x4, address).
                let flags = args.len() >= 4;
                let checks = if flags {
                    [
                        arg_bool(args, 0)?,
                        arg_bool(args, 1)?,
                        arg_bool(args, 2)?,
                        arg_bool(args, 3)?,
                    ]
                } else {
                    [true; 4]
                };
                let emitter = match args.len() {
                    1 => Some(arg_address(args, 0)?),
                    5 => Some(arg_address(args, 4)?),
                    _ => None,
                };
                self.expected_emits.push(ExpectedEmit {
                    depth: ctx.journaled_state.depth(),
                    checks,
                    emitter,
                    log: None,
                    found: false,
                });
            }
            VmOp::Addr => {
                let key = signing_key(arg_u256(args, 0)?, "vm.addr")?;
                return Ok(VmOutcome::Return(encode(DynSolValue::Address(
                    key_address(&key),
                ))));
            }
            VmOp::Store => {
                let address = arg_address(args, 0)?;
                let key = U256::from_be_bytes(arg_b256(args, 1)?.0);
                let value = U256::from_be_bytes(arg_b256(args, 2)?.0);
                ctx.journaled_state
                    .load_account(address)
                    .map_err(|_| Abort::Error("failed to load account for vm.store".into()))?;
                ctx.journaled_state
                    .sstore(address, key, value)
                    .map_err(|_| Abort::Error("vm.store could not write the slot".into()))?;
            }
            VmOp::Load => {
                let address = arg_address(args, 0)?;
                let key = U256::from_be_bytes(arg_b256(args, 1)?.0);
                ctx.journaled_state
                    .load_account(address)
                    .map_err(|_| Abort::Error("failed to load account for vm.load".into()))?;
                let value = ctx
                    .journaled_state
                    .sload(address, key)
                    .map_err(|_| Abort::Error("vm.load could not read the slot".into()))?;
                return Ok(VmOutcome::Return(encode(DynSolValue::FixedBytes(
                    B256::from(value.data),
                    32,
                ))));
            }
            VmOp::Assert(assertion) => {
                if let Some(message) = failed_assertion(assertion, call.signature, args)? {
                    return Ok(VmOutcome::Revert(revert_message(&message)));
                }
            }
            VmOp::Sign => {
                let key = signing_key(arg_u256(args, 0)?, "vm.sign")?;
                let digest = arg_b256(args, 1)?;
                let (signature, recovery) = key.sign_prehash_recoverable(digest.as_slice());
                let (r, s) = signature.split_bytes();
                return Ok(VmOutcome::Return(encode_params(vec![
                    DynSolValue::Uint(U256::from(27 + recovery.to_byte()), 8),
                    DynSolValue::FixedBytes(B256::from_slice(&r), 32),
                    DynSolValue::FixedBytes(B256::from_slice(&s), 32),
                ])));
            }
            VmOp::Label => {
                self.labels
                    .insert(arg_address(args, 0)?, arg_str(args, 1)?.to_owned());
            }
            VmOp::Assume => {
                if !arg_bool(args, 0)? {
                    return Err(Abort::Invalid);
                }
            }
        }
        Ok(VmOutcome::Return(Bytes::new()))
    }

    /// Render a DSTest diagnostic event the way Forge prints it, so a classic
    /// assertion's explanation reaches the report alongside `console.log`.
    fn capture_dstest_log(&mut self, log: &Log) {
        let Some(call) = log.topics().first().and_then(protocol::dstest_log) else {
            return;
        };
        let rendered = match call.decode(&log.data.data) {
            Ok(values) => {
                let mut parts = values.iter().map(|value| match value {
                    DynSolValue::Address(address) => self.display(*address),
                    DynSolValue::String(text) => text.clone(),
                    other => format_value(other),
                });
                match (parts.next(), parts.next()) {
                    (Some(key), Some(value)) => format!("{key}: {value}"),
                    (Some(value), None) => value,
                    _ => String::new(),
                }
            }
            Err(error) => format!("<{error}>"),
        };
        self.console.push(format!("log: {rendered}"));
    }

    fn capture_console(&mut self, data: &[u8]) {
        self.console.push(self.decode_console(data));
    }

    fn decode_console(&self, data: &[u8]) -> String {
        let Some((selector, args)) = split_selector(data) else {
            return "console.log: <invalid calldata>".into();
        };
        let Some(call) = protocol::console_call(selector) else {
            return format!(
                "console.log: <unsupported overload 0x{}>",
                alloy_primitives::hex::encode(selector)
            );
        };
        match call.decode(args) {
            Ok(values) => {
                let rendered = values
                    .iter()
                    .map(|value| match value {
                        // `vm.label` exists so addresses read as names; honour it
                        // here too rather than printing raw hex twice over.
                        DynSolValue::Address(address) => self.display(*address),
                        DynSolValue::String(text) => text.clone(),
                        other => format_value(other),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("console.log: {rendered}")
            }
            Err(error) => format!("console.log: <{error}>"),
        }
    }

    fn record(&mut self, name: String, kind: &str, value: String) {
        self.trace.push(Draw {
            name,
            kind: kind.into(),
            value,
        });
    }
}

impl<DB: Database> Inspector<RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>, EthInterpreter>
    for HegelInspector<'_>
{
    fn step(
        &mut self,
        interp: &mut Interpreter<EthInterpreter>,
        _ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>,
    ) {
        let opcode = interp.bytecode.opcode();
        let address = interp.input.bytecode_address;
        if address == Some(self.trace_target) {
            let pc = interp.bytecode.pc();
            self.last_pc = Some(pc);
            if matches!(opcode, REVERT | INVALID) {
                self.failure_pc = Some(pc);
            }
        }
        if opcode == JUMPDEST
            && let Some(coverage) = &mut self.coverage
        {
            coverage.insert(block_id(address, interp.bytecode.pc()));
        }
    }

    fn call(
        &mut self,
        ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        if let Some(timestamp) = self.timestamp {
            ctx.block.timestamp = timestamp;
        }
        if let Some(number) = self.block_number {
            ctx.block.number = number;
        }
        if inputs.target_address == protocol::console_address() {
            let input = inputs.input.bytes(ctx);
            self.capture_console(&input);
            return Some(intercepted(inputs, InstructionResult::Return, Bytes::new()));
        }
        if inputs.target_address == protocol::hevm_address() {
            let input = inputs.input.bytes(ctx);
            let (instruction, output) = match self.dispatch_vm(ctx, &input) {
                Ok(VmOutcome::Return(output)) => (InstructionResult::Return, output),
                Ok(VmOutcome::Revert(output)) => (InstructionResult::Revert, output),
                Err(abort) => {
                    self.outcome = Some(abort);
                    (InstructionResult::Revert, Bytes::new())
                }
            };
            return Some(intercepted(inputs, instruction, output));
        }
        if inputs.target_address == protocol::hegel_address() {
            let input = inputs.input.bytes(ctx);
            let (instruction, output) = match self.dispatch(&input) {
                Ok(output) => (InstructionResult::Return, output),
                Err(abort) => {
                    self.outcome = Some(abort);
                    (InstructionResult::Revert, Bytes::new())
                }
            };
            return Some(intercepted(inputs, instruction, output));
        }
        // revm reports the transaction's entry frame one deeper than zero, so the
        // depth seen first is the baseline rather than a constant.
        let depth = ctx.journaled_state.depth();
        let entry_depth = *self.entry_depth.get_or_insert(depth);
        if depth > entry_depth
            && let Some(sender) = self.next_prank.take().or(self.persistent_prank)
        {
            inputs.caller = sender;
        }
        if let Some(expected) = self.expected_revert.take() {
            self.active_expect = Some((inputs.target_address, expected));
        }
        // The first call made from the frame that declared event expectations is
        // the one whose events must satisfy them.
        if self.emit_check.is_none()
            && self
                .expected_emits
                .iter()
                .any(|expected| expected.log.is_some() && expected.depth == depth)
        {
            self.emit_check = Some((depth, inputs.target_address));
        }
        None
    }

    fn log(&mut self, ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>, log: Log) {
        if log.address == self.trace_target {
            self.capture_dstest_log(&log);
        }
        let depth = ctx.journaled_state.depth();
        // Until the call under check starts, an event from the declaring frame is
        // the example of what to expect, not a candidate.
        if self.emit_check.is_none()
            && let Some(expected) = self
                .expected_emits
                .iter_mut()
                .find(|expected| expected.log.is_none())
        {
            if expected.depth == depth {
                expected.log = Some(log);
            }
            return;
        }
        if self.emit_check.is_some()
            && let Some(expected) = self
                .expected_emits
                .iter_mut()
                .find(|expected| !expected.found)
            && expected.matches(&log)
        {
            expected.found = true;
        }
    }

    fn call_end(
        &mut self,
        _ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>,
        inputs: &CallInputs,
        result: &mut CallOutcome,
    ) {
        let Some((target, expected)) = self.active_expect.take() else {
            return;
        };
        if inputs.target_address != target {
            self.active_expect = Some((target, expected));
            return;
        }
        let reverted = !result.result.is_ok();
        let matches = expected
            .as_ref()
            .is_none_or(|expected| result.result.output.starts_with(expected));
        if reverted && matches {
            result.result.result = InstructionResult::Return;
            result.result.output = Bytes::from_static(&MATCHED_REVERT_OUTPUT);
        } else {
            result.result.result = InstructionResult::Revert;
            result.result.output = revert_message("hegel-sol: expected revert did not match");
        }
    }
}

/// Reproduce a sender's address and signing key from a raw private key, as
/// `vm.addr` and `vm.sign` do.
fn signing_key(key: U256, cheatcode: &str) -> Result<k256::ecdsa::SigningKey, Abort> {
    k256::ecdsa::SigningKey::from_slice(&key.to_be_bytes::<32>()).map_err(|_| {
        Abort::Error(format!(
            "{cheatcode}: the private key is zero or not below the secp256k1 curve order"
        ))
    })
}

fn key_address(key: &k256::ecdsa::SigningKey) -> Address {
    let point = key.verifying_key().to_sec1_point(false);
    Address::from_raw_public_key(&point.as_bytes()[1..])
}

/// Evaluate a `vm.assert*` call, returning the message to revert with when it
/// fails. Messaged overloads end in a string; decimal overloads carry a decimals
/// argument before it, which only affects Forge's rendering and is dropped here.
fn failed_assertion(
    assertion: Assertion,
    signature: &str,
    args: &[DynSolValue],
) -> Result<Option<String>, Abort> {
    let mut args = args.to_vec();
    let message = match args.last() {
        Some(DynSolValue::String(_)) => match args.pop() {
            Some(DynSolValue::String(text)) => Some(text),
            _ => None,
        },
        _ => None,
    };
    if signature.contains("Decimal") {
        args.pop();
    }
    let value = |index: usize| args.get(index).map(format_value).unwrap_or_default();
    let numeric = |index: usize| -> Result<I256, Abort> {
        match args.get(index) {
            Some(DynSolValue::Uint(value, _)) => {
                I256::checked_from_sign_and_abs(alloy_primitives::Sign::Positive, *value)
                    .ok_or_else(|| Abort::Error(format!("{signature}: value does not fit int256")))
            }
            Some(DynSolValue::Int(value, _)) => Ok(*value),
            _ => Err(mismatch(index, "a number")),
        }
    };
    let failure = match assertion {
        Assertion::True => (!arg_bool(&args, 0)?).then(|| "assertion failed".to_owned()),
        Assertion::False => arg_bool(&args, 0)?.then(|| "assertion failed".to_owned()),
        Assertion::Eq => {
            (args.first() != args.get(1)).then(|| format!("{} != {}", value(0), value(1)))
        }
        Assertion::NotEq => {
            (args.first() == args.get(1)).then(|| format!("{} == {}", value(0), value(1)))
        }
        Assertion::Lt | Assertion::Le | Assertion::Gt | Assertion::Ge => {
            let (left, right) = (numeric(0)?, numeric(1)?);
            let (holds, violated) = match assertion {
                Assertion::Lt => (left < right, ">="),
                Assertion::Le => (left <= right, ">"),
                Assertion::Gt => (left > right, "<="),
                _ => (left >= right, "<"),
            };
            (!holds).then(|| format!("{} {violated} {}", value(0), value(1)))
        }
        Assertion::ApproxEqAbs => {
            let (left, right) = (numeric(0)?, numeric(1)?);
            let max_delta = arg_u256(&args, 2)?;
            let delta = absolute_difference(left, right);
            (delta > max_delta).then(|| {
                format!(
                    "{} !~= {} (max delta: {max_delta}, real delta: {delta})",
                    value(0),
                    value(1)
                )
            })
        }
        Assertion::ApproxEqRel => {
            let (left, right) = (numeric(0)?, numeric(1)?);
            let max_percent = arg_u256(&args, 2)?;
            // Forge expresses the bound as a fraction of `right` scaled by 1e18,
            // and a zero reference requires exact equality.
            let delta = absolute_difference(left, right);
            let scale = U256::from(10).pow(U256::from(18));
            let percent = if right.is_zero() {
                if left == right { U256::ZERO } else { U256::MAX }
            } else {
                delta
                    .checked_mul(scale)
                    .map_or(U256::MAX, |scaled| scaled / right.unsigned_abs())
            };
            (percent > max_percent).then(|| {
                format!(
                    "{} !~= {} (max delta: {max_percent}, real delta: {percent})",
                    value(0),
                    value(1)
                )
            })
        }
    };
    Ok(failure.map(|detail| match message {
        Some(message) => format!("{message}: {detail}"),
        None => format!("assertion failed: {detail}"),
    }))
}

/// `|left - right|` as a magnitude, saturating where int256 cannot hold it.
fn absolute_difference(left: I256, right: I256) -> U256 {
    let (larger, smaller) = if left >= right {
        (left, right)
    } else {
        (right, left)
    };
    larger
        .checked_sub(smaller)
        .map_or(U256::MAX, |delta| delta.unsigned_abs())
}

/// Revert data a Solidity `revert("message")` would produce, so the runner's own
/// verdicts decode like any other reason string.
fn revert_message(message: &str) -> Bytes {
    let mut data = protocol::selector("Error(string)").to_vec();
    data.extend(
        DynSolValue::Tuple(vec![DynSolValue::String(message.to_owned())]).abi_encode_params(),
    );
    Bytes::from(data)
}

/// Identify a basic block by the contract it belongs to and its offset. Address
/// bits are folded in so the same offset in two contracts counts separately;
/// collisions only blunt the coverage score, never change a verdict.
fn block_id(address: Option<Address>, pc: usize) -> u64 {
    let address = address.unwrap_or_default();
    let high = u64::from_be_bytes(
        address[12..20]
            .try_into()
            .expect("an address is twenty bytes"),
    );
    high.rotate_left(32) ^ pc as u64
}

/// Answer an intercepted call without entering the EVM. Draws cost no gas: the
/// full limit is returned so an example's budget is the engine's alone.
fn intercepted(inputs: &CallInputs, instruction: InstructionResult, output: Bytes) -> CallOutcome {
    CallOutcome::new(
        InterpreterResult::new(instruction, output, Gas::new(inputs.gas_limit)),
        inputs.return_memory_offset.clone(),
    )
}

fn split_selector(data: &[u8]) -> Option<(&[u8], &[u8])> {
    (data.len() >= 4).then(|| data.split_at(4))
}

fn unknown_selector(selector: &[u8]) -> Abort {
    Abort::Error(format!(
        "unknown Hegel selector 0x{}; this usually means the project's Hegel.sol is from a \
         different release than the runner (expected protocol version {})",
        alloy_primitives::hex::encode(selector),
        protocol::PROTOCOL_VERSION
    ))
}

fn encode(value: DynSolValue) -> Bytes {
    Bytes::from(value.abi_encode())
}

/// Encode several return values, as a function returning a tuple does.
fn encode_params(values: Vec<DynSolValue>) -> Bytes {
    Bytes::from(DynSolValue::Tuple(values).abi_encode_params())
}

fn mismatch(index: usize, expected: &str) -> Abort {
    Abort::Error(format!("protocol argument {index} is not {expected}"))
}

fn arg_u256(args: &[DynSolValue], index: usize) -> Result<U256, Abort> {
    match args.get(index) {
        Some(DynSolValue::Uint(value, _)) => Ok(*value),
        _ => Err(mismatch(index, "an unsigned integer")),
    }
}

fn arg_i256(args: &[DynSolValue], index: usize) -> Result<I256, Abort> {
    match args.get(index) {
        Some(DynSolValue::Int(value, _)) => Ok(*value),
        _ => Err(mismatch(index, "a signed integer")),
    }
}

fn arg_u64(args: &[DynSolValue], index: usize) -> Result<u64, Abort> {
    u64::try_from(arg_u256(args, index)?)
        .map_err(|_| Abort::Error(format!("protocol argument {index} does not fit u64")))
}

fn arg_bool(args: &[DynSolValue], index: usize) -> Result<bool, Abort> {
    match args.get(index) {
        Some(DynSolValue::Bool(value)) => Ok(*value),
        _ => Err(mismatch(index, "a boolean")),
    }
}

fn arg_address(args: &[DynSolValue], index: usize) -> Result<Address, Abort> {
    match args.get(index) {
        Some(DynSolValue::Address(value)) => Ok(*value),
        _ => Err(mismatch(index, "an address")),
    }
}

fn arg_str(args: &[DynSolValue], index: usize) -> Result<&str, Abort> {
    match args.get(index) {
        Some(DynSolValue::String(value)) => Ok(value),
        _ => Err(mismatch(index, "a string")),
    }
}

fn arg_b256(args: &[DynSolValue], index: usize) -> Result<B256, Abort> {
    match args.get(index) {
        Some(DynSolValue::FixedBytes(value, 32)) => Ok(*value),
        _ => Err(mismatch(index, "a bytes32")),
    }
}

/// Read a `bytes` or `bytesN` argument as raw bytes. `vm.expectRevert` accepts
/// both, so the two overloads share one handler.
fn arg_data(args: &[DynSolValue], index: usize) -> Result<Vec<u8>, Abort> {
    match args.get(index) {
        Some(DynSolValue::Bytes(value)) => Ok(value.clone()),
        Some(DynSolValue::FixedBytes(value, size)) => Ok(value[..*size].to_vec()),
        _ => Err(mismatch(index, "bytes")),
    }
}

fn map_engine(error: Error) -> Abort {
    match error {
        Error::Stop(_) => Abort::Overrun,
        Error::Assume => Abort::Invalid,
        other => Abort::Error(other.to_string()),
    }
}
