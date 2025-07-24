#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Parsing failed: {0}")]
    Parse(anyhow::Error),
    #[error("Component model unsupported, module: {0}")]
    ComponentModelUnsupported(String),
    #[error("Infinite Import Cycle")]
    ImportCycle,
    /// Types Mismatch
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f" (result i32)))
    /// (module "B" (import "A" "f" (result i64)))
    /// (module "C" (import "A" "f" (result f64)))
    /// ```
    /// Would result in a `Set { A:f:i32 -> { B:f:i64, C:f:f64 } }`.
    #[error("Type Mismatch")]
    TypeMismatch, // TODO: type mismatch should report conflicting types
    /// Name Clashes
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f")) ;; (a)
    /// (module "B" (export "f")) ;; (b)
    /// ;; ==>
    /// (module "M" (export "f")) ;; (a) or (b) ?
    /// ```
    ///
    /// If no other module imports "f", then M
    /// Would result in a `Map { "f" -> { A:f, B:f } }`.
    #[error("Export Name Clash")]
    ExportNameClash, // TODO: clashing names should be reported + module
    #[error("Duplicate name \"{0}\" export for same type: {1:?}")]
    DuplicateNameExport(String, ExportKind),
}

/// An exported item.
#[derive(Copy, Clone, Debug)]
pub enum ExportKind {
    /// An exported function.
    Function,
    /// An exported table.
    Table,
    /// An exported memory.
    Memory,
    /// An exported global.
    Global,
}
