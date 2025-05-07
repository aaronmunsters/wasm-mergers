#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub struct MergeOptions {
    /// Rename duplicate exports.
    ///
    /// Current strategy is to prefix the exports with the module-name
    /// and the separator `:`.
    pub rename_duplicate_exports: bool,
}
