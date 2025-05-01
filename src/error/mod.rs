#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Component model unsupported, module: {0}")]
    ComponentModelUnsupported(String),
    #[error("Resolve error: {0}")]
    Resolve(crate::resolver::error::Error),
    #[error("Validation error")]
    Validation,
}
