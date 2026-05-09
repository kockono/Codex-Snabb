//! Workspace: quick open, rename y save-as.
//!
//! Modales auxiliares del workspace que sobreviven al refactor de
//! `explorer/` y `projects/`. Se mantienen en este crate hasta el
//! próximo refactor que los reubique en sus dominios definitivos.

pub mod quick_open;
pub mod quit_modal;
pub mod rename;
pub mod save_as;

// Re-exports de compatibilidad temporal (se moverán en refactor siguiente)
pub use quick_open::QuickOpenState;
