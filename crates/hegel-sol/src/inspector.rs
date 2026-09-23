use std::collections::{HashMap, HashSet};

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, B256, I256, U256};
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
    protocol::{self, Op, VmOp},
    values::{self, format_value},
};

/// JUMPDEST. Recording coverage only at basic-block entries keeps the per-opcode
/// `step` hook cheap while still distinguishing the paths a test took.
const JUMPDEST: u8 = 0x5b;
const REVERT: u8 = 0xfd;
const INVALID: u8 = 0xfe;

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
    labels: HashMap<Address, String>,
    timestamp: Option<U256>,
    block_number: Option<U256>,
    trace_target: Address,
    last_pc: Option<usize>,
    failure_pc: Option<usize>,
    coverage: Option<HashSet<u64>>,
}

struct PoolValues {
    engine: Pool,
    values: HashMap<i64, B256>,
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
            labels: HashMap::new(),
            timestamp: None,
            block_number: None,
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

    /// Number of distinct basic blocks entered so far, or `None` when coverage
    /// collection is disabled.
    pub fn coverage_score(&self) -> Option<usize> {
        self.coverage.as_ref().map(HashSet::len)
    }

    pub fn reset_source_trace(&mut self) {
        self.last_pc = None;
        self.failure_pc = None;
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
                let score = arg_i256(args, 0)?;
                self.tc
                    .target(label, values::i256_as_f64(score))
                    .map_err(map_engine)?;
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
    ) -> Result<Bytes, Abort> {
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
            VmOp::ExpectRevertAny => self.expected_revert = Some(None),
            VmOp::ExpectRevertData => self.expected_revert = Some(Some(arg_data(args, 0)?)),
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
        Ok(Bytes::new())
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
                Ok(output) => (InstructionResult::Return, output),
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
        if let Some(sender) = self.next_prank.take().or(self.persistent_prank) {
            inputs.caller = sender;
        }
        if let Some(expected) = self.expected_revert.take() {
            self.active_expect = Some((inputs.target_address, expected));
        }
        None
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
            result.result.output = Bytes::new();
        } else {
            result.result.result = InstructionResult::Revert;
            result.result.output = Bytes::from_static(b"hegel-sol: expected revert did not match");
        }
    }
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
