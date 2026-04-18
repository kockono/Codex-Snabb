//! IDE TUI — Punto de entrada principal.
//!
//! Inicializa tracing, ejecuta la aplicación, y maneja errores
//! con exit code apropiado. Nada de lógica de negocio acá.

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

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Inicializar tracing con filtro por variable de entorno.
    // RUST_LOG=debug para desarrollo, info o warn para producción.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .init();

    // Ejecutar la aplicación. Errores se propagan con contexto.
    if let Err(err) = app::run().await {
        // Imprimir error con cadena de contexto completa
        eprintln!("Error fatal: {err:#}");
        std::process::exit(1);
    }

    Ok(())
}
