#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Component model unsupported, module: {0}")]
    ComponentModelUnsupported(String),
    #[error("Validation error")]
    Validation,
}
