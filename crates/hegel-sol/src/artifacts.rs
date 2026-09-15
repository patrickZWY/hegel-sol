use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use alloy_json_abi::JsonAbi;
use anyhow::{Context as _, Result, bail};
use revm::primitives::Bytes;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Contract {
    pub name: String,
    pub abi: JsonAbi,
    pub bytecode: Bytes,
    pub path: PathBuf,
}

impl Contract {
    pub fn test_functions(&self) -> impl Iterator<Item = &alloy_json_abi::Function> {
        self.abi
            .functions()
            .filter(|function| function.name.starts_with("test"))
    }
}

#[derive(Deserialize)]
struct Artifact {
    abi: JsonAbi,
    bytecode: ArtifactBytecode,
}

#[derive(Deserialize)]
struct ArtifactBytecode {
    object: String,
}

pub fn build(root: &Path) -> Result<Vec<Contract>> {
    let output = Command::new("forge")
        .args(["build", "--root"])
        .arg(root)
        .output()
        .with_context(|| format!("failed to run forge in {}", root.display()))?;
    if !output.status.success() {
        bail!(
            "forge build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    load(root)
}

pub fn load(root: &Path) -> Result<Vec<Contract>> {
    let out = root.join("out");
    let mut files = Vec::new();
    visit_json(&out, &mut files)
        .with_context(|| format!("failed to read Foundry artifacts under {}", out.display()))?;
    let mut contracts = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)?;
        let artifact: Artifact = match serde_json::from_str(&text) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let object = artifact
            .bytecode
            .object
            .strip_prefix("0x")
            .unwrap_or(&artifact.bytecode.object);
        if object.is_empty() || object.contains("__") {
            continue;
        }
        let bytecode = alloy_primitives::hex::decode(object)
            .with_context(|| format!("invalid bytecode in {}", path.display()))?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        contracts.push(Contract {
            name,
            abi: artifact.abi,
            bytecode: Bytes::from(bytecode),
            path,
        });
    }
    contracts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(contracts)
}

fn visit_json(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            visit_json(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_spike_contract_and_test_selector() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
        let contracts = build(&root).unwrap();
        let spike = contracts
            .iter()
            .find(|contract| contract.name == "ERC20SpikeTest")
            .unwrap();
        assert!(
            spike
                .test_functions()
                .any(|function| function.name == "test_transfer_preserves_supply")
        );
    }
}
