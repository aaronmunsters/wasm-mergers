use std::collections::HashSet as Set;

use crate::kinds::{Function, Global, Memory, Table, Tag};
use crate::kinds::{IdentifierItem, IdentifierModule};

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub enum ResolvedExports {
    #[default]
    Remove,
    Keep,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct ExportIdentifier<KindName> {
    pub module: IdentifierModule,
    pub name: KindName,
}

pub type IdentifierFunction = IdentifierItem<Function>;
pub type IdentifierTable = IdentifierItem<Table>;
pub type IdentifierMemory = IdentifierItem<Memory>;
pub type IdentifierGlobal = IdentifierItem<Global>;
pub type IdentifierTag = IdentifierItem<Tag>;

/// The rename strategy for exports.
#[derive(Debug, Hash, Clone)]
pub struct RenameStrategy {
    pub first_occurrence: bool,
    pub functions: fn(&IdentifierModule, IdentifierFunction) -> IdentifierFunction,
    pub tables: fn(&IdentifierModule, IdentifierTable) -> IdentifierTable,
    pub memories: fn(&IdentifierModule, IdentifierMemory) -> IdentifierMemory,
    pub globals: fn(&IdentifierModule, IdentifierGlobal) -> IdentifierGlobal,
    pub tags: fn(&IdentifierModule, IdentifierTag) -> IdentifierTag,
}

impl RenameStrategy {
    #[must_use]
    pub fn functions(&self) -> &fn(&IdentifierModule, IdentifierFunction) -> IdentifierFunction {
        &self.functions
    }

    #[must_use]
    pub fn tables(&self) -> &fn(&IdentifierModule, IdentifierTable) -> IdentifierTable {
        &self.tables
    }

    #[must_use]
    pub fn memories(&self) -> &fn(&IdentifierModule, IdentifierMemory) -> IdentifierMemory {
        &self.memories
    }

    #[must_use]
    pub fn globals(&self) -> &fn(&IdentifierModule, IdentifierGlobal) -> IdentifierGlobal {
        &self.globals
    }

    #[must_use]
    pub fn tags(&self) -> &fn(&IdentifierModule, IdentifierTag) -> IdentifierTag {
        &self.tags
    }
}

#[derive(Debug, Default, Hash, Clone)]
pub enum ClashingExports {
    Rename(RenameStrategy),
    #[default]
    Signal,
}

#[derive(Debug, Default, Hash, Clone)]
pub enum LinkTypeMismatch {
    Ignore,
    #[default]
    Signal,
}

#[derive(Debug, Clone, Default)]
pub struct KeepExports {
    pub functions: Set<ExportIdentifier<IdentifierFunction>>,
    pub tables: Set<ExportIdentifier<IdentifierTable>>,
    pub memories: Set<ExportIdentifier<IdentifierMemory>>,
    pub globals: Set<ExportIdentifier<IdentifierGlobal>>,
    pub tags: Set<ExportIdentifier<IdentifierTag>>,
}

impl KeepExports {
    #[must_use]
    pub fn functions(&self) -> &Set<ExportIdentifier<IdentifierFunction>> {
        &self.functions
    }

    #[must_use]
    pub fn tables(&self) -> &Set<ExportIdentifier<IdentifierTable>> {
        &self.tables
    }

    #[must_use]
    pub fn memories(&self) -> &Set<ExportIdentifier<IdentifierMemory>> {
        &self.memories
    }

    #[must_use]
    pub fn globals(&self) -> &Set<ExportIdentifier<IdentifierGlobal>> {
        &self.globals
    }

    #[must_use]
    pub fn tags(&self) -> &Set<ExportIdentifier<IdentifierTag>> {
        &self.tags
    }

    pub fn keep_function(&mut self, module: IdentifierModule, name: String) {
        let name = name.into();
        let identifier = ExportIdentifier { module, name };
        self.functions.insert(identifier);
    }

    pub fn keep_tables(&mut self, module: IdentifierModule, name: String) {
        let name = name.into();
        let identifier = ExportIdentifier { module, name };
        self.tables.insert(identifier);
    }

    pub fn keep_memory(&mut self, module: IdentifierModule, name: String) {
        let name = name.into();
        let identifier = ExportIdentifier { module, name };
        self.memories.insert(identifier);
    }

    pub fn keep_globals(&mut self, module: IdentifierModule, name: String) {
        let name = name.into();
        let identifier = ExportIdentifier { module, name };
        self.globals.insert(identifier);
    }
}

#[derive(Debug, Default, Clone)]
pub struct MergeOptions {
    pub clashing_exports: ClashingExports,
    pub link_type_mismatch: LinkTypeMismatch,
    pub resolved_exports: ResolvedExports,
    pub keep_exports: Option<KeepExports>,
}

/// Default rename strategy provided by this library is to rename each duplicate
/// items by joining the namespace with the export name with `:` inbetween.
/// See [`default_rename`](default_rename).
pub const DEFAULT_RENAMER: RenameStrategy = RenameStrategy {
    first_occurrence: true,
    functions: default_rename,
    tables: default_rename,
    memories: default_rename,
    globals: default_rename,
    tags: default_rename,
};

/// Default rename strategy provided by this library is to rename duplicate
/// items by joining the namespace with the export name.
///
/// Eg. merging the following:
/// ```text
/// (mod "A" (export "f" x))
/// (mod "B" (export "f" y))
/// ```
/// yields:
/// ```text
/// (mod (export "A:f" x)
///      (export "B:f" y))
/// ```
pub fn default_rename<T: Into<String> + From<String>>(m: &IdentifierModule, v: T) -> T {
    let v = v.into();
    format!("{m}:{v}").into()
}
