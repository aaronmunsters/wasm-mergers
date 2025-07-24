use std::collections::HashSet as Set;

use crate::kinds::{Function, Global, IdentifierItem, IdentifierModule, Memory, Table};

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

#[derive(Debug, Hash, Clone)]
pub struct RenameStrategy {
    pub functions: fn(&IdentifierModule, IdentifierFunction) -> IdentifierFunction,
    pub tables: fn(&IdentifierModule, IdentifierTable) -> IdentifierTable,
    pub memories: fn(&IdentifierModule, IdentifierMemory) -> IdentifierMemory,
    pub globals: fn(&IdentifierModule, IdentifierGlobal) -> IdentifierGlobal,
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
    pub functions: Set<ExportIdentifier<IdentifierItem<Function>>>,
    pub tables: Set<ExportIdentifier<IdentifierItem<Table>>>,
    pub memories: Set<ExportIdentifier<IdentifierItem<Memory>>>,
    pub globals: Set<ExportIdentifier<IdentifierItem<Global>>>,
}

impl KeepExports {
    #[must_use]
    pub fn functions(&self) -> &Set<ExportIdentifier<IdentifierItem<Function>>> {
        &self.functions
    }

    #[must_use]
    pub fn tables(&self) -> &Set<ExportIdentifier<IdentifierItem<Table>>> {
        &self.tables
    }

    #[must_use]
    pub fn memories(&self) -> &Set<ExportIdentifier<IdentifierItem<Memory>>> {
        &self.memories
    }

    #[must_use]
    pub fn globals(&self) -> &Set<ExportIdentifier<IdentifierItem<Global>>> {
        &self.globals
    }

    pub fn keep_function(&mut self, module: IdentifierModule, name: String) {
        let identifier: ExportIdentifier<IdentifierItem<Function>> = ExportIdentifier {
            module,
            name: name.into(),
        };
        self.functions.insert(identifier);
    }

    pub fn keep_tables(&mut self, module: IdentifierModule, name: String) {
        let identifier: ExportIdentifier<IdentifierItem<Table>> = ExportIdentifier {
            module,
            name: name.into(),
        };
        self.tables.insert(identifier);
    }

    pub fn keep_memory(&mut self, module: IdentifierModule, name: String) {
        let identifier: ExportIdentifier<IdentifierItem<Memory>> = ExportIdentifier {
            module,
            name: name.into(),
        };
        self.memories.insert(identifier);
    }

    pub fn keep_globals(&mut self, module: IdentifierModule, name: String) {
        let identifier: ExportIdentifier<IdentifierItem<Global>> = ExportIdentifier {
            module,
            name: name.into(),
        };
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
    functions: default_rename::<IdentifierFunction>,
    tables: default_rename::<IdentifierTable>,
    memories: default_rename::<IdentifierMemory>,
    globals: default_rename::<IdentifierGlobal>,
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
fn default_rename<T: Into<String> + From<String>>(m: &IdentifierModule, v: T) -> T {
    let v = v.into();
    format!("{m}:{v}").into()
}
