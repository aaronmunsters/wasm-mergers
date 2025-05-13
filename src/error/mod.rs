use crate::resolver::resolution_schema::ValidationFailure;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Component model unsupported, module: {0}")]
    ComponentModelUnsupported(String),
    #[error("Validation error {0:?}")]
    Validation(Box<ValidationFailure>),
    #[error("Duplicate name export for same type: {0}")]
    DuplicateNameExport(String),
}
