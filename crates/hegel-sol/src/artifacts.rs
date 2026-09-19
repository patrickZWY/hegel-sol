//! Loading compiled contracts out of a Solidity project.
//!
//! Everything the runner needs from a build — an ABI, deployable bytecode, and
//! optionally debug information — is standard compiler output. *Where* that
//! output lives, and in what layout, is a property of the build tool. The
//! [`Project`] trait is that boundary: [`FoundryProject`] knows Foundry's layout,
//! and nothing above this module does.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use alloy_json_abi::JsonAbi;
use anyhow::{Context as _, Result, bail};
use revm::primitives::Bytes;
use serde::Deserialize;

use crate::{
    discovery::Conventions,
    sourcemap::{
        NoSources, SolcSourceMap, SourceFile, SourceIndex, SourceLocation, SourceResolver,
    },
};

static BUILD_LOCK: Mutex<()> = Mutex::new(());

/// A compiled contract the runner can deploy and call.
#[derive(Clone)]
pub struct Contract {
    pub name: String,
    pub abi: JsonAbi,
    pub bytecode: Bytes,
    /// Artifact path, used as a stable identity for the contract.
    pub path: PathBuf,
    sources: Arc<dyn SourceResolver>,
}

impl std::fmt::Debug for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Contract")
            .field("name", &self.name)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Contract {
    pub fn test_functions<'a>(
        &'a self,
        conventions: &'a Conventions,
    ) -> impl Iterator<Item = &'a alloy_json_abi::Function> {
        self.functions_named(&conventions.test)
    }

    pub fn rule_functions<'a>(
        &'a self,
        conventions: &'a Conventions,
    ) -> impl Iterator<Item = &'a alloy_json_abi::Function> {
        self.functions_named(&conventions.rule)
    }

    pub fn invariant_functions<'a>(
        &'a self,
        conventions: &'a Conventions,
    ) -> impl Iterator<Item = &'a alloy_json_abi::Function> {
        self.functions_named(&conventions.invariant)
    }

    fn functions_named<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a alloy_json_abi::Function> {
        self.abi
            .functions()
            .filter(move |function| function.name.starts_with(prefix))
    }

    /// The single zero-argument function with this name, if the contract has one.
    pub fn nullary(&self, name: &str) -> Option<&alloy_json_abi::Function> {
        self.abi
            .function(name)
            .and_then(|functions| functions.iter().find(|function| function.inputs.is_empty()))
    }

    /// Source position for a program counter in this contract's deployed
    /// bytecode, when debug information is available.
    pub fn source_location(&self, pc: usize) -> Option<SourceLocation> {
        self.sources.resolve(pc)
    }
}

/// A Solidity project the runner can build and load contracts from.
pub trait Project {
    /// Compile the project, then load every contract it produced.
    fn compile(&self) -> Result<Loaded>;
}

/// Contracts from one build, with any non-fatal problems hit while loading them.
pub struct Loaded {
    pub contracts: Vec<Contract>,
    /// Conditions that cost debug information or narrowed discovery but did not
    /// stop the build from being usable. The CLI prints these.
    pub warnings: Vec<String>,
}

/// A project laid out and compiled by Foundry.
pub struct FoundryProject {
    root: PathBuf,
}

impl FoundryProject {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Project for FoundryProject {
    fn compile(&self) -> Result<Loaded> {
        build(&self.root)?;
        Ok(load(&self.root))
    }
}

fn build(root: &Path) -> Result<()> {
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
    Ok(())
}

/// Load contracts from an already-built project.
pub fn load(root: &Path) -> Loaded {
    let mut warnings = Vec::new();
    let files = artifact_files(root, &mut warnings);
    let mut contracts = Vec::new();
    let mut sources_by_build = BTreeMap::<String, Arc<SourceIndex>>::new();

    for file in files {
        let Ok(text) = fs::read_to_string(&file.path) else {
            continue;
        };
        // Foundry writes other JSON under the artifact tree; anything without the
        // fields below simply is not a contract.
        let Ok(artifact) = serde_json::from_str::<Artifact>(&text) else {
            continue;
        };
        let object = strip_hex(&artifact.bytecode.object);
        // An empty object is an interface or abstract contract; `__` marks an
        // unlinked library placeholder, which is not deployable as-is.
        if object.is_empty() || object.contains("__") {
            continue;
        }
        let Ok(bytecode) = alloy_primitives::hex::decode(object) else {
            warnings.push(format!(
                "ignoring {}: bytecode is not valid hex",
                file.path.display()
            ));
            continue;
        };
        let name = file
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let sources: Arc<dyn SourceResolver> = match &file.build_id {
            Some(build_id) => {
                let index = match sources_by_build.get(build_id) {
                    Some(sources) => Arc::clone(sources),
                    None => {
                        let sources = Arc::new(load_sources(root, build_id, &mut warnings));
                        sources_by_build.insert(build_id.clone(), Arc::clone(&sources));
                        sources
                    }
                };
                Arc::new(SolcSourceMap::parse(
                    &artifact.deployed_bytecode.object,
                    &artifact.deployed_bytecode.source_map,
                    index,
                ))
            }
            None => Arc::new(NoSources),
        };

        contracts.push(Contract {
            name,
            abi: artifact.abi,
            bytecode: Bytes::from(bytecode),
            path: file.path,
            sources,
        });
    }
    contracts.sort_by(|a, b| a.name.cmp(&b.name));
    Loaded {
        contracts,
        warnings,
    }
}

#[derive(Deserialize)]
struct Artifact {
    abi: JsonAbi,
    bytecode: ArtifactBytecode,
    #[serde(rename = "deployedBytecode", default)]
    deployed_bytecode: ArtifactBytecode,
}

#[derive(Deserialize, Default)]
struct ArtifactBytecode {
    #[serde(default)]
    object: String,
    #[serde(rename = "sourceMap", default)]
    source_map: String,
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

#[derive(Debug, PartialEq, Eq)]
struct ArtifactFile {
    path: PathBuf,
    build_id: Option<String>,
}

#[derive(Deserialize)]
struct BuildInfo {
    source_id_to_path: BTreeMap<String, PathBuf>,
}

/// Artifacts belonging to the current build.
///
/// Foundry's cache is the authoritative answer: it lists only what the latest
/// compilation produced, so a contract that was renamed or deleted is excluded
/// even though its old JSON is still on disk. The cache is an internal file with
/// no compatibility guarantee, so when it cannot be read the loader falls back to
/// scanning the artifact directory. That fallback can resurrect stale artifacts,
/// which is why it warns — but it keeps the runner working across a Foundry
/// release that changes the cache format.
fn artifact_files(root: &Path, warnings: &mut Vec<String>) -> Vec<ArtifactFile> {
    match cached_artifact_files(root) {
        Ok(files) if !files.is_empty() => files,
        Ok(_) => {
            warnings.push(
                "Foundry's build cache lists no artifacts; scanning the artifact directory \
                 instead, which may include contracts from an earlier build"
                    .into(),
            );
            scan_artifact_dir(&root.join("out"))
        }
        Err(error) => {
            warnings.push(format!(
                "could not read Foundry's build cache ({error:#}); scanning the artifact \
                 directory instead, which may include contracts from an earlier build"
            ));
            scan_artifact_dir(&root.join("out"))
        }
    }
}

fn cached_artifact_files(root: &Path) -> Result<Vec<ArtifactFile>> {
    let cache_path = root.join("cache/solidity-files-cache.json");
    let text = fs::read_to_string(&cache_path)
        .with_context(|| format!("failed to read {}", cache_path.display()))?;
    let cache: ArtifactCache = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", cache_path.display()))?;
    let artifact_root = root.join(&cache.paths.artifacts);
    let mut relative_paths = Vec::new();
    for source in cache.files.values() {
        for artifact in source.artifacts.values() {
            collect_artifact_paths(artifact, &mut relative_paths);
        }
    }
    relative_paths.sort_by(|a, b| a.0.cmp(&b.0));
    relative_paths.dedup_by(|a, b| a.0 == b.0);
    Ok(relative_paths
        .into_iter()
        .map(|(path, build_id)| ArtifactFile {
            path: artifact_root.join(path),
            build_id,
        })
        .collect())
}

/// Walk the artifact tree directly. Used only when the cache is unreadable, so
/// it makes no assumption beyond "compiled contracts are JSON files under `out`".
fn scan_artifact_dir(dir: &Path) -> Vec<ArtifactFile> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        // build-info holds compilation metadata, not contracts.
        if current.file_name().is_some_and(|name| name == "build-info") {
            continue;
        }
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                found.push(ArtifactFile {
                    path,
                    build_id: None,
                });
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn collect_artifact_paths(value: &serde_json::Value, paths: &mut Vec<(String, Option<String>)>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(serde_json::Value::as_str) {
                let build_id = object
                    .get("build_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                paths.push((path.to_owned(), build_id));
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

/// Sources of one compilation. Debug information is optional, so a missing or
/// unreadable file costs source positions for that file alone.
fn load_sources(root: &Path, build_id: &str, warnings: &mut Vec<String>) -> SourceIndex {
    let path = root.join("out/build-info").join(format!("{build_id}.json"));
    let info = match fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(serde_json::from_str::<BuildInfo>(&text)?))
    {
        Ok(info) => info,
        Err(error) => {
            warnings.push(format!(
                "failures in this build will be reported without a source location: \
                 could not read {} ({error})",
                path.display()
            ));
            return SourceIndex::new();
        }
    };
    let mut sources = SourceIndex::new();
    for (id, relative) in info.source_id_to_path {
        let Ok(id) = id.parse::<i64>() else {
            continue;
        };
        match fs::read_to_string(root.join(&relative)) {
            Ok(contents) => {
                sources.insert(
                    id,
                    SourceFile {
                        path: relative,
                        contents,
                    },
                );
            }
            Err(error) => warnings.push(format!(
                "could not read Solidity source {} ({error}); failures inside it will be \
                 reported without a line number",
                relative.display()
            )),
        }
    }
    sources
}

fn strip_hex(object: &str) -> &str {
    object.strip_prefix("0x").unwrap_or(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory removed even when the test that made it fails.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hegel-sol-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const CACHE: &str = r#"{
        "paths": {"artifacts": "out"},
        "files": {
            "test/Current.t.sol": {
                "artifacts": {
                    "Current": {"0.8.30": {"default": {"path": "Current.t.sol/Current.json"}}}
                }
            }
        }
    }"#;

    #[test]
    fn an_artifact_the_current_build_did_not_produce_is_ignored() {
        // Renaming a contract leaves its old JSON under `out`. Discovering it
        // would run a test that no longer exists in the source tree.
        let dir = TempDir::new("stale");
        dir.write("cache/solidity-files-cache.json", CACHE);
        dir.write("out/Current.t.sol/Current.json", "{}");
        dir.write("out/Current.t.sol/RenamedAway.json", "{}");

        let mut warnings = Vec::new();
        let files = artifact_files(&dir.0, &mut warnings);

        assert_eq!(
            files,
            [ArtifactFile {
                path: dir.0.join("out/Current.t.sol/Current.json"),
                build_id: None,
            }]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn an_unreadable_cache_falls_back_to_scanning_with_a_warning() {
        // Foundry's cache format carries no compatibility guarantee. Losing it
        // must cost precision, not the ability to run at all.
        let dir = TempDir::new("degraded");
        dir.write("cache/solidity-files-cache.json", "{ not json");
        dir.write("out/Current.t.sol/Current.json", "{}");
        dir.write("out/build-info/abc.json", "{}");

        let mut warnings = Vec::new();
        let files = artifact_files(&dir.0, &mut warnings);

        assert_eq!(
            files,
            [ArtifactFile {
                path: dir.0.join("out/Current.t.sol/Current.json"),
                build_id: None,
            }],
            "build-info is metadata, not a contract"
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("scanning the artifact directory"));
    }

    #[test]
    fn missing_build_info_costs_source_locations_but_not_the_build() {
        let dir = TempDir::new("nosources");
        let mut warnings = Vec::new();

        let sources = load_sources(&dir.0, "missing", &mut warnings);

        assert!(sources.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("without a source location"));
    }
}
