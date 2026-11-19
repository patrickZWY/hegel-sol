//! Probe fixture registration and settings on disposable EVM snapshots.
//! Scheduling, generated execution, and reporting stay in the parent runner.

use std::collections::{BTreeMap, BTreeSet};

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, U256};
use anyhow::{Context as _, Result, bail};

use crate::{
    artifacts::Contract,
    discovery::{
        EXCLUDE_CONTRACTS, EXCLUDE_SENDERS, TARGET_CONTRACTS, TARGET_SELECTORS, TARGET_SENDERS,
    },
    evm::Harness,
    protocol,
};

// Discovery and preparation must agree on which suites register external targets.
pub(super) fn has_target_registration(contract: &Contract) -> bool {
    [TARGET_CONTRACTS, TARGET_SELECTORS, TARGET_SENDERS]
        .iter()
        .any(|name| contract.nullary(name).is_some())
}

/// What a fixture registered for the state machine once `setUp()` has run.
#[derive(Default)]
pub(super) struct Discovered<'a> {
    pub(super) targets: Vec<Target<'a>>,
    pub(super) senders: Vec<Address>,
}

/// A contract the state machine calls in a Forge-style stateful test.
pub(super) struct Target<'a> {
    pub(super) contract: &'a Contract,
    pub(super) address: Address,
    pub(super) selectors: Option<BTreeSet<[u8; 4]>>,
}

/// Read a per-contract setting from a nullary view such as `stepCount()`.
///
/// The probe runs on a clone so even a non-view implementation cannot mutate
/// the snapshot shared by the actual tests. A missing function, a revert, or
/// output that is not one word all mean "use the default".
pub(super) fn probe_setting(
    harness: &Harness,
    address: Address,
    function: Option<&Function>,
) -> Option<U256> {
    let function = function?;
    let mut probe = harness.clone();
    let result = probe.call(address, selector_calldata(function)).ok()?;
    let output = result.output()?;
    (output.len() >= 32).then(|| U256::from_be_slice(&output[..32]))
}

/// Find the contracts a Forge-style test wants fuzzed and the accounts it
/// wants them called from.
///
/// Both only exist once `setUp()` has deployed and registered them, so the
/// fixture runs on a throwaway copy without an inspector; a fixture whose
/// registration depends on a cheatcode that returns a value cannot be probed
/// and registers nothing. Forge's rules apply: an explicit `targetContracts()`
/// list wins, otherwise every contract the fixture deployed is a target, and
/// `excludeContracts()` removes from either. Each target is matched back to a
/// compiled artifact by its runtime code to learn its ABI. `targetSenders()`
/// minus `excludeSenders()` replaces the synthetic actors when it is non-empty.
pub(super) fn discover_targets<'a>(
    contract: &Contract,
    contracts: &'a [Contract],
    harness: &Harness,
    address: Address,
    setup: Option<&Bytes>,
) -> Result<Discovered<'a>> {
    let base = harness.snapshot();
    let mut probe = harness.clone();
    if let Some(setup) = setup {
        // A fixture that reverts is reported when the tests run; here it just
        // leaves nothing to find.
        if !probe.call(address, setup.clone())?.is_success() {
            return Ok(Discovered::default());
        }
    }
    let registered = address_list(&mut probe, contract, address, TARGET_SENDERS)?;
    let excluded_senders = address_list(&mut probe, contract, address, EXCLUDE_SENDERS)?;
    let mut senders = Vec::new();
    for sender in &registered {
        if !excluded_senders.contains(sender) && !senders.contains(sender) {
            senders.push(*sender);
        }
    }
    if senders.is_empty() && !registered.is_empty() {
        bail!(
            "{}.{EXCLUDE_SENDERS}() removes every account {}.{TARGET_SENDERS}() registers, \
             leaving nothing to call handlers from",
            contract.name,
            contract.name
        );
    }

    let explicit = address_list(&mut probe, contract, address, TARGET_CONTRACTS)?;
    let excluded = address_list(&mut probe, contract, address, EXCLUDE_CONTRACTS)?;
    let selectors = selector_list(&mut probe, contract, address)?;

    let mut targets = Vec::new();
    if explicit.is_empty() {
        for (created, code) in probe.contracts_created_since(&base) {
            if created == address || excluded.contains(&created) {
                continue;
            }
            // A deployment no artifact describes, such as a minimal proxy, has
            // no ABI to generate calls from and is skipped rather than refused.
            if let Some(found) = contracts
                .iter()
                .find(|candidate| candidate.matches_deployed_code(&code))
            {
                targets.push(Target {
                    contract: found,
                    address: created,
                    selectors: selectors.get(&created).cloned(),
                });
            }
        }
    } else {
        for target in explicit {
            if excluded.contains(&target) || targets.iter().any(|found| found.address == target) {
                continue;
            }
            let found = registered_target(&probe, contracts, contract, target, TARGET_CONTRACTS)?;
            targets.push(Target {
                contract: found,
                address: target,
                selectors: selectors.get(&target).cloned(),
            });
        }
    }
    for (selected, allowed) in selectors {
        if excluded.contains(&selected) || targets.iter().any(|target| target.address == selected) {
            continue;
        }
        let found = registered_target(&probe, contracts, contract, selected, TARGET_SELECTORS)?;
        targets.push(Target {
            contract: found,
            address: selected,
            selectors: Some(allowed),
        });
    }
    Ok(Discovered { targets, senders })
}

// Explicit registrations must resolve to an ABI; implicit deployments may be skipped.
fn registered_target<'a>(
    probe: &Harness,
    contracts: &'a [Contract],
    fixture: &Contract,
    address: Address,
    registration: &str,
) -> Result<&'a Contract> {
    probe
        .code_at(address)
        .and_then(|code| contracts.iter().find(|candidate| candidate.matches_deployed_code(&code)))
        .with_context(|| format!(
            "{}.{registration}() names {address}, which matches no contract compiled in this project",
            fixture.name
        ))
}

/// Registration views may be absent or revert when nothing was registered.
/// Successful calls must return the expected array, even if the ABI declares otherwise.
fn registration_list(
    probe: &mut Harness,
    contract: &Contract,
    address: Address,
    name: &str,
    element: DynSolType,
    expected: &str,
) -> Result<Vec<DynSolValue>> {
    let Some(function) = contract.nullary(name) else {
        return Ok(Vec::new());
    };
    let result = probe.call(address, selector_calldata(function))?;
    if !result.is_success() {
        return Ok(Vec::new());
    }
    let kind = DynSolType::Tuple(vec![DynSolType::Array(Box::new(element))]);
    let decoded = kind
        .abi_decode_params(result.output().unwrap_or_default())
        .with_context(|| format!("{}.{name}() did not return {expected}", contract.name))?;
    let DynSolValue::Tuple(mut values) = decoded else {
        unreachable!("tuple decoder returned another shape")
    };
    let Some(DynSolValue::Array(items)) = values.pop() else {
        unreachable!("array decoder returned another shape")
    };
    Ok(items)
}

/// Decode Forge's `FuzzSelector[]`: `(address addr, bytes4[] selectors)[]`.
fn selector_list(
    probe: &mut Harness,
    contract: &Contract,
    address: Address,
) -> Result<BTreeMap<Address, BTreeSet<[u8; 4]>>> {
    let entries = registration_list(
        probe,
        contract,
        address,
        TARGET_SELECTORS,
        DynSolType::Tuple(vec![
            DynSolType::Address,
            DynSolType::Array(Box::new(DynSolType::FixedBytes(4))),
        ]),
        "FuzzSelector[]",
    )?;
    let mut selected = BTreeMap::<Address, BTreeSet<[u8; 4]>>::new();
    for entry in entries {
        let DynSolValue::Tuple(fields) = entry else {
            unreachable!("FuzzSelector decoder returned another shape")
        };
        let [DynSolValue::Address(target), DynSolValue::Array(functions)] = fields.as_slice()
        else {
            unreachable!("FuzzSelector decoder returned another shape")
        };
        let allowed = selected.entry(*target).or_default();
        for function in functions {
            let DynSolValue::FixedBytes(bytes, 4) = function else {
                unreachable!("FuzzSelector decoder returned another shape")
            };
            allowed.insert(bytes[..4].try_into().expect("four selector bytes"));
        }
    }
    Ok(selected)
}

/// Call a view returning `address[]`, treating a missing or reverting one as
/// empty; Solady's base reverts when nothing was registered.
fn address_list(
    probe: &mut Harness,
    contract: &Contract,
    address: Address,
    name: &str,
) -> Result<Vec<Address>> {
    let items = registration_list(
        probe,
        contract,
        address,
        name,
        DynSolType::Address,
        "address[]",
    )?;
    Ok(items
        .into_iter()
        .filter_map(|item| match item {
            DynSolValue::Address(address) => Some(address),
            _ => None,
        })
        .collect())
}

/// Reject a `Hegel.sol` built against a different release of the protocol.
///
/// Test projects hold their own copy of the Solidity surface, so it can lag the
/// runner. Detecting that here turns a confusing unknown-selector failure, raised
/// mid-test, into one actionable message before any test runs. Contracts that do
/// not extend `HegelTest` have no version to report and are left alone.
pub(super) fn check_protocol_version(
    contract: &Contract,
    harness: &Harness,
    address: Address,
) -> Result<()> {
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

pub(super) fn selector_calldata(function: &Function) -> Bytes {
    Bytes::copy_from_slice(function.selector().as_slice())
}
