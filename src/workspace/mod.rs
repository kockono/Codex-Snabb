//! Workspace: explorer de archivos, quick open, project manager, recientes.
//!
//! Gestiona la navegación del workspace: árbol lazy de archivos,
//! lista de proyectos recientes, y el índice liviano de paths para
//! quick open. Todo con refresh controlado y sin indexación agresiva.

pub mod explorer;
pub mod project;
pub mod quick_open;
pub mod tree;

pub use explorer::ExplorerState;
pub use quick_open::QuickOpenState;
