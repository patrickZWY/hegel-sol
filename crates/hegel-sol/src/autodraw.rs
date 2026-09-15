use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, I256, U256, hex};
use revm::primitives::Bytes;

use crate::{
    engine::{
        Error, TestCase,
        integer::{i256_from_le_signed, i256_to_le_signed, u256_from_le_signed, u256_to_le_signed},
    },
    inspector::Draw,
};

const MAX_DYNAMIC_LENGTH: u64 = 8;
const MAX_BYTE_LENGTH: u64 = 64;

pub fn calldata(function: &Function, tc: &TestCase) -> Result<(Bytes, Vec<Draw>), Error> {
    let mut values = Vec::with_capacity(function.inputs.len());
    let mut trace = Vec::with_capacity(function.inputs.len());
    for (index, parameter) in function.inputs.iter().enumerate() {
        let name = if parameter.name.is_empty() {
            format!("arg_{index}")
        } else {
            parameter.name.clone()
        };
        let ty = parameter
            .resolve()
            .map_err(|error| Error::Engine(error.to_string()))?;
        tc.start_span(&format!("hegel.sol.argument.{name}"))?;
        let value = match draw_value(&ty, tc) {
            Ok(value) => value,
            Err(error) => {
                let _ = tc.stop_span(true);
                return Err(error);
            }
        };
        tc.stop_span(false)?;
        trace.push(Draw {
            name,
            kind: ty.to_string(),
            value: format_value(&value),
        });
        values.push(value);
    }
    let encoded = function
        .abi_encode_input(&values)
        .map_err(|error| Error::Engine(error.to_string()))?;
    Ok((Bytes::from(encoded), trace))
}

fn draw_value(ty: &DynSolType, tc: &TestCase) -> Result<DynSolValue, Error> {
    match ty {
        DynSolType::Bool => Ok(DynSolValue::Bool(tc.boolean(0.5)?)),
        DynSolType::Uint(bits) => {
            let max = if *bits == 256 {
                U256::MAX
            } else {
                (U256::from(1) << bits) - U256::from(1)
            };
            let raw = tc.integer_big(&u256_to_le_signed(U256::ZERO), &u256_to_le_signed(max))?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Error::Engine("invalid unsigned integer from Hegel".into()))?;
            Ok(DynSolValue::Uint(value, *bits))
        }
        DynSolType::Int(bits) => {
            let min_raw = U256::MAX << (bits - 1);
            let max_raw = (U256::from(1) << (bits - 1)) - U256::from(1);
            let min = I256::from_raw(min_raw);
            let max = I256::from_raw(max_raw);
            let raw = tc.integer_big(&i256_to_le_signed(min), &i256_to_le_signed(max))?;
            let value = i256_from_le_signed(&raw)
                .ok_or_else(|| Error::Engine("invalid signed integer from Hegel".into()))?;
            Ok(DynSolValue::Int(value, *bits))
        }
        DynSolType::Address => {
            let max = (U256::from(1) << 160) - U256::from(1);
            let raw = tc.integer_big(&u256_to_le_signed(U256::ZERO), &u256_to_le_signed(max))?;
            let value = u256_from_le_signed(&raw)
                .ok_or_else(|| Error::Engine("invalid address from Hegel".into()))?;
            let word = value.to_be_bytes::<32>();
            Ok(DynSolValue::Address(Address::from_slice(&word[12..])))
        }
        DynSolType::FixedBytes(size) => {
            let bytes = tc.bytes(*size as u64, *size as u64)?;
            let mut word = B256::ZERO;
            word[..*size].copy_from_slice(&bytes);
            Ok(DynSolValue::FixedBytes(word, *size))
        }
        DynSolType::Bytes => Ok(DynSolValue::Bytes(tc.bytes(0, MAX_BYTE_LENGTH)?)),
        DynSolType::String => {
            let bytes = tc.bytes(0, MAX_BYTE_LENGTH)?;
            let text: String = bytes
                .into_iter()
                .map(|byte| char::from(32 + byte % 95))
                .collect();
            Ok(DynSolValue::String(text))
        }
        DynSolType::Array(inner) => {
            let len = tc.integer(0, MAX_DYNAMIC_LENGTH as i64)? as usize;
            let values = (0..len)
                .map(|_| draw_value(inner, tc))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::Array(values))
        }
        DynSolType::FixedArray(inner, len) => {
            let values = (0..*len)
                .map(|_| draw_value(inner, tc))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::FixedArray(values))
        }
        DynSolType::Tuple(types) => {
            let values = types
                .iter()
                .map(|ty| draw_value(ty, tc))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::Tuple(values))
        }
        DynSolType::Function => Err(Error::Engine(
            "Solidity function-pointer arguments are not supported".into(),
        )),
    }
}

fn format_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Bool(value) => value.to_string(),
        DynSolValue::Int(value, _) => value.to_string(),
        DynSolValue::Uint(value, _) => value.to_string(),
        DynSolValue::FixedBytes(value, size) => format!("0x{}", hex::encode(&value[..*size])),
        DynSolValue::Address(value) => value.to_string(),
        DynSolValue::Function(value) => format!("0x{}", hex::encode(value)),
        DynSolValue::Bytes(value) => format!("hex\"{}\"", hex::encode(value)),
        DynSolValue::String(value) => format!("{value:?}"),
        DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
            format!(
                "[{}]",
                values
                    .iter()
                    .map(format_value)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        DynSolValue::Tuple(values) => {
            format!(
                "({})",
                values
                    .iter()
                    .map(format_value)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}
