//! IDE TUI — Punto de entrada principal.
//!
//! Inicializa tracing a archivo, parsea argumentos de línea de comandos,
//! ejecuta la aplicación, y maneja errores con exit code apropiado.
//! Nada de lógica de negocio acá.
//!
//! Uso: `ide-tui [archivo]`
//! Si se pasa un archivo, se abre al iniciar.

mod app;
mod core;
mod editor;
mod git;
mod lsp;
mod observe;
mod search;
mod terminal;
mod ui;
mod workspace;

use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Inicializar tracing redirigido a archivo.
    //
    // Los logs van a `ide-tui.log` para no interferir con el alternate
    // screen de ratatui. Si no se puede crear el archivo, la app
    // continúa sin logs — nunca crashear por observabilidad.
    if let Ok(log_file) = File::create("ide-tui.log") {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_target(false)
            .with_file(true)
            .with_line_number(true)
            .with_writer(Mutex::new(log_file))
            .with_ansi(false) // sin colores ANSI en archivo de log
            .init();
    }

    // Parsear argumento de archivo opcional: `ide-tui [file]`
    let file: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

    // Ejecutar la aplicación. Errores se propagan con contexto.
    if let Err(err) = app::run(file).await {
        // Imprimir error con cadena de contexto completa
        eprintln!("Error fatal: {err:#}");
        std::process::exit(1);
    }

    Ok(())
}
