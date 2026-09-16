use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use alloy_json_abi::JsonAbi;
use anyhow::{Context as _, Result, bail};
use revm::primitives::Bytes;
use serde::Deserialize;

static BUILD_LOCK: Mutex<()> = Mutex::new(());

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

    pub fn rule_functions(&self) -> impl Iterator<Item = &alloy_json_abi::Function> {
        self.abi
            .functions()
            .filter(|function| function.name.starts_with("rule_"))
    }

    pub fn invariant_functions(&self) -> impl Iterator<Item = &alloy_json_abi::Function> {
        self.abi
            .functions()
            .filter(|function| function.name.starts_with("invariant_"))
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

#[derive(Deserialize)]
struct ArtifactCache {
    paths: CachePaths,
    files: BTreeMap<String, CachedSource>,
}

#[derive(Deserialize)]
struct CachePaths {
    artifacts: PathBuf,
}

#[derive(Deserialize)]
struct CachedSource {
    artifacts: BTreeMap<String, serde_json::Value>,
}

pub fn build(root: &Path) -> Result<Vec<Contract>> {
    // Foundry rewrites its cache during a build. Keep in-process callers from
    // reading that cache while another build is still replacing it.
    let _guard = BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let files = current_artifact_files(root)?;
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

fn current_artifact_files(root: &Path) -> Result<Vec<PathBuf>> {
    let cache_path = root.join("cache/solidity-files-cache.json");
    let text = fs::read_to_string(&cache_path)
        .with_context(|| format!("failed to read Foundry cache {}", cache_path.display()))?;
    let cache: ArtifactCache = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse Foundry cache {}", cache_path.display()))?;
    let artifact_root = root.join(&cache.paths.artifacts);
    let mut relative_paths = Vec::new();
    for source in cache.files.values() {
        for artifact in source.artifacts.values() {
            collect_artifact_paths(artifact, &mut relative_paths);
        }
    }
    relative_paths.sort();
    relative_paths.dedup();
    Ok(relative_paths
        .into_iter()
        .map(|path| artifact_root.join(path))
        .collect())
}

fn collect_artifact_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(serde_json::Value::as_str) {
                paths.push(path.to_owned());
            } else {
                for value in object.values() {
                    collect_artifact_paths(value, paths);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_artifact_paths(value, paths);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_paths_come_from_the_current_foundry_cache() {
        let root = std::env::temp_dir().join(format!("hegel-sol-artifacts-{}", std::process::id()));
        fs::create_dir_all(root.join("cache")).unwrap();
        fs::write(
            root.join("cache/solidity-files-cache.json"),
            r#"{
                "paths": {"artifacts": "generated"},
                "files": {
                    "test/Current.t.sol": {
                        "artifacts": {
                            "Current": {"0.8.30": {"default": {"path": "Current.t.sol/Current.json"}}}
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let files = current_artifact_files(&root).unwrap();
        assert_eq!(files, [root.join("generated/Current.t.sol/Current.json")]);
        fs::remove_dir_all(root).unwrap();
    }

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
