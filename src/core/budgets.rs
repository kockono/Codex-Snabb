//! Budgets: límites de performance por subsistema.
//!
//! Define los presupuestos duros de RAM, latencia y CPU que toda
//! feature debe respetar. Estos valores vienen de `architecture.md`
//! y son restricciones de diseño, no sugerencias.
//!
//! Cada subsistema puede consultar sus límites para auto-regularse:
//! truncar scrollback, cancelar operaciones lentas, o degradar
//! calidad antes de violar un budget.

/// Límites de performance del sistema.
///
/// Constantes derivadas de `architecture.md`. Todos los subsistemas
/// deben operar dentro de estos límites. Si una feature los viola,
/// se degrada o se rechaza — no se relajan los límites.
#[derive(Debug, Clone, Copy)]
pub struct BudgetLimits {
    /// RAM máxima en idle sin LSP activo (bytes).
    #[expect(dead_code, reason = "se usará para validación de budgets en runtime")]
    pub idle_ram_bytes: u64,
    /// RAM máxima en uso normal con buffers abiertos (bytes).
    #[expect(dead_code, reason = "se usará para validación de budgets en runtime")]
    pub working_ram_bytes: u64,
    /// Latencia target de input-to-render en microsegundos (16ms = ~60fps).
    pub input_to_render_target_us: u64,
    /// Latencia hard limit de input-to-render en microsegundos (33ms = ~30fps).
    pub input_to_render_hard_us: u64,
    /// Tiempo máximo de arranque en frío en milisegundos.
    #[expect(dead_code, reason = "se usará para medición de startup time")]
    pub cold_startup_ms: u64,
}

/// Budgets por defecto derivados de `architecture.md`.
///
/// - Idle RAM sin LSP: < 85 MB
/// - RAM en uso normal: < 100 MB
/// - Input-to-render target: < 16 ms
/// - Input-to-render hard limit: < 33 ms
/// - Cold startup: < 150 ms
pub const DEFAULT_BUDGETS: BudgetLimits = BudgetLimits {
    idle_ram_bytes: 85 * 1024 * 1024,     // 85 MB
    working_ram_bytes: 100 * 1024 * 1024, // 100 MB
    input_to_render_target_us: 16_000,    // 16 ms
    input_to_render_hard_us: 33_000,      // 33 ms
    cold_startup_ms: 150,
};

impl Default for BudgetLimits {
    fn default() -> Self {
        DEFAULT_BUDGETS
    }
}

impl BudgetLimits {
    /// Verifica si una latencia de frame excede el target (pero no el hard limit).
    ///
    /// Útil para logging de advertencia sin panic.
    pub fn frame_exceeds_target(&self, frame_time_us: u64) -> bool {
        frame_time_us > self.input_to_render_target_us
    }

    /// Verifica si una latencia de frame viola el hard limit.
    ///
    /// Indica un problema serio que debe investigarse.
    #[expect(dead_code, reason = "se usará para alertas críticas de latencia")]
    pub fn frame_exceeds_hard_limit(&self, frame_time_us: u64) -> bool {
        frame_time_us > self.input_to_render_hard_us
    }
}
