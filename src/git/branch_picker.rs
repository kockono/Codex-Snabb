//! Branch Picker: estado del overlay de selección de rama.
//!
//! Overlay modal para listar, filtrar y hacer checkout de ramas git.
//! Similar al QuickOpen pero específico para ramas.
//! El filtrado se hace en `update_filter()` — NUNCA en render.

use std::path::Path;

use crate::git::commands::{self, BranchInfo};

// ─── BranchPicker ──────────────────────────────────────────────────────────────

/// Estado del branch picker (overlay de selección de rama).
///
/// Mantiene la lista de ramas, el filtro de búsqueda, y la selección actual.
/// El filtrado se cachea en `filtered` como índices a `branches`.
#[derive(Debug)]
pub struct BranchPicker {
    /// Si el picker está visible.
    pub visible: bool,
    /// Texto de búsqueda del usuario.
    pub input: String,
    /// Posición del cursor dentro del input.
    pub cursor_pos: usize,
    /// Lista completa de ramas del repo.
    pub branches: Vec<BranchInfo>,
    /// Índices filtrados en `branches` según `input`.
    pub filtered: Vec<usize>,
    /// Índice de la selección dentro de `filtered`.
    pub selected_index: usize,
    /// Offset de scroll para listas largas.
    pub scroll_offset: usize,
}

/// Máximo de items visibles en la lista del branch picker.
pub const MAX_VISIBLE_ITEMS: usize = 15;

impl BranchPicker {
    /// Crea un nuevo estado (picker cerrado).
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::with_capacity(64),
            cursor_pos: 0,
            branches: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
        }
    }

    /// Abre el picker: carga ramas del repo, muestra todas.
    ///
    /// Si git no está disponible o no es repo, retorna error gracefully.
    pub fn open(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        self.branches = commands::list_branches(repo_path)?;
        self.visible = true;
        self.input.clear();
        self.cursor_pos = 0;
        self.selected_index = 0;
        self.scroll_offset = 0;

        // Inicialmente mostrar todas las ramas
        self.filtered = (0..self.branches.len()).collect();

        Ok(())
    }

    /// Cierra el picker y limpia estado.
    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.cursor_pos = 0;
        self.branches.clear();
        self.filtered.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Actualiza la lista filtrada según `input` (case-insensitive, substring).
    ///
    /// El filtrado se hace acá, no en render. Reusa la capacidad del Vec.
    pub fn update_filter(&mut self) {
        self.filtered.clear();

        if self.input.is_empty() {
            // Sin filtro → mostrar todas
            self.filtered.extend(0..self.branches.len());
        } else {
            let query = self.input.to_lowercase();
            for (i, branch) in self.branches.iter().enumerate() {
                if branch.name.to_lowercase().contains(&query) {
                    self.filtered.push(i);
                }
            }
        }

        // Reset selección si se sale del rango
        if self.selected_index >= self.filtered.len() {
            self.selected_index = 0;
        }
        self.scroll_offset = 0;
    }

    /// Mover selección una posición arriba.
    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                // Wrap al final
                self.selected_index = self.filtered.len() - 1;
            }
            self.ensure_visible();
        }
    }

    /// Mover selección una posición abajo.
    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index + 1 < self.filtered.len() {
                self.selected_index += 1;
            } else {
                // Wrap al inicio
                self.selected_index = 0;
            }
            self.ensure_visible();
        }
    }

    /// Inserta un carácter en el input y re-filtra.
    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
        self.update_filter();
    }

    /// Elimina el carácter antes del cursor y re-filtra.
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            // Encontrar el boundary del char anterior
            let prev_boundary = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.input.drain(prev_boundary..self.cursor_pos);
            self.cursor_pos = prev_boundary;
            self.update_filter();
        }
    }

    /// Retorna la rama actualmente seleccionada (si hay).
    pub fn selected_branch(&self) -> Option<&BranchInfo> {
        let &branch_idx = self.filtered.get(self.selected_index)?;
        self.branches.get(branch_idx)
    }

    /// Hace checkout de la rama seleccionada y cierra el picker.
    ///
    /// Retorna error si el checkout falla. El caller debe refrescar
    /// el git state después.
    pub fn checkout_selected(&mut self, repo_path: &Path) -> anyhow::Result<()> {
        let branch_name = self
            .selected_branch()
            .map(|b| b.name.as_str())
            .unwrap_or("");

        if branch_name.is_empty() {
            anyhow::bail!("no hay rama seleccionada");
        }

        // CLONE: necesario — branch_name es referencia a self.branches,
        // y close() va a limpiar self.branches
        let name = branch_name.to_string();

        commands::checkout_branch(repo_path, &name)?;
        self.close();

        Ok(())
    }

    /// Ajusta el scroll para que la selección sea visible.
    fn ensure_visible(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + MAX_VISIBLE_ITEMS {
            self.scroll_offset = self.selected_index - MAX_VISIBLE_ITEMS + 1;
        }
    }
}
