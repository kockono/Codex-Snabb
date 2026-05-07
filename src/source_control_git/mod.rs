//! Git: status, diff, stage/unstage, commit básico, branch picker.
//!
//! Integración austera con Git: snapshot de estado, panel de cambios,
//! diff por archivo, stage/unstage por archivo, commit básico y
//! selección de rama via branch picker.
//! Usa el binario `git` directamente — sin libgit2.

pub mod branch_picker;
pub mod commands;
pub mod render;

use std::path::Path;

use commands::GitFileStatus;

// ─── GitState ──────────────────────────────────────────────────────────────────

/// Estado completo del panel de Git / source control.
///
/// Contiene el snapshot actual del repo: branch, archivos cambiados,
/// diff del archivo seleccionado, y estado del input de commit.
#[derive(Debug)]
pub struct GitState {
    /// Si el panel está visible en la sidebar.
    pub visible: bool,
    /// Si el directorio actual es un repo git.
    pub is_repo: bool,
    /// Nombre del branch actual (vacío si detached HEAD).
    pub branch: String,
    /// Commits locales por delante del upstream (pendientes de push).
    pub ahead: u32,
    /// Commits remotos por detrás del upstream (pendientes de pull).
    pub behind: u32,
    /// Lista de archivos con cambios (staged + unstaged).
    pub files: Vec<GitFileStatus>,
    /// Índice del archivo seleccionado en la lista.
    pub selected_index: usize,
    /// Offset de scroll para la lista de archivos.
    pub scroll_offset: usize,
    /// Contenido del diff del archivo seleccionado (si se pidió).
    pub diff_content: Option<String>,
    /// Offset de scroll del diff.
    pub diff_scroll: usize,
    /// Texto del mensaje de commit que el usuario está escribiendo.
    pub commit_input: String,
    /// Si el usuario está en modo commit (escribiendo mensaje).
    pub commit_mode: bool,
    /// Si se está mostrando el diff del archivo seleccionado.
    pub show_diff: bool,
    /// Si el contenido en `diff_content` es el archivo completo (true)
    /// o un diff real (false). Afecta el título del panel.
    pub showing_file_content: bool,
}

impl GitState {
    /// Crea un nuevo estado de Git vacío (no visible).
    pub fn new() -> Self {
        Self {
            visible: false,
            is_repo: false,
            branch: String::new(),
            ahead: 0,
            behind: 0,
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            diff_content: None,
            diff_scroll: 0,
            commit_input: String::with_capacity(128),
            commit_mode: false,
            show_diff: false,
            showing_file_content: false,
        }
    }

    /// Refresca el status del repo: branch y archivos cambiados.
    ///
    /// Si el directorio no es un repo, marca `is_repo = false` y limpia.
    /// Si git no está disponible, maneja gracefully sin crash.
    pub fn refresh(&mut self, repo_path: &Path) {
        self.is_repo = commands::is_git_repo(repo_path);

        if !self.is_repo {
            self.branch.clear();
            self.files.clear();
            self.selected_index = 0;
            self.diff_content = None;
            self.ahead = 0;
            self.behind = 0;
            return;
        }

        // Branch actual
        match commands::current_branch(repo_path) {
            Ok(branch) => {
                self.branch.clear();
                self.branch.push_str(&branch);
            }
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo obtener branch actual");
                self.branch.clear();
                self.branch.push_str("(detached)");
            }
        }

        // Ahead/behind respecto al upstream
        let (ahead, behind) = commands::ahead_behind(repo_path);
        self.ahead = ahead;
        self.behind = behind;

        // Status de archivos
        match commands::status(repo_path) {
            Ok(files) => {
                self.files = files;
            }
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo obtener git status");
                self.files.clear();
            }
        }

        // Clampear selección al nuevo tamaño de la lista
        if !self.files.is_empty() {
            self.selected_index = self.selected_index.min(self.files.len() - 1);
        } else {
            self.selected_index = 0;
        }

        // Limpiar diff al refrescar (puede haber cambiado)
        self.diff_content = None;
        self.show_diff = false;
        self.diff_scroll = 0;
        self.showing_file_content = false;
    }

    /// Mover selección hacia arriba en la lista de archivos.
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Mover selección hacia abajo en la lista de archivos.
    pub fn move_down(&mut self) {
        if !self.files.is_empty() && self.selected_index < self.files.len() - 1 {
            self.selected_index += 1;
        }
    }

    /// Toggle stage/unstage del archivo seleccionado.
    ///
    /// Si está staged → unstage. Si está unstaged → stage.
    /// Refresca el status después de la operación.
    pub fn stage_toggle(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        let Some(file) = self.files.get(self.selected_index) else {
            return Ok(());
        };

        // CLONE: necesario — path se usa después de &mut self vía refresh
        let file_path = file.path.clone();
        let is_staged = file.staged;

        if is_staged {
            commands::unstage_file(repo_path, &file_path)?;
        } else {
            commands::stage_file(repo_path, &file_path)?;
        }

        self.refresh(repo_path);
        Ok(())
    }

    /// Stage o unstage el archivo en la posición `idx` del `Vec<GitFileStatus>`.
    ///
    /// Toggle simétrico a `stage_toggle` pero direccionado por índice — se usa
    /// para click en el botón `[+]`/`[-]` de una row específica del panel.
    /// Refresca el status después de la operación.
    pub fn stage_file_at(&mut self, repo_path: &Path, idx: usize) -> anyhow::Result<()> {
        let file = self
            .files
            .get(idx)
            .ok_or_else(|| anyhow::anyhow!("git: índice {} fuera de rango", idx))?;
        // CLONE: necesario — liberamos el borrow de self.files antes de llamar refresh(&mut self)
        let path = file.path.clone();
        let staged = file.staged;

        if staged {
            commands::unstage_file(repo_path, &path)?;
        } else {
            commands::stage_file(repo_path, &path)?;
        }

        self.refresh(repo_path);
        Ok(())
    }

    /// Descarta o elimina cambios del working tree para el archivo en `idx`.
    ///
    /// - Modified/Deleted unstaged → `git restore -- <file>` (vuelve al index)
    /// - Added/Untracked unstaged  → `git clean -f -- <file>` (elimina del disco)
    /// - Staged → no hace nada (no hay working-tree changes que descartar)
    ///
    /// Refresca el status después de la operación.
    pub fn discard_file_at(&mut self, repo_path: &Path, idx: usize) -> anyhow::Result<()> {
        let file = self
            .files
            .get(idx)
            .ok_or_else(|| anyhow::anyhow!("git: índice {} fuera de rango", idx))?;
        if file.staged {
            return Ok(()); // staged files no tienen working-tree changes que descartar
        }
        // CLONE: necesario — liberamos el borrow de self.files antes de llamar refresh(&mut self)
        let path = file.path.clone();
        let status = file.status;

        match status {
            commands::FileChangeType::Added | commands::FileChangeType::Untracked => {
                // Archivo nuevo — eliminarlo del disco con git clean
                commands::delete_untracked_file(repo_path, &path)?;
            }
            _ => {
                // Modified, Deleted, Renamed, Copied — restaurar desde el index
                commands::discard_file(repo_path, &path)?;
            }
        }

        self.refresh(repo_path);
        Ok(())
    }

    /// Quita todos los archivos del staging area (`git restore --staged .`).
    ///
    /// No descarta cambios del working tree — solo mueve todo de staged a unstaged.
    /// Refresca el status después.
    pub fn unstage_all(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        commands::unstage_all(repo_path)?;
        self.refresh(repo_path);
        Ok(())
    }

    /// Stage de todos los archivos unstaged (equivalente a `git add .`).
    ///
    /// Usa `stage_file(repo_path, ".")` que delega en `git add -- .` —
    /// `run_git` no es pública desde `commands`, pero `stage_file` con
    /// path `"."` produce el mismo efecto y mantiene el patrón consistente.
    /// Refresca el status después.
    pub fn stage_all(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        commands::stage_file(repo_path, ".")?;
        self.refresh(repo_path);
        Ok(())
    }

    /// Carga el diff del archivo seleccionado.
    ///
    /// Determina si usar diff staged o unstaged según el estado del archivo.
    pub fn load_diff(&mut self, repo_path: &Path) {
        let Some(file) = self.files.get(self.selected_index) else {
            self.diff_content = None;
            return;
        };

        let staged = file.staged;
        // CLONE: necesario — path se usa después para diff_file
        let file_path = file.path.clone();

        match commands::diff_file(repo_path, &file_path, staged) {
            Ok(diff) if !diff.trim().is_empty() => {
                self.diff_content = Some(diff);
                self.showing_file_content = false;
            }
            // Diff vacío → archivo nuevo o sin cambios — mostrar contenido del archivo
            Ok(_) => {
                let full_path = repo_path.join(&file_path);
                match std::fs::read_to_string(&full_path) {
                    Ok(content) if !content.is_empty() => {
                        self.diff_content = Some(content);
                        self.showing_file_content = true;
                    }
                    Ok(_) => {
                        self.diff_content = Some(String::from("(archivo vacío)"));
                        self.showing_file_content = true;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = %file_path, "no se pudo leer el archivo");
                        self.diff_content = Some(String::from("(no se pudo leer el archivo)"));
                        self.showing_file_content = true;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %file_path, "no se pudo obtener diff");
                self.diff_content = Some(String::from("(error al obtener diff)"));
                self.showing_file_content = false;
            }
        }
        self.diff_scroll = 0;
    }

    /// Toggle mostrar/ocultar diff del archivo seleccionado.
    ///
    /// LEGACY: el overlay global de diff fue reemplazado por tabs virtuales.
    /// Se conserva para no romper código que aún lo invoque hasta el cleanup.
    #[expect(
        dead_code,
        reason = "legacy del overlay de diff — reemplazado por tabs virtuales (diff_view)"
    )]
    pub fn toggle_diff(&mut self, repo_path: &Path) {
        self.show_diff = !self.show_diff;
        if self.show_diff {
            self.load_diff(repo_path);
        } else {
            self.diff_content = None;
            self.diff_scroll = 0;
            self.showing_file_content = false;
        }
    }

    /// Ejecutar commit con el mensaje actual.
    ///
    /// Limpia el input de commit y refresca el status.
    pub fn commit(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        let message = self.commit_input.trim();
        if message.is_empty() {
            anyhow::bail!("mensaje de commit vacío");
        }

        commands::commit(repo_path, message)?;
        self.commit_input.clear();
        self.commit_mode = false;
        self.refresh(repo_path);
        Ok(())
    }

    /// Scrollear diff hacia arriba.
    pub fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(3);
    }

    /// Scrollear diff hacia abajo.
    pub fn scroll_diff_down(&mut self) {
        let max = self
            .diff_content
            .as_ref()
            .map(|d| d.lines().count().saturating_sub(1))
            .unwrap_or(0);
        self.diff_scroll = (self.diff_scroll + 3).min(max);
    }

    /// Asegurar que la selección está visible en el viewport.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }
}
