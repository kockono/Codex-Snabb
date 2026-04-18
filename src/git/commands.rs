//! Git commands: wrapper sobre el binario `git` via `std::process::Command`.
//!
//! Ejecuta comandos git como subprocesos y parsea la salida.
//! Estrategia: usar el `git` instalado del usuario — más liviano que libgit2,
//! sin dependencias pesadas, suficiente para MVP.
//!
//! Todas las funciones son síncronas (ok para MVP — los comandos son rápidos
//! para repos normales). Si `git` no está instalado, se maneja gracefully.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

// ─── Types ─────────────────────────────────────────────────────────────────────

/// Estado de cambio de un archivo en el working tree o index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeType {
    /// Archivo modificado respecto al último commit.
    Modified,
    /// Archivo nuevo agregado al index.
    Added,
    /// Archivo eliminado.
    Deleted,
    /// Archivo renombrado.
    Renamed,
    /// Archivo no rastreado por git.
    Untracked,
    /// Archivo copiado.
    Copied,
}

/// Estado de un archivo individual en el repo.
///
/// Un archivo puede aparecer dos veces en la lista: una vez staged
/// y otra unstaged, si tiene cambios en ambos.
#[derive(Debug, Clone)]
pub struct GitFileStatus {
    /// Path relativo al root del repo.
    pub path: String,
    /// Tipo de cambio (modified, added, deleted, etc.).
    pub status: FileChangeType,
    /// Si el cambio está en el staging area (index).
    pub staged: bool,
}

// ─── Commands ──────────────────────────────────────────────────────────────────

/// Ejecuta un comando git en el directorio dado y retorna stdout.
///
/// Retorna `Err` si el comando falla o git no está disponible.
fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .context("no se pudo ejecutar git — ¿está instalado?")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} falló: {}", args.join(" "), stderr.trim());
    }
}

/// Verifica si un directorio es un repo git.
///
/// Ejecuta `git rev-parse --git-dir` — retorna `true` si el comando tiene éxito.
pub fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(path)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Obtiene el nombre del branch actual.
///
/// Ejecuta `git branch --show-current`. Retorna string vacío si está en detached HEAD.
pub fn current_branch(repo_path: &Path) -> Result<String> {
    let output = run_git(repo_path, &["branch", "--show-current"])?;
    Ok(output.trim().to_string())
}

/// Obtiene el status de archivos del repo.
///
/// Ejecuta `git status --porcelain=v1` y parsea la salida.
/// Un archivo puede aparecer dos veces: staged + unstaged.
pub fn status(repo_path: &Path) -> Result<Vec<GitFileStatus>> {
    let output = run_git(repo_path, &["status", "--porcelain=v1"])?;
    let mut files = Vec::with_capacity(output.lines().count());

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }

        let bytes = line.as_bytes();
        let index_status = bytes[0];
        let worktree_status = bytes[1];
        // El path empieza en posición 3 (después de "XY ")
        let path = &line[3..];

        // Manejar renamed: "R  old -> new" — tomar el path nuevo
        let file_path = if let Some(arrow_pos) = path.find(" -> ") {
            &path[arrow_pos + 4..]
        } else {
            path
        };

        // Cambios en el index (staged)
        if index_status != b' '
            && index_status != b'?'
            && let Some(change_type) = parse_status_char(index_status)
        {
            files.push(GitFileStatus {
                path: file_path.to_string(),
                status: change_type,
                staged: true,
            });
        }

        // Cambios en el working tree (unstaged)
        if worktree_status != b' '
            && let Some(change_type) = parse_status_char(worktree_status)
        {
            files.push(GitFileStatus {
                path: file_path.to_string(),
                status: change_type,
                staged: false,
            });
        }
    }

    Ok(files)
}

/// Obtiene el diff de un archivo específico.
///
/// Para archivos unstaged: `git diff -- <file>`
/// Para archivos staged: `git diff --cached -- <file>`
pub fn diff_file(repo_path: &Path, file_path: &str, staged: bool) -> Result<String> {
    let args = if staged {
        vec!["diff", "--cached", "--", file_path]
    } else {
        vec!["diff", "--", file_path]
    };
    run_git(repo_path, &args)
}

/// Agrega un archivo al staging area.
///
/// Ejecuta `git add -- <file>`.
pub fn stage_file(repo_path: &Path, file_path: &str) -> Result<()> {
    run_git(repo_path, &["add", "--", file_path])?;
    Ok(())
}

/// Quita un archivo del staging area (restore --staged).
///
/// Ejecuta `git restore --staged -- <file>`.
pub fn unstage_file(repo_path: &Path, file_path: &str) -> Result<()> {
    run_git(repo_path, &["restore", "--staged", "--", file_path])?;
    Ok(())
}

/// Ejecuta un commit con el mensaje dado.
///
/// Ejecuta `git commit -m <message>`. Retorna el output del commit.
pub fn commit(repo_path: &Path, message: &str) -> Result<String> {
    run_git(repo_path, &["commit", "-m", message])
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

/// Parsea un carácter de status de `git status --porcelain=v1`.
///
/// Formato: `M` = modified, `A` = added, `D` = deleted, `R` = renamed,
/// `?` = untracked, `C` = copied.
fn parse_status_char(ch: u8) -> Option<FileChangeType> {
    match ch {
        b'M' => Some(FileChangeType::Modified),
        b'A' => Some(FileChangeType::Added),
        b'D' => Some(FileChangeType::Deleted),
        b'R' => Some(FileChangeType::Renamed),
        b'?' => Some(FileChangeType::Untracked),
        b'C' => Some(FileChangeType::Copied),
        _ => None,
    }
}
