//! Mapping a failing program counter back to Solidity source.
//!
//! This is the most compiler-specific code in the crate: it decodes solc's
//! source-map encoding and the build-info file that names the sources it refers
//! to. Both are outputs of a toolchain the runner does not control, so the whole
//! area sits behind [`SourceResolver`] and answers with `Option`. A format change
//! should cost a line number, never a test run.

use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

/// A position in a Solidity source file. Paths are relative to the project root
/// and both coordinates are one-based, matching how editors and compilers report
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

/// Resolves a program counter in a contract's deployed bytecode to a source
/// position, when one is known.
pub trait SourceResolver: Send + Sync {
    fn resolve(&self, pc: usize) -> Option<SourceLocation>;
}

/// Used when a project carries no usable debug information. Reporting stays
/// correct; failures simply arrive without a file and line.
pub struct NoSources;

impl SourceResolver for NoSources {
    fn resolve(&self, _pc: usize) -> Option<SourceLocation> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub contents: String,
}

/// Source files of one compilation, keyed by the source ids solc emits.
pub type SourceIndex = BTreeMap<i64, SourceFile>;

/// solc's source map for one contract, paired with that compilation's sources.
pub struct SolcSourceMap {
    by_pc: BTreeMap<usize, SourceElement>,
    sources: std::sync::Arc<SourceIndex>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SourceElement {
    offset: usize,
    length: usize,
    file: i64,
}

impl SolcSourceMap {
    /// Decode solc's `s:l:f:j:m` entries against the bytecode they annotate.
    ///
    /// Entries are positional: one per instruction, with empty fields inheriting
    /// the previous entry's value, and trailing fields omitted entirely. Only the
    /// first three are needed here; jump type and modifier depth are ignored. A
    /// malformed entry ends decoding rather than failing, so a newer encoding
    /// degrades to partial coverage.
    pub fn parse(bytecode: &str, encoded: &str, sources: std::sync::Arc<SourceIndex>) -> Self {
        let object = bytecode.strip_prefix("0x").unwrap_or(bytecode);
        let Ok(code) = alloy_primitives::hex::decode(object) else {
            return Self {
                by_pc: BTreeMap::new(),
                sources,
            };
        };
        let mut previous = SourceElement::default();
        let mut entries = encoded.split(';');
        let mut by_pc = BTreeMap::new();
        let mut pc = 0;
        while pc < code.len() {
            let Some(entry) = entries.next() else {
                break;
            };
            let mut fields = entry.split(':');
            if !inherit_field(&mut previous.offset, fields.next())
                || !inherit_field(&mut previous.length, fields.next())
                || !inherit_field(&mut previous.file, fields.next())
            {
                break;
            }
            by_pc.insert(pc, previous);
            // PUSHn carries its immediate operand inline, so the next
            // instruction, and therefore the next source-map entry, is n+1 bytes
            // further along rather than one.
            let push_bytes = match code[pc] {
                0x60..=0x7f => usize::from(code[pc] - 0x5f),
                _ => 0,
            };
            pc = pc.saturating_add(1 + push_bytes);
        }
        Self { by_pc, sources }
    }

    #[cfg(test)]
    fn offsets(&self) -> Vec<usize> {
        self.by_pc.keys().copied().collect()
    }
}

impl SourceResolver for SolcSourceMap {
    fn resolve(&self, pc: usize) -> Option<SourceLocation> {
        let element = self.by_pc.get(&pc)?;
        let source = self.sources.get(&element.file)?;
        let offset = element.offset.min(source.contents.len());
        let prefix = source.contents.get(..offset)?;
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix.len() + 1, |(_, tail)| tail.len() + 1);
        Some(SourceLocation {
            path: source.path.to_string_lossy().into_owned(),
            line,
            column,
        })
    }
}

/// Apply one source-map field, treating an empty field as "inherit". Returns
/// false when the field is present but unparseable, which ends decoding.
fn inherit_field<T: std::str::FromStr>(current: &mut T, field: Option<&str>) -> bool {
    let Some(value) = field.filter(|value| !value.is_empty()) else {
        return true;
    };
    let Ok(value) = value.parse() else {
        return false;
    };
    *current = value;
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn index(contents: &str) -> Arc<SourceIndex> {
        Arc::new(SourceIndex::from([(
            0,
            SourceFile {
                path: PathBuf::from("test/Example.t.sol"),
                contents: contents.to_owned(),
            },
        )]))
    }

    #[test]
    fn entries_advance_past_push_operands() {
        // PUSH1 spans PCs 0 and 1, so the second entry annotates PC 2 and PC 1
        // never appears.
        let map = SolcSourceMap::parse("0x600100", "0:1:0;2:3:0", index("abc"));
        assert_eq!(map.offsets(), [0, 2]);
    }

    #[test]
    fn omitted_fields_inherit_the_previous_entry() {
        let map = SolcSourceMap::parse("0x0000", "4:2:0;", index("line one\nline two\n"));
        assert_eq!(map.resolve(0), map.resolve(1));
    }

    #[test]
    fn positions_are_one_based_and_count_from_the_line_start() {
        let map = SolcSourceMap::parse("0x00", "9:3:0", index("line one\nline two\n"));
        assert_eq!(
            map.resolve(0),
            Some(SourceLocation {
                path: "test/Example.t.sol".into(),
                line: 2,
                column: 1,
            })
        );
    }

    #[test]
    fn an_unparseable_entry_truncates_the_map_instead_of_failing() {
        // A future solc could add fields or change the encoding; partial
        // coverage is the intended degradation, not an error.
        let map = SolcSourceMap::parse("0x000000", "0:1:0;not-a-number:1:0;4:1:0", index("abcd"));
        assert_eq!(map.offsets(), [0]);
    }

    #[test]
    fn a_source_id_with_no_loaded_file_resolves_to_nothing() {
        let map = SolcSourceMap::parse("0x00", "0:1:7", index("abc"));
        assert_eq!(map.resolve(0), None);
    }
}
