use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("no render state available from egui")]
    NoRenderState,
    #[error("renderer resources not yet initialized")]
    ResourcesNotInitialized,
}
