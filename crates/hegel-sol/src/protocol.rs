//! The host-call protocol between Solidity test code and the runner.
//!
//! Solidity reaches the runner by calling well-known addresses that hold no real
//! code; the inspector intercepts those calls and answers them itself. The wire
//! format is therefore ordinary Solidity ABI calldata, and this module is the
//! single place mapping a selector to the operation and parameter types it
//! carries.
//!
//! Holding the mapping in one table, rather than a chain of signature
//! comparisons, has two consequences the rest of the crate relies on: selectors
//! are hashed once at process start instead of once per comparison per
//! intercepted call, and arguments are decoded by `alloy-dyn-abi` instead of by
//! a private reimplementation of the ABI specification.

use std::{collections::HashMap, sync::LazyLock};

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, keccak256};

/// Wire-format version of the Solidity surface in `contracts/src/Hegel.sol`.
///
/// Test projects copy or import that file, so the runner and the contract it is
/// driving are versioned independently and can drift. Bump this on any change to
/// the selectors or the semantics behind them; `HegelTest` reports the version it
/// was compiled against through `hegelProtocolVersion()`, which lets the runner
/// reject a stale copy by name instead of failing on an unknown selector.
pub const PROTOCOL_VERSION: u64 = 1;

/// Name of the `HegelTest` accessor reporting the version above. Contracts that
/// do not extend `HegelTest` simply do not declare it.
pub const PROTOCOL_VERSION_FUNCTION: &str = "hegelProtocolVersion";

/// Operations Solidity can request from the generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    DrawUint256,
    DrawInt256,
    DrawBool,
    DrawAddress,
    DrawBytes32,
    DrawBytes,
    DrawString,
    Assume,
    StartSpan,
    StopSpan,
    Note,
    Target,
    RecordEvent,
    PoolNew,
    PoolAdd,
    PoolPick,
}

/// Foundry cheatcodes the runner emulates in-process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmOp {
    Prank,
    StartPrank,
    StopPrank,
    Deal,
    Warp,
    Roll,
    ExpectRevertAny,
    ExpectRevertData,
    Label,
    Assume,
}

struct Spec<T: 'static> {
    op: T,
    signature: &'static str,
    params: &'static str,
    /// Whether the first parameter is a caller-supplied draw name. Named and
    /// auto-named overloads share an `Op`, so the flag is what tells them apart.
    named: bool,
}

/// A decoded protocol entry: which operation the selector names, and how to read
/// the calldata that follows it.
pub struct Call<T: 'static> {
    pub op: T,
    pub signature: &'static str,
    named: bool,
    params: DynSolType,
}

impl<T: Copy> Call<T> {
    /// Decode the argument section of an intercepted call into one value per
    /// declared parameter.
    pub fn decode(&self, args: &[u8]) -> Result<Vec<DynSolValue>, String> {
        let DynSolType::Tuple(types) = &self.params else {
            unreachable!("protocol parameters are always declared as a tuple");
        };
        if types.is_empty() {
            return Ok(Vec::new());
        }
        match self.params.abi_decode_params(args) {
            Ok(DynSolValue::Tuple(values)) => Ok(values),
            Ok(value) => Ok(vec![value]),
            Err(error) => Err(format!(
                "{} has malformed calldata: {error}",
                self.signature
            )),
        }
    }

    /// Split a decoded argument list into the draw name and the remaining
    /// arguments. Auto-named overloads leave the name to the caller.
    pub fn split_name<'v>(
        &self,
        values: &'v [DynSolValue],
    ) -> Result<(Option<&'v str>, &'v [DynSolValue]), String> {
        if !self.named {
            return Ok((None, values));
        }
        match values.first() {
            Some(DynSolValue::String(name)) => Ok((Some(name), &values[1..])),
            _ => Err(format!("{} is missing its draw name", self.signature)),
        }
    }
}

fn table<T: Copy>(specs: &'static [Spec<T>]) -> HashMap<[u8; 4], Call<T>> {
    specs
        .iter()
        .map(|spec| {
            let params = DynSolType::parse(spec.params).unwrap_or_else(|error| {
                panic!("invalid protocol signature {:?}: {error}", spec.signature)
            });
            debug_assert!(
                matches!(params, DynSolType::Tuple(_)),
                "protocol parameters must be declared as a tuple"
            );
            (
                selector(spec.signature),
                Call {
                    op: spec.op,
                    signature: spec.signature,
                    named: spec.named,
                    params,
                },
            )
        })
        .collect()
}

/// Keccak-256 selector of an ABI function signature.
pub fn selector(signature: &str) -> [u8; 4] {
    keccak256(signature)[..4]
        .try_into()
        .expect("keccak256 output is 32 bytes")
}

macro_rules! spec {
    ($op:expr, $signature:literal, $params:literal, $named:expr) => {
        Spec {
            op: $op,
            signature: $signature,
            params: $params,
            named: $named,
        }
    };
}

static HEGEL_SPECS: &[Spec<Op>] = &[
    spec!(
        Op::DrawUint256,
        "drawUint256(uint256,uint256)",
        "(uint256,uint256)",
        false
    ),
    spec!(
        Op::DrawUint256,
        "drawUint256(string,uint256,uint256)",
        "(string,uint256,uint256)",
        true
    ),
    spec!(
        Op::DrawInt256,
        "drawInt256(int256,int256)",
        "(int256,int256)",
        false
    ),
    spec!(
        Op::DrawInt256,
        "drawInt256(string,int256,int256)",
        "(string,int256,int256)",
        true
    ),
    spec!(Op::DrawBool, "drawBool()", "()", false),
    spec!(Op::DrawBool, "drawBool(string)", "(string)", true),
    spec!(Op::DrawAddress, "drawAddress()", "()", false),
    spec!(Op::DrawAddress, "drawAddress(string)", "(string)", true),
    spec!(Op::DrawBytes32, "drawBytes32()", "()", false),
    spec!(Op::DrawBytes32, "drawBytes32(string)", "(string)", true),
    spec!(
        Op::DrawBytes,
        "drawBytes(uint256,uint256)",
        "(uint256,uint256)",
        false
    ),
    spec!(
        Op::DrawBytes,
        "drawBytes(string,uint256,uint256)",
        "(string,uint256,uint256)",
        true
    ),
    spec!(Op::DrawString, "drawString(uint256)", "(uint256)", false),
    spec!(
        Op::DrawString,
        "drawString(string,uint256)",
        "(string,uint256)",
        true
    ),
    spec!(Op::Assume, "assume(bool)", "(bool)", false),
    spec!(Op::StartSpan, "startSpan(string)", "(string)", false),
    spec!(Op::StopSpan, "stopSpan(bool)", "(bool)", false),
    spec!(Op::Note, "note(string)", "(string)", false),
    spec!(Op::Target, "target(int256)", "(int256)", false),
    spec!(Op::Target, "target(string,int256)", "(string,int256)", true),
    spec!(Op::RecordEvent, "recordEvent(string)", "(string)", false),
    spec!(Op::PoolNew, "poolNew(string)", "(string)", false),
    spec!(
        Op::PoolAdd,
        "poolAdd(uint256,bytes32)",
        "(uint256,bytes32)",
        false
    ),
    spec!(
        Op::PoolPick,
        "poolPick(uint256,bool)",
        "(uint256,bool)",
        false
    ),
];

static VM_SPECS: &[Spec<VmOp>] = &[
    spec!(VmOp::Prank, "prank(address)", "(address)", false),
    spec!(VmOp::StartPrank, "startPrank(address)", "(address)", false),
    spec!(VmOp::StopPrank, "stopPrank()", "()", false),
    spec!(
        VmOp::Deal,
        "deal(address,uint256)",
        "(address,uint256)",
        false
    ),
    spec!(VmOp::Warp, "warp(uint256)", "(uint256)", false),
    spec!(VmOp::Roll, "roll(uint256)", "(uint256)", false),
    spec!(VmOp::ExpectRevertAny, "expectRevert()", "()", false),
    spec!(
        VmOp::ExpectRevertData,
        "expectRevert(bytes)",
        "(bytes)",
        false
    ),
    spec!(
        VmOp::ExpectRevertData,
        "expectRevert(bytes4)",
        "(bytes4)",
        false
    ),
    spec!(
        VmOp::Label,
        "label(address,string)",
        "(address,string)",
        false
    ),
    spec!(VmOp::Assume, "assume(bool)", "(bool)", false),
];

static HEGEL_TABLE: LazyLock<HashMap<[u8; 4], Call<Op>>> = LazyLock::new(|| table(HEGEL_SPECS));
static VM_TABLE: LazyLock<HashMap<[u8; 4], Call<VmOp>>> = LazyLock::new(|| table(VM_SPECS));

pub fn hegel_call(selector: &[u8]) -> Option<&'static Call<Op>> {
    HEGEL_TABLE.get(<&[u8; 4]>::try_from(selector).ok()?)
}

pub fn vm_call(selector: &[u8]) -> Option<&'static Call<VmOp>> {
    VM_TABLE.get(<&[u8; 4]>::try_from(selector).ok()?)
}

/// `console.log` overloads the runner renders. Foundry derives each selector from
/// the signature, so extending this list is the only step needed to support more.
static CONSOLE_SPECS: &[Spec<()>] = &[
    spec!((), "log(string)", "(string)", false),
    spec!((), "log(uint256)", "(uint256)", false),
    spec!((), "log(int256)", "(int256)", false),
    spec!((), "log(address)", "(address)", false),
    spec!((), "log(bool)", "(bool)", false),
    spec!((), "log(bytes)", "(bytes)", false),
    spec!((), "log(bytes32)", "(bytes32)", false),
    spec!((), "logString(string)", "(string)", false),
    spec!((), "logUint(uint256)", "(uint256)", false),
    spec!((), "logInt(int256)", "(int256)", false),
    spec!((), "logAddress(address)", "(address)", false),
    spec!((), "logBool(bool)", "(bool)", false),
    spec!((), "logBytes(bytes)", "(bytes)", false),
    spec!((), "logBytes32(bytes32)", "(bytes32)", false),
    spec!((), "log(string,string)", "(string,string)", false),
    spec!((), "log(string,uint256)", "(string,uint256)", false),
    spec!((), "log(string,int256)", "(string,int256)", false),
    spec!((), "log(string,address)", "(string,address)", false),
    spec!((), "log(string,bool)", "(string,bool)", false),
    spec!((), "log(string,bytes32)", "(string,bytes32)", false),
    spec!((), "log(uint256,uint256)", "(uint256,uint256)", false),
    spec!((), "log(address,uint256)", "(address,uint256)", false),
    spec!((), "log(address,address)", "(address,address)", false),
    spec!(
        (),
        "log(string,string,string)",
        "(string,string,string)",
        false
    ),
    spec!(
        (),
        "log(string,uint256,uint256)",
        "(string,uint256,uint256)",
        false
    ),
    spec!(
        (),
        "log(string,address,uint256)",
        "(string,address,uint256)",
        false
    ),
    spec!(
        (),
        "log(string,address,address)",
        "(string,address,address)",
        false
    ),
];

static CONSOLE_TABLE: LazyLock<HashMap<[u8; 4], Call<()>>> = LazyLock::new(|| table(CONSOLE_SPECS));

pub fn console_call(selector: &[u8]) -> Option<&'static Call<()>> {
    CONSOLE_TABLE.get(<&[u8; 4]>::try_from(selector).ok()?)
}

/// Address that Solidity calls to reach the generator.
pub fn hegel_address() -> Address {
    address_from_seed("hegel.sol")
}

/// Foundry's cheatcode address, so unmodified forge tests reach the shim.
pub fn hevm_address() -> Address {
    address_from_seed("hevm cheat code")
}

/// Foundry's `console.log` address.
pub fn console_address() -> Address {
    Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, b'c', b'o', b'n', b's', b'o', b'l', b'e', b'.', b'l', b'o', b'g',
    ])
}

fn address_from_seed(seed: &str) -> Address {
    Address::from_slice(&keccak256(seed)[12..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_signature_resolves_to_its_own_selector() {
        // A duplicate selector would silently shadow an operation in the table,
        // so assert the table is as large as the specification list.
        assert_eq!(HEGEL_TABLE.len(), HEGEL_SPECS.len());
        assert_eq!(VM_TABLE.len(), VM_SPECS.len());
        assert_eq!(CONSOLE_TABLE.len(), CONSOLE_SPECS.len());
        for spec in HEGEL_SPECS {
            let call =
                hegel_call(&selector(spec.signature)).expect("declared signature is routable");
            assert_eq!(call.signature, spec.signature);
        }
    }

    #[test]
    fn named_and_auto_named_overloads_decode_to_the_same_bounds() {
        let named = hegel_call(&selector("drawUint256(string,uint256,uint256)")).unwrap();
        let auto = hegel_call(&selector("drawUint256(uint256,uint256)")).unwrap();

        let named_args = DynSolValue::Tuple(vec![
            DynSolValue::String("amount".into()),
            DynSolValue::Uint(alloy_primitives::U256::from(3), 256),
            DynSolValue::Uint(alloy_primitives::U256::from(9), 256),
        ])
        .abi_encode_params();
        let auto_args = DynSolValue::Tuple(vec![
            DynSolValue::Uint(alloy_primitives::U256::from(3), 256),
            DynSolValue::Uint(alloy_primitives::U256::from(9), 256),
        ])
        .abi_encode_params();

        let named_values = named.decode(&named_args).unwrap();
        let auto_values = auto.decode(&auto_args).unwrap();
        let (name, named_rest) = named.split_name(&named_values).unwrap();
        let (absent, auto_rest) = auto.split_name(&auto_values).unwrap();

        assert_eq!(name, Some("amount"));
        assert_eq!(absent, None);
        assert_eq!(named_rest, auto_rest);
    }

    #[test]
    fn truncated_calldata_is_rejected_rather_than_read_past_the_end() {
        let call = hegel_call(&selector("drawUint256(uint256,uint256)")).unwrap();
        assert!(call.decode(&[0; 31]).is_err());
    }
}
