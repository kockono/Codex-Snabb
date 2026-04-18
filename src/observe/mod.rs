//! Observe: métricas, timers, budget inspectors.
//!
//! Observabilidad desde el día 1. Este módulo provee instrumentación
//! mínima pero funcional para trackear performance del sistema:
//! frame time, input-to-render latency, conteo de eventos, y frames
//! dropeados. Visible en logs y futuro panel de diagnóstico interno.
//!
//! Ninguna estructura acá aloca en heap ni usa canales. Todo es
//! contadores in-place actualizados síncronamente por el event loop.

use std::time::Instant;

// ─── Metrics ───────────────────────────────────────────────────────────────────

/// Métricas de performance del sistema.
///
/// Contadores in-place actualizados síncronamente por el event loop.
/// Sin allocaciones, sin canales, sin overhead. El event loop llama
/// a los métodos `record_*` después de cada operación medida.
///
/// El promedio de frame time usa media móvil exponencial (EMA) para
/// dar más peso a frames recientes sin necesitar un buffer circular.
#[derive(Debug)]
pub struct Metrics {
    /// Cantidad total de frames renderizados.
    pub frame_count: u64,
    /// Duración del último frame en microsegundos.
    pub last_frame_time_us: u64,
    /// Promedio móvil de frame time en microsegundos (EMA).
    pub avg_frame_time_us: u64,
    /// Latencia del último input-to-render en microsegundos.
    pub last_input_latency_us: u64,
    /// Cantidad total de eventos procesados.
    pub event_count: u64,
    /// Cantidad de frames que excedieron el target de latencia.
    pub dropped_frames: u64,
}

impl Metrics {
    /// Crea métricas inicializadas en cero.
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            last_frame_time_us: 0,
            avg_frame_time_us: 0,
            last_input_latency_us: 0,
            event_count: 0,
            dropped_frames: 0,
        }
    }

    /// Registra la duración de un frame.
    ///
    /// Actualiza `last_frame_time_us`, `avg_frame_time_us` (EMA con
    /// alpha = 0.1 para suavizar outliers), `frame_count`, y detecta
    /// frames que exceden el target de 16ms para `dropped_frames`.
    pub fn record_frame(&mut self, duration_us: u64) {
        self.frame_count += 1;
        self.last_frame_time_us = duration_us;

        // EMA: avg = avg * 0.9 + sample * 0.1
        // Para el primer frame, usar el valor directo.
        if self.frame_count == 1 {
            self.avg_frame_time_us = duration_us;
        } else {
            // Aritmética entera para evitar float:
            // avg = (avg * 9 + sample) / 10
            self.avg_frame_time_us = (self.avg_frame_time_us * 9 + duration_us) / 10;
        }

        // Frame que excede target de 16ms se cuenta como "dropped"
        if duration_us > crate::core::budgets::DEFAULT_BUDGETS.input_to_render_target_us {
            self.dropped_frames += 1;
        }
    }

    /// Registra la latencia de un ciclo input-to-render.
    pub fn record_input_latency(&mut self, latency_us: u64) {
        self.last_input_latency_us = latency_us;
    }

    /// Registra que se procesó un evento.
    pub fn record_event(&mut self) {
        self.event_count += 1;
    }

    /// Resetea todas las métricas a cero.
    ///
    /// Útil para benchmarking o cuando se necesita una ventana limpia.
    #[expect(dead_code, reason = "se usará para panel de diagnóstico y tests")]
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

// ─── FrameTimer ────────────────────────────────────────────────────────────────

/// Timer liviano para medir la duración de un frame.
///
/// Se crea al inicio de un frame con `FrameTimer::start()` y se
/// consulta al final con `elapsed_us()`. Sin allocaciones.
///
/// ```ignore
/// let timer = FrameTimer::start();
/// // ... render frame ...
/// let duration = timer.elapsed_us();
/// metrics.record_frame(duration);
/// ```
#[derive(Debug)]
pub struct FrameTimer {
    /// Instante en que se inició la medición.
    start: Instant,
}

impl FrameTimer {
    /// Inicia un nuevo timer capturando el instante actual.
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Retorna los microsegundos transcurridos desde el inicio.
    ///
    /// Se puede llamar múltiples veces — el timer no se consume.
    pub fn elapsed_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }
}
