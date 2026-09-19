use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, I256, U256};
use revm::primitives::Bytes;

use crate::{
    engine::{
        Error, TestCase,
        integer::{i256_from_le_signed, i256_to_le_signed, u256_from_le_signed, u256_to_le_signed},
    },
    inspector::Draw,
    values::{self, format_value},
};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_dynamic_length: u64,
    pub max_byte_length: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dynamic_length: 8,
            max_byte_length: 64,
        }
    }
}

pub fn calldata(
    function: &Function,
    tc: &TestCase,
    limits: Limits,
) -> Result<(Bytes, Vec<Draw>), Error> {
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
        let value = match draw_value(&ty, tc, limits) {
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

fn draw_value(ty: &DynSolType, tc: &TestCase, limits: Limits) -> Result<DynSolValue, Error> {
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
            let raw = tc.integer_big(
                &u256_to_le_signed(U256::ZERO),
                &u256_to_le_signed(values::address_max()),
            )?;
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
        DynSolType::Bytes => Ok(DynSolValue::Bytes(tc.bytes(0, limits.max_byte_length)?)),
        DynSolType::String => Ok(DynSolValue::String(values::printable_string(
            tc.bytes(0, limits.max_byte_length)?,
        ))),
        DynSolType::Array(inner) => {
            let max = i64::try_from(limits.max_dynamic_length).map_err(|_| {
                Error::Engine("maximum dynamic-array length does not fit i64".into())
            })?;
            let len = tc.integer(0, max)? as usize;
            let values = (0..len)
                .map(|_| draw_value(inner, tc, limits))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::Array(values))
        }
        DynSolType::FixedArray(inner, len) => {
            let values = (0..*len)
                .map(|_| draw_value(inner, tc, limits))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::FixedArray(values))
        }
        DynSolType::Tuple(types) => {
            let values = types
                .iter()
                .map(|ty| draw_value(ty, tc, limits))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DynSolValue::Tuple(values))
        }
        DynSolType::Function => Err(Error::Engine(
            "Solidity function-pointer arguments are not supported".into(),
        )),
    }
}
