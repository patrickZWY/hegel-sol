use std::collections::HashMap;

use alloy_primitives::{Address, B256, I256, U256, keccak256};
use revm::{
    Context as RevmContext, Inspector,
    context::{
        BlockEnv, CfgEnv, Journal, JournalTr, TxEnv, journaled_state::account::JournaledAccountTr,
    },
    database_interface::Database,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult,
        interpreter::EthInterpreter,
    },
    primitives::Bytes,
};
use serde::Serialize;

use crate::engine::{
    Error, Pool, TestCase,
    integer::{i256_from_le_signed, i256_to_le_signed, u256_from_le_signed, u256_to_le_signed},
};

#[derive(Debug, Clone, Serialize)]
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
}

struct PoolValues {
    engine: Pool,
    values: HashMap<i64, B256>,
}

impl<'a> HegelInspector<'a> {
    pub fn new(tc: &'a TestCase) -> Self {
        Self {
            tc,
            trace: Vec::new(),
            console: Vec::new(),
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
        }
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

    fn dispatch(&mut self, data: &[u8]) -> Result<Bytes, Abort> {
        if data.len() < 4 {
            return Err(Abort::Error("Hegel call has no selector".into()));
        }
        let selector = &data[..4];
        let args = &data[4..];

        if selector == function_selector("drawUint256(uint256,uint256)") {
            let name = self.auto_name();
            let lo = word_u256(args, 0)?;
            let hi = word_u256(args, 1)?;
            let raw = self
                .tc
                .integer_big(&u256_to_le_signed(lo), &u256_to_le_signed(hi))
                .map_err(map_engine)?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid uint256 from Hegel".into()))?;
            self.record(name, "uint256", value.to_string());
            return Ok(Bytes::copy_from_slice(&value.to_be_bytes::<32>()));
        }
        if selector == function_selector("drawInt256(int256,int256)") {
            let name = self.auto_name();
            let lo = I256::from_raw(word_u256(args, 0)?);
            let hi = I256::from_raw(word_u256(args, 1)?);
            let raw = self
                .tc
                .integer_big(&i256_to_le_signed(lo), &i256_to_le_signed(hi))
                .map_err(map_engine)?;
            let value = i256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid int256 from Hegel".into()))?;
            self.record(name, "int256", value.to_string());
            return Ok(Bytes::copy_from_slice(
                &value.into_raw().to_be_bytes::<32>(),
            ));
        }
        if selector == function_selector("drawBool()") {
            let name = self.auto_name();
            let value = self.tc.boolean(0.5).map_err(map_engine)?;
            self.record(name, "bool", value.to_string());
            return Ok(abi_bool(value));
        }
        if selector == function_selector("drawAddress()") {
            let name = self.auto_name();
            let max = (U256::from(1) << 160) - U256::from(1);
            let raw = self
                .tc
                .integer_big(&u256_to_le_signed(U256::ZERO), &u256_to_le_signed(max))
                .map_err(map_engine)?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid address from Hegel".into()))?;
            let word = value.to_be_bytes::<32>();
            let address = Address::from_slice(&word[12..]);
            self.record(name, "address", address.to_string());
            return Ok(Bytes::copy_from_slice(&word));
        }
        if selector == function_selector("drawBytes32()") {
            let name = self.auto_name();
            let value = self.tc.bytes(32, 32).map_err(map_engine)?;
            let b256 = B256::from_slice(&value);
            self.record(name, "bytes32", b256.to_string());
            return Ok(Bytes::from(value));
        }
        if selector == function_selector("drawBytes(uint256,uint256)") {
            let name = self.auto_name();
            let min = u64_bound(word_u256(args, 0)?)?;
            let max = u64_bound(word_u256(args, 1)?)?;
            let value = self.tc.bytes(min, max).map_err(map_engine)?;
            self.record(
                name,
                "bytes",
                format!("0x{}", alloy_primitives::hex::encode(&value)),
            );
            return Ok(abi_dynamic(&value));
        }
        if selector == function_selector("drawString(uint256)") {
            let name = self.auto_name();
            let max = u64_bound(word_u256(args, 0)?)?;
            let value = self.tc.bytes(0, max).map_err(map_engine)?;
            let printable = String::from_utf8_lossy(&value).into_owned();
            self.record(name, "string", format!("{printable:?}"));
            return Ok(abi_dynamic(&value));
        }

        if selector == function_selector("drawUint256(string,uint256,uint256)") {
            let name = dynamic_string(args, 0)?;
            let lo = word_u256(args, 1)?;
            let hi = word_u256(args, 2)?;
            let raw = self
                .tc
                .integer_big(&u256_to_le_signed(lo), &u256_to_le_signed(hi))
                .map_err(map_engine)?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid uint256 from Hegel".into()))?;
            self.record(name, "uint256", value.to_string());
            return Ok(Bytes::copy_from_slice(&value.to_be_bytes::<32>()));
        }
        if selector == function_selector("drawInt256(string,int256,int256)") {
            let name = dynamic_string(args, 0)?;
            let lo = I256::from_raw(word_u256(args, 1)?);
            let hi = I256::from_raw(word_u256(args, 2)?);
            let raw = self
                .tc
                .integer_big(&i256_to_le_signed(lo), &i256_to_le_signed(hi))
                .map_err(map_engine)?;
            let value = i256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid int256 from Hegel".into()))?;
            self.record(name, "int256", value.to_string());
            return Ok(Bytes::copy_from_slice(
                &value.into_raw().to_be_bytes::<32>(),
            ));
        }
        if selector == function_selector("drawBool(string)") {
            let name = dynamic_string(args, 0)?;
            let value = self.tc.boolean(0.5).map_err(map_engine)?;
            self.record(name, "bool", value.to_string());
            return Ok(abi_bool(value));
        }
        if selector == function_selector("drawAddress(string)") {
            let name = dynamic_string(args, 0)?;
            let max = (U256::from(1) << 160) - U256::from(1);
            let raw = self
                .tc
                .integer_big(&u256_to_le_signed(U256::ZERO), &u256_to_le_signed(max))
                .map_err(map_engine)?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Abort::Error("invalid address from Hegel".into()))?;
            let word = value.to_be_bytes::<32>();
            let address = Address::from_slice(&word[12..]);
            self.record(name, "address", address.to_string());
            return Ok(Bytes::copy_from_slice(&word));
        }
        if selector == function_selector("drawBytes32(string)") {
            let name = dynamic_string(args, 0)?;
            let value = self.tc.bytes(32, 32).map_err(map_engine)?;
            let b256 = B256::from_slice(&value);
            self.record(name, "bytes32", b256.to_string());
            return Ok(Bytes::from(value));
        }
        if selector == function_selector("drawBytes(string,uint256,uint256)") {
            let name = dynamic_string(args, 0)?;
            let min = u64_bound(word_u256(args, 1)?)?;
            let max = u64_bound(word_u256(args, 2)?)?;
            let value = self.tc.bytes(min, max).map_err(map_engine)?;
            self.record(
                name,
                "bytes",
                format!("0x{}", alloy_primitives::hex::encode(&value)),
            );
            return Ok(abi_dynamic(&value));
        }
        if selector == function_selector("drawString(string,uint256)") {
            let name = dynamic_string(args, 0)?;
            let max = u64_bound(word_u256(args, 1)?)?;
            let value = self.tc.bytes(0, max).map_err(map_engine)?;
            let printable = String::from_utf8_lossy(&value).into_owned();
            self.record(name, "string", format!("{printable:?}"));
            return Ok(abi_dynamic(&value));
        }
        if selector == function_selector("assume(bool)") {
            if word_u256(args, 0)? == U256::ZERO {
                return Err(Abort::Invalid);
            }
            return Ok(Bytes::new());
        }
        if selector == function_selector("startSpan(string)") {
            self.tc
                .start_span(&dynamic_string(args, 0)?)
                .map_err(map_engine)?;
            return Ok(Bytes::new());
        }
        if selector == function_selector("stopSpan(bool)") {
            self.tc
                .stop_span(word_u256(args, 0)? != U256::ZERO)
                .map_err(map_engine)?;
            return Ok(Bytes::new());
        }
        if selector == function_selector("note(string)") {
            self.tc
                .note(&dynamic_string(args, 0)?)
                .map_err(map_engine)?;
            return Ok(Bytes::new());
        }
        if selector == function_selector("target(int256)") {
            let score = I256::from_raw(word_u256(args, 0)?)
                .to_string()
                .parse::<f64>()
                .map_err(|error| Abort::Error(format!("invalid target score: {error}")))?;
            self.tc.target(score).map_err(map_engine)?;
            return Ok(Bytes::new());
        }
        if selector == function_selector("recordEvent(string)") {
            let label = dynamic_string(args, 0)?;
            self.tc.event(&label).map_err(map_engine)?;
            self.record(label, "event", String::new());
            return Ok(Bytes::new());
        }
        if selector == function_selector("poolNew(string)") {
            let name = dynamic_string(args, 0)?;
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
            return Ok(Bytes::copy_from_slice(&U256::from(id).to_be_bytes::<32>()));
        }
        if selector == function_selector("poolAdd(uint256,bytes32)") {
            let id = u64_bound(word_u256(args, 0)?)?;
            let value = B256::from_slice(word(args, 1)?);
            let pool = self
                .pools
                .get_mut(&id)
                .ok_or_else(|| Abort::Error(format!("unknown Hegel pool {id}")))?;
            let variable = pool.engine.add(self.tc).map_err(map_engine)?;
            pool.values.insert(variable, value);
            return Ok(Bytes::new());
        }
        if selector == function_selector("poolPick(uint256,bool)") {
            let id = u64_bound(word_u256(args, 0)?)?;
            let consume = word_u256(args, 1)? != U256::ZERO;
            let pool = self
                .pools
                .get_mut(&id)
                .ok_or_else(|| Abort::Error(format!("unknown Hegel pool {id}")))?;
            let variable = pool.engine.generate(self.tc, consume).map_err(map_engine)?;
            let value = pool.values.get(&variable).copied().ok_or_else(|| {
                Abort::Error(format!("pool {id} returned unknown variable {variable}"))
            })?;
            if consume {
                pool.values.remove(&variable);
            }
            self.record(format!("pool_{id}"), "bytes32", value.to_string());
            return Ok(Bytes::copy_from_slice(value.as_slice()));
        }
        Err(Abort::Error(format!(
            "unknown Hegel selector 0x{}",
            alloy_primitives::hex::encode(selector)
        )))
    }

    fn dispatch_vm<DB: Database>(
        &mut self,
        ctx: &mut RevmContext<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>>,
        data: &[u8],
    ) -> Result<Bytes, Abort> {
        if data.len() < 4 {
            return Err(Abort::Error("cheatcode call has no selector".into()));
        }
        let selector = &data[..4];
        let args = &data[4..];
        if selector == function_selector("prank(address)") {
            self.next_prank = Some(word_address(args, 0)?);
            return Ok(Bytes::new());
        }
        if selector == function_selector("startPrank(address)") {
            self.persistent_prank = Some(word_address(args, 0)?);
            return Ok(Bytes::new());
        }
        if selector == function_selector("stopPrank()") {
            self.persistent_prank = None;
            self.next_prank = None;
            return Ok(Bytes::new());
        }
        if selector == function_selector("deal(address,uint256)") {
            let address = word_address(args, 0)?;
            let balance = word_u256(args, 1)?;
            let mut account = ctx
                .journaled_state
                .load_account_mut(address)
                .map_err(|_| Abort::Error("failed to load account for vm.deal".into()))?;
            account.data.set_balance(balance);
            return Ok(Bytes::new());
        }
        if selector == function_selector("warp(uint256)") {
            let value = word_u256(args, 0)?;
            ctx.block.timestamp = value;
            self.timestamp = Some(value);
            return Ok(Bytes::new());
        }
        if selector == function_selector("roll(uint256)") {
            let value = word_u256(args, 0)?;
            ctx.block.number = value;
            self.block_number = Some(value);
            return Ok(Bytes::new());
        }
        if selector == function_selector("expectRevert()") {
            self.expected_revert = Some(None);
            return Ok(Bytes::new());
        }
        if selector == function_selector("expectRevert(bytes)") {
            self.expected_revert = Some(Some(dynamic_data(args, 0)?.to_vec()));
            return Ok(Bytes::new());
        }
        if selector == function_selector("expectRevert(bytes4)") {
            self.expected_revert = Some(Some(word(args, 0)?[..4].to_vec()));
            return Ok(Bytes::new());
        }
        if selector == function_selector("label(address,string)") {
            self.labels
                .insert(word_address(args, 0)?, dynamic_string(args, 1)?);
            return Ok(Bytes::new());
        }
        if selector == function_selector("assume(bool)") {
            if word_u256(args, 0)? == U256::ZERO {
                return Err(Abort::Invalid);
            }
            return Ok(Bytes::new());
        }
        Err(Abort::Error(format!(
            "unsupported Foundry cheatcode selector 0x{}",
            alloy_primitives::hex::encode(selector)
        )))
    }

    fn capture_console(&mut self, data: &[u8]) {
        self.console.push(decode_console(data));
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
        if inputs.target_address == console_address() {
            let input = inputs.input.bytes(ctx);
            self.capture_console(&input);
            return Some(return_outcome(inputs, Bytes::new()));
        }
        if inputs.target_address == hevm_address() {
            let input = inputs.input.bytes(ctx);
            let (instruction, output) = match self.dispatch_vm(ctx, &input) {
                Ok(output) => (InstructionResult::Return, output),
                Err(abort) => {
                    self.outcome = Some(abort);
                    (InstructionResult::Revert, Bytes::new())
                }
            };
            return Some(outcome(inputs, instruction, output));
        }
        if inputs.target_address == hegel_address() {
            let input = inputs.input.bytes(ctx);
            let (instruction, output) = match self.dispatch(&input) {
                Ok(output) => (InstructionResult::Return, output),
                Err(abort) => {
                    self.outcome = Some(abort);
                    (InstructionResult::Revert, Bytes::new())
                }
            };
            return Some(outcome(inputs, instruction, output));
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

fn outcome(inputs: &CallInputs, instruction: InstructionResult, output: Bytes) -> CallOutcome {
    CallOutcome::new(
        InterpreterResult::new(instruction, output, Gas::new(inputs.gas_limit)),
        inputs.return_memory_offset.clone(),
    )
}

fn return_outcome(inputs: &CallInputs, output: Bytes) -> CallOutcome {
    outcome(inputs, InstructionResult::Return, output)
}

pub fn hegel_address() -> Address {
    let hash = keccak256("hegel.sol");
    Address::from_slice(&hash[12..])
}

fn function_selector(signature: &str) -> [u8; 4] {
    keccak256(signature)[..4].try_into().unwrap()
}

fn word(args: &[u8], index: usize) -> Result<&[u8], Abort> {
    let start = index
        .checked_mul(32)
        .ok_or_else(|| Abort::Error("ABI offset overflow".into()))?;
    args.get(start..start + 32)
        .ok_or_else(|| Abort::Error("truncated Hegel calldata".into()))
}

fn word_u256(args: &[u8], index: usize) -> Result<U256, Abort> {
    Ok(U256::from_be_slice(word(args, index)?))
}

fn word_address(args: &[u8], index: usize) -> Result<Address, Abort> {
    Ok(Address::from_slice(&word(args, index)?[12..]))
}

fn dynamic_data(args: &[u8], index: usize) -> Result<&[u8], Abort> {
    let offset = u64_bound(word_u256(args, index)?)? as usize;
    let len_word = args
        .get(offset..offset + 32)
        .ok_or_else(|| Abort::Error("invalid ABI bytes offset".into()))?;
    let len = u64_bound(U256::from_be_slice(len_word))? as usize;
    args.get(offset + 32..offset + 32 + len)
        .ok_or_else(|| Abort::Error("truncated ABI bytes".into()))
}

fn dynamic_string(args: &[u8], index: usize) -> Result<String, Abort> {
    let offset = u64_bound(word_u256(args, index)?)? as usize;
    let len_word = args
        .get(offset..offset + 32)
        .ok_or_else(|| Abort::Error("invalid ABI string offset".into()))?;
    let len = u64_bound(U256::from_be_slice(len_word))? as usize;
    let value = args
        .get(offset + 32..offset + 32 + len)
        .ok_or_else(|| Abort::Error("truncated ABI string".into()))?;
    String::from_utf8(value.to_vec())
        .map_err(|error| Abort::Error(format!("invalid UTF-8 draw name: {error}")))
}

fn u64_bound(value: U256) -> Result<u64, Abort> {
    u64::try_from(value).map_err(|_| Abort::Error("value does not fit u64".into()))
}

fn abi_bool(value: bool) -> Bytes {
    let mut word = [0; 32];
    word[31] = u8::from(value);
    Bytes::copy_from_slice(&word)
}

fn abi_dynamic(value: &[u8]) -> Bytes {
    let padded = value.len().div_ceil(32) * 32;
    let mut output = vec![0; 64 + padded];
    output[31] = 32;
    U256::from(value.len())
        .to_be_bytes::<32>()
        .iter()
        .enumerate()
        .for_each(|(i, byte)| output[32 + i] = *byte);
    output[64..64 + value.len()].copy_from_slice(value);
    Bytes::from(output)
}

fn decode_console(data: &[u8]) -> String {
    if data.len() < 4 {
        return "console.log: <invalid calldata>".into();
    }
    let selector = &data[..4];
    let args = &data[4..];
    let value = if selector == function_selector("log(string)") {
        dynamic_string(args, 0).unwrap_or_else(|_| "<invalid string>".into())
    } else if selector == function_selector("log(uint256)") {
        word_u256(args, 0)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "<invalid uint256>".into())
    } else if selector == function_selector("log(int256)") {
        word_u256(args, 0)
            .map(|value| I256::from_raw(value).to_string())
            .unwrap_or_else(|_| "<invalid int256>".into())
    } else if selector == function_selector("log(address)") {
        word_address(args, 0)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "<invalid address>".into())
    } else if selector == function_selector("log(bool)") {
        word_u256(args, 0)
            .map(|value| (value != U256::ZERO).to_string())
            .unwrap_or_else(|_| "<invalid bool>".into())
    } else {
        format!("selector 0x{}", alloy_primitives::hex::encode(selector))
    };
    format!("console.log: {value}")
}

fn map_engine(error: Error) -> Abort {
    match error {
        Error::Stop(_) => Abort::Overrun,
        Error::Assume => Abort::Invalid,
        other => Abort::Error(other.to_string()),
    }
}

pub fn hevm_address() -> Address {
    let hash = keccak256("hevm cheat code");
    Address::from_slice(&hash[12..])
}

pub fn console_address() -> Address {
    Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, b'c', b'o', b'n', b's', b'o', b'l', b'e', b'.', b'l', b'o', b'g',
    ])
}
