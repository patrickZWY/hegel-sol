use alloy_primitives::{Address, B256, I256, U256, keccak256};
use revm::{
    Inspector,
    context_interface::ContextTr,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult,
        interpreter::EthInterpreter,
    },
    primitives::Bytes,
};
use serde::Serialize;

use crate::engine::{
    Error, TestCase,
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
    pub outcome: Option<Abort>,
}

impl<'a> HegelInspector<'a> {
    pub fn new(tc: &'a TestCase) -> Self {
        Self {
            tc,
            trace: Vec::new(),
            outcome: None,
        }
    }

    fn dispatch(&mut self, data: &[u8]) -> Result<Bytes, Abort> {
        if data.len() < 4 {
            return Err(Abort::Error("Hegel call has no selector".into()));
        }
        let selector = &data[..4];
        let args = &data[4..];

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
            self.tc
                .event(&dynamic_string(args, 0)?)
                .map_err(map_engine)?;
            return Ok(Bytes::new());
        }
        Err(Abort::Error(format!(
            "unknown Hegel selector 0x{}",
            alloy_primitives::hex::encode(selector)
        )))
    }

    fn record(&mut self, name: String, kind: &str, value: String) {
        self.trace.push(Draw {
            name,
            kind: kind.into(),
            value,
        });
    }
}

impl<CTX: ContextTr> Inspector<CTX, EthInterpreter> for HegelInspector<'_> {
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address != hegel_address() {
            return None;
        }
        let input = inputs.input.bytes(ctx);
        let (instruction, output) = match self.dispatch(&input) {
            Ok(output) => (InstructionResult::Return, output),
            Err(abort) => {
                self.outcome = Some(abort);
                (InstructionResult::Revert, Bytes::new())
            }
        };
        Some(CallOutcome::new(
            InterpreterResult::new(instruction, output, Gas::new(inputs.gas_limit)),
            inputs.return_memory_offset.clone(),
        ))
    }
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

fn map_engine(error: Error) -> Abort {
    match error {
        Error::Stop(_) => Abort::Overrun,
        Error::Assume => Abort::Invalid,
        other => Abort::Error(other.to_string()),
    }
}
