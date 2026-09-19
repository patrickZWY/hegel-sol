use anyhow::{Context as _, Result, anyhow};
use revm::{
    Context, ExecuteCommitEvm, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::{CfgEnv, TxEnv, result::ExecutionResult},
    database::{CacheDB, EmptyDB},
    primitives::{Address, Bytes, TxKind, U256},
    state::AccountInfo,
};

pub type Database = CacheDB<EmptyDB>;

pub const DEFAULT_CALLER: Address = Address::new([0x10; 20]);
pub const GAS_LIMIT: u64 = 100_000_000;

#[derive(Clone)]
pub struct Harness {
    db: Database,
    pub caller: Address,
    pub gas_limit: u64,
}

impl Harness {
    /// Build an empty chain that can run test contracts.
    ///
    /// `actors` are the additional accounts handler calls may originate from;
    /// each is funded so a call from one is never rejected for lack of balance.
    pub fn new(actors: &[Address]) -> Self {
        let mut db = Database::default();
        for caller in std::iter::once(&DEFAULT_CALLER).chain(actors) {
            db.insert_account_info(
                *caller,
                AccountInfo {
                    balance: U256::MAX,
                    ..Default::default()
                },
            );
        }
        // Solidity checks that a call target has code before some external
        // calls, so the intercepted addresses need a non-empty account. The
        // inspector short-circuits every call to them, so this STOP never runs.
        for address in [
            crate::protocol::hegel_address(),
            crate::protocol::hevm_address(),
            crate::protocol::console_address(),
        ] {
            db.insert_account_info(
                address,
                AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(&[0x00]))),
            );
        }
        Self {
            db,
            caller: DEFAULT_CALLER,
            gas_limit: GAS_LIMIT,
        }
    }

    pub fn snapshot(&self) -> Database {
        self.db.clone()
    }

    pub fn deploy(&mut self, bytecode: Bytes) -> Result<Address> {
        let context = Context::mainnet().with_db(&mut self.db).with_cfg(config());
        let mut evm = context.build_mainnet();
        let tx = TxEnv::builder()
            .caller(self.caller)
            .kind(TxKind::Create)
            .data(bytecode)
            .gas_limit(self.gas_limit)
            .build_fill();
        let result = evm
            .transact_commit(tx)
            .map_err(|error| anyhow!("deployment failed: {error:?}"))?;
        result
            .created_address()
            .context("contract deployment did not succeed")
    }

    pub fn call(&mut self, to: Address, calldata: Bytes) -> Result<ExecutionResult> {
        execute(&mut self.db, self.caller, self.gas_limit, to, calldata)
    }
}

pub fn config() -> CfgEnv {
    let mut cfg = CfgEnv::default();
    cfg.disable_nonce_check = true;
    cfg.tx_gas_limit_cap = Some(GAS_LIMIT);
    cfg
}

pub fn transaction(caller: Address, gas_limit: u64, to: Address, calldata: Bytes) -> TxEnv {
    TxEnv::builder()
        .caller(caller)
        .kind(TxKind::Call(to))
        .data(calldata)
        .gas_limit(gas_limit)
        .build_fill()
}

pub fn execute(
    db: &mut Database,
    caller: Address,
    gas_limit: u64,
    to: Address,
    calldata: Bytes,
) -> Result<ExecutionResult> {
    let context = Context::mainnet().with_db(db).with_cfg(config());
    let mut evm = context.build_mainnet();
    evm.transact_commit(transaction(caller, gas_limit, to, calldata))
        .map_err(|error| anyhow!("EVM transaction failed: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploys_and_calls_trivial_contract() {
        // Constructor returns runtime bytecode `PUSH1 0 PUSH1 0 RETURN`.
        let init = Bytes::from_static(&[
            0x64, 0x60, 0x00, 0x60, 0x00, 0xf3, 0x60, 0x00, 0x52, 0x60, 0x05, 0x60, 0x1b, 0xf3,
        ]);
        let mut harness = Harness::new(&[]);
        let address = harness.deploy(init).unwrap();
        assert!(harness.call(address, Bytes::new()).unwrap().is_success());
    }
}
