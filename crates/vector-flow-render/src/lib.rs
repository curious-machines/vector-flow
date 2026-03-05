pub mod error;
pub mod vertex;
pub mod camera;
pub mod batch;
pub mod renderer;
pub mod overlay;

pub use camera::{Camera, CameraUniform};
pub use batch::{CollectedShape, DrawBatch, PreparedScene, collect_shapes, prepare_scene};
pub use renderer::{CanvasRenderer, PrimitiveUniform};
pub use overlay::{CanvasCallback, CanvasRenderResources, canvas_paint_callback};
pub use vertex::CanvasVertex;
pub use error::RenderError;
