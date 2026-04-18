//! Theme: sistema de theming cyberpunk con tokens semánticos precomputados.
//!
//! Paleta fija y precomputada en un solo `Theme`. Tokens semánticos para
//! mantener consistencia visual sin costo de cómputo en render. Los colores
//! se calculan una vez al inicio y se pasan por referencia a cada render.
//!
//! Cero gradientes, cero animaciones, cero cómputo dinámico por frame.

use ratatui::style::Color;

/// Tema visual completo con tokens semánticos precomputados.
///
/// Cada campo es un `Color::Rgb(r, g, b)` para máximo control visual.
/// El tema se crea UNA vez fuera del event loop y se pasa por `&Theme`
/// a todas las funciones de render. Nada de esto aloca en heap.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // ── Backgrounds ──
    /// Fondo principal oscuro (casi negro con tinte azul).
    pub bg_primary: Color,
    /// Fondo de sidebar/panels (ligeramente más claro que primario).
    pub bg_secondary: Color,
    /// Fondo del panel enfocado (sutil diferencia para indicar foco).
    pub bg_active: Color,
    /// Fondo para hover states (futuro — mouse hover).
    #[expect(dead_code, reason = "se usará cuando se implemente mouse hover")]
    pub bg_hover: Color,
    /// Fondo de la status bar.
    pub bg_status: Color,

    // ── Foregrounds ──
    /// Texto principal legible sobre fondos oscuros.
    pub fg_primary: Color,
    /// Texto secundario/dimmed para información de soporte.
    pub fg_secondary: Color,
    /// Acento principal — cyan eléctrico cyberpunk.
    pub fg_accent: Color,
    /// Acento alternativo — magenta/pink cyberpunk.
    pub fg_accent_alt: Color,
    /// Color de advertencia — amarillo cálido.
    #[expect(dead_code, reason = "se usará para diagnósticos y warnings")]
    pub fg_warning: Color,
    /// Color de error — rojo brillante.
    #[expect(dead_code, reason = "se usará para errores y diagnósticos")]
    pub fg_error: Color,
    /// Color de éxito — verde neón.
    #[expect(dead_code, reason = "se usará para estados de éxito")]
    pub fg_success: Color,

    // ── Semánticos ──
    /// Selección de texto (fondo de selección).
    pub selection: Color,
    /// Color del cursor (reservado para modos visuales futuros, ej: VISUAL mode highlight).
    #[expect(
        dead_code,
        reason = "reservado para cursor visual en modos como VISUAL/INSERT"
    )]
    pub cursor: Color,
    /// Números de línea normales.
    pub line_number: Color,
    /// Número de línea activa (línea del cursor).
    pub line_number_active: Color,
    /// Borde del panel enfocado — cyan eléctrico prominente.
    pub border_focused: Color,
    /// Borde de paneles no enfocados — gris oscuro sutil.
    pub border_unfocused: Color,
    /// Git: línea agregada — verde.
    pub diff_add: Color,
    /// Git: línea eliminada — rojo.
    pub diff_remove: Color,
    /// Coincidencia de búsqueda — fondo highlight.
    pub search_match: Color,
}

impl Theme {
    /// Tema default oscuro cyberpunk.
    ///
    /// Paleta inspirada en estéticas cyberpunk sobrias:
    /// - Fondos muy oscuros con tinte azulado
    /// - Cyan eléctrico (`#00d4ff`) como acento principal
    /// - Magenta (`#e91e8c`) como acento secundario
    /// - Alto contraste en foco, selección y bordes activos
    /// - Texto principal gris claro para legibilidad prolongada
    pub fn cyberpunk() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::Rgb(10, 14, 20), // #0a0e14 — base muy oscura
            bg_secondary: Color::Rgb(13, 17, 23), // #0d1117 — panels/sidebar
            bg_active: Color::Rgb(18, 24, 33),  // #121821 — panel enfocado
            bg_hover: Color::Rgb(22, 29, 39),   // #161d27 — hover sutil
            bg_status: Color::Rgb(8, 11, 16),   // #080b10 — status bar más oscura

            // Foregrounds
            fg_primary: Color::Rgb(212, 212, 212), // #d4d4d4 — texto principal
            fg_secondary: Color::Rgb(106, 115, 125), // #6a737d — texto dimmed
            fg_accent: Color::Rgb(0, 212, 255),    // #00d4ff — cyan eléctrico
            fg_accent_alt: Color::Rgb(233, 30, 140), // #e91e8c — magenta/pink
            fg_warning: Color::Rgb(255, 203, 0),   // #ffcb00 — amarillo
            fg_error: Color::Rgb(248, 81, 73),     // #f85149 — rojo
            fg_success: Color::Rgb(63, 185, 80),   // #3fb950 — verde

            // Semánticos
            selection: Color::Rgb(38, 79, 120), // #264f78 — selección azul oscuro
            cursor: Color::Rgb(0, 212, 255),    // #00d4ff — cursor cyan
            line_number: Color::Rgb(72, 79, 88), // #484f58 — line numbers dimmed
            line_number_active: Color::Rgb(212, 212, 212), // #d4d4d4 — línea activa
            border_focused: Color::Rgb(0, 212, 255), // #00d4ff — cyan eléctrico
            border_unfocused: Color::Rgb(48, 54, 61), // #30363d — gris oscuro
            diff_add: Color::Rgb(63, 185, 80),  // #3fb950 — verde
            diff_remove: Color::Rgb(248, 81, 73), // #f85149 — rojo
            search_match: Color::Rgb(159, 130, 0), // #9f8200 — amarillo oscuro
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::cyberpunk()
    }
}
