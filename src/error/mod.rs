use crate::resolver::resolution_schema::ValidationFailure;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Parsing failed: {0}")]
    Parse(anyhow::Error),
    #[error("Component model unsupported, module: {0}")]
    ComponentModelUnsupported(String),
    #[error("Validation error {0:?}")]
    Validation(Box<ValidationFailure>),
    #[error("Duplicate name \"{0}\" export for same type: {1:?}")]
    DuplicateNameExport(String, ExportKind),
}

#[derive(Debug)]
pub enum ExportKind {
    Function,
    Table,
    Memory,
    Global,
}
