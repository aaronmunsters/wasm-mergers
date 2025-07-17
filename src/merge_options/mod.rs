use crate::resolver::FunctionName;
use crate::resolver::GlobalName;
use crate::resolver::MemoryName;
use crate::resolver::ModuleName;
use crate::resolver::TableName;

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub enum ResolvedExports {
    #[default]
    Remove,
    Keep,
}

#[derive(Debug, Hash, Clone)]
pub struct RenameStrategy {
    pub functions: fn(ModuleName, FunctionName) -> FunctionName,
    pub tables: fn(ModuleName, TableName) -> TableName,
    pub memory: fn(ModuleName, MemoryName) -> MemoryName,
    pub globals: fn(ModuleName, GlobalName) -> GlobalName,
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

#[derive(Debug, Default, Hash, Clone)]
pub struct MergeOptions {
    pub clashing_exports: ClashingExports,
    pub link_type_mismatch: LinkTypeMismatch,
    pub resolved_exports: ResolvedExports,
}

/// Default rename strategy provided by this library is to rename each duplicate
/// items by joining the namespace with the export name with `:` inbetween.
/// See [`default_rename`](default_rename).
pub const DEFAULT_RENAMER: RenameStrategy = RenameStrategy {
    functions: default_rename,
    tables: default_rename,
    memory: default_rename,
    globals: default_rename,
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
fn default_rename<T: AsRef<str> + From<String>>(ModuleName(m): ModuleName, v: T) -> T {
    format!("{m}:{}", v.as_ref()).into()
}
