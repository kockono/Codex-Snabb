//! IDs tipados: identificadores únicos por subsistema.
//!
//! Cada entidad del sistema tiene un ID tipado basado en `NonZeroU32`
//! para aprovechar niche optimization: `Option<BufferId>` ocupa los
//! mismos 4 bytes que `BufferId` solo, sin overhead de discriminante.
//!
//! Los IDs se generan con contadores atómicos monotónicos. Son únicos
//! dentro de una sesión de la aplicación, no entre sesiones.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};

/// Identificador único de buffer.
///
/// Cada archivo abierto en el editor recibe un `BufferId` único.
/// Basado en `NonZeroU32` para niche optimization:
/// `Option<BufferId>` = 4 bytes (igual que `BufferId`).
///
/// El counter atómico garantiza unicidad sin mutex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(NonZeroU32);

#[expect(
    dead_code,
    reason = "se usará en épica 2 — editor base para identificar buffers"
)]
impl BufferId {
    /// Genera el siguiente `BufferId` único.
    ///
    /// Thread-safe via `AtomicU32`. El counter empieza en 1 y solo
    /// incrementa, así que `NonZeroU32::new` nunca retorna `None`
    /// (a menos que se generen 2^32 - 1 buffers, lo cual no va a pasar).
    ///
    /// Nota: `expect` acá es aceptable porque es un invariante de programa
    /// (counter empieza en 1 y solo incrementa). Un overflow de u32 en
    /// un contador de buffers indica un bug catastrófico, no un error
    /// recuperable.
    pub fn next() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(1);
        let val = COUNTER.fetch_add(1, Ordering::Relaxed);
        // SAFETY: counter empieza en 1 y solo incrementa. Overflow de u32
        // requeriría abrir ~4 mil millones de buffers en una sesión.
        Self(NonZeroU32::new(val).expect("BufferId counter overflow — se generaron 2^32 IDs"))
    }

    /// Retorna el valor numérico interno del ID.
    ///
    /// Útil para logging y serialización.
    #[expect(dead_code, reason = "se usará para logging y serialización de buffers")]
    pub fn as_u32(self) -> u32 {
        self.0.get()
    }
}
