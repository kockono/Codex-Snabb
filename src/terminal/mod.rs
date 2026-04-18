//! Terminal: sesiones, IO, scrollback limitado.
//!
//! Gestiona la terminal integrada: proceso shell en worker dedicado,
//! PTY detrás de un boundary claro, scrollback acotado por líneas/bytes,
//! y render solo del viewport visible. MVP: una sesión estable.
//!
//! Status: stub — pendiente de implementación.
