#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// When parsing of a WebAssembly module failed.
    ///
    /// Since parsing can fail in multiple ways,
    /// this variant wraps multiple failures as an
    /// anyhow error.
    #[error("Parsing failed: {0}")]
    Parse(anyhow::Error),

    /// Infinite Import Cycle
    ///
    /// This occurs when two or more modules import from each other
    /// without any of them providing a concrete definition.
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (import "b")
    ///             (export "a" $b))
    /// (module "B" (import "a")
    ///             (export "b" $a))
    /// ```
    /// Here, `A`'s `"a"` is just `B`'s `"b"`, and `B`'s `"b"` is just `A`'s `"a"`.
    /// No actual function is defined anywhere, so resolution is not possible.
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
    /// If no other module imports `"f"`, then `"M"`
    /// would result in a `Map { "f" -> { A:f, B:f } }`.
    #[error("Export Name Clash")]
    ExportNameClash, // TODO: clashing names should be reported + module
}
