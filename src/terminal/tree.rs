//! Tree: layout recursivo de panes de terminal.
//!
//! `PaneTree` es un árbol binario donde las hojas son `PaneId`
//! y los nodos internos son splits (horizontal o vertical) con un ratio.
//! Profundidad máxima práctica: ~4 (límite de usabilidad humana).
//!
//! El diseño prioriza cero allocations en hot paths:
//! - `collect_rects` usa un buffer externo pre-alocado
//! - No hay `Box<dyn>` ni trait objects

use ratatui::layout::Rect;

/// Identificador de pane de terminal. Simple `u32` — barato de copiar.
pub type PaneId = u32;

/// Orientación de un split entre dos panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Split lado a lado (divide el ancho).
    Horizontal,
    /// Split arriba/abajo (divide la altura).
    Vertical,
}

/// Árbol binario recursivo de panes de terminal.
///
/// Cada hoja es un `PaneId`. Cada nodo interno divide el espacio
/// entre dos sub-árboles con una orientación y un ratio.
#[derive(Debug)]
pub enum PaneTree {
    /// Pane individual.
    Leaf(PaneId),
    /// División en dos sub-panes.
    Split {
        orientation: Orientation,
        /// Fracción del espacio para `first` (0.0..1.0).
        ratio: f32,
        first: Box<PaneTree>,
        second: Box<PaneTree>,
    },
}

impl PaneTree {
    /// Retorna el `PaneId` de la primera hoja en orden depth-first.
    pub fn first_leaf(&self) -> PaneId {
        match self {
            PaneTree::Leaf(id) => *id,
            PaneTree::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Computa el `Rect` para un `PaneId` dado dividiendo `area` recursivamente.
    #[allow(dead_code)] // se usará en batch 4 para render per-pane
    pub fn rect_for(&self, id: PaneId, area: Rect) -> Option<Rect> {
        match self {
            PaneTree::Leaf(leaf_id) => {
                if *leaf_id == id {
                    Some(area)
                } else {
                    None
                }
            }
            PaneTree::Split {
                orientation,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = split_rect(area, *orientation, *ratio);
                first
                    .rect_for(id, first_rect)
                    .or_else(|| second.rect_for(id, second_rect))
            }
        }
    }

    /// Recolecta todos los `(PaneId, Rect)` en el buffer de salida.
    ///
    /// Usa un buffer externo para evitar allocations repetidas —
    /// el caller pre-aloca con `Vec::with_capacity(panes.len())`.
    pub fn collect_rects(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            PaneTree::Leaf(id) => out.push((*id, area)),
            PaneTree::Split {
                orientation,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = split_rect(area, *orientation, *ratio);
                first.collect_rects(first_rect, out);
                second.collect_rects(second_rect, out);
            }
        }
    }

    /// Encuentra el siguiente `PaneId` en orden depth-first después de `current`.
    /// Cicla al primero si `current` es el último.
    pub fn next_after(&self, current: PaneId) -> Option<PaneId> {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        let pos = ids.iter().position(|&id| id == current)?;
        Some(ids[(pos + 1) % ids.len()])
    }

    /// Recolecta todos los PaneId en orden depth-first.
    pub(crate) fn collect_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            PaneTree::Leaf(id) => out.push(*id),
            PaneTree::Split { first, second, .. } => {
                first.collect_ids(out);
                second.collect_ids(out);
            }
        }
    }

    /// Divide la hoja con `target_id` en dos panes.
    ///
    /// El pane existente queda como `first`, el nuevo como `second`.
    /// Retorna `false` si `target_id` no se encontró.
    pub fn split_leaf(
        &mut self,
        target_id: PaneId,
        orientation: Orientation,
        new_id: PaneId,
    ) -> bool {
        match self {
            PaneTree::Leaf(id) if *id == target_id => {
                let old_id = *id;
                *self = PaneTree::Split {
                    orientation,
                    ratio: 0.5,
                    first: Box::new(PaneTree::Leaf(old_id)),
                    second: Box::new(PaneTree::Leaf(new_id)),
                };
                true
            }
            PaneTree::Leaf(_) => false,
            PaneTree::Split { first, second, .. } => {
                first.split_leaf(target_id, orientation, new_id)
                    || second.split_leaf(target_id, orientation, new_id)
            }
        }
    }

    /// Elimina la hoja con `target_id`.
    ///
    /// Si el parent split queda con un solo hijo, se reemplaza por ese hijo
    /// (colapso del nodo intermedio).
    pub fn remove_leaf(&mut self, target_id: PaneId) -> bool {
        match self {
            PaneTree::Leaf(_) => false,
            PaneTree::Split { first, second, .. } => {
                // Check if first child is the target leaf
                if let PaneTree::Leaf(id) = first.as_ref() {
                    if *id == target_id {
                        // Replace self with second (colapsar split)
                        let second_owned = std::mem::replace(
                            second.as_mut(),
                            PaneTree::Leaf(0), // placeholder temporal
                        );
                        *self = second_owned;
                        return true;
                    }
                }
                // Check if second child is the target leaf
                if let PaneTree::Leaf(id) = second.as_ref() {
                    if *id == target_id {
                        let first_owned = std::mem::replace(
                            first.as_mut(),
                            PaneTree::Leaf(0), // placeholder temporal
                        );
                        *self = first_owned;
                        return true;
                    }
                }
                // Recurse into children
                first.remove_leaf(target_id) || second.remove_leaf(target_id)
            }
        }
    }
}

/// Divide un `Rect` en dos sub-rects según orientación y ratio.
fn split_rect(area: Rect, orientation: Orientation, ratio: f32) -> (Rect, Rect) {
    match orientation {
        Orientation::Horizontal => {
            let first_width = ((area.width as f32) * ratio).round() as u16;
            let second_width = area.width.saturating_sub(first_width);
            let first = Rect::new(area.x, area.y, first_width, area.height);
            let second = Rect::new(area.x + first_width, area.y, second_width, area.height);
            (first, second)
        }
        Orientation::Vertical => {
            let first_height = ((area.height as f32) * ratio).round() as u16;
            let second_height = area.height.saturating_sub(first_height);
            let first = Rect::new(area.x, area.y, area.width, first_height);
            let second = Rect::new(area.x, area.y + first_height, area.width, second_height);
            (first, second)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn test_leaf_rect() {
        let tree = PaneTree::Leaf(1);
        let area = rect(0, 0, 100, 40);
        assert_eq!(tree.rect_for(1, area), Some(area));
        assert_eq!(tree.rect_for(2, area), None);
    }

    #[test]
    fn test_horizontal_split_rects() {
        let tree = PaneTree::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneTree::Leaf(1)),
            second: Box::new(PaneTree::Leaf(2)),
        };
        let area = rect(0, 0, 100, 40);
        let r1 = tree.rect_for(1, area).unwrap();
        let r2 = tree.rect_for(2, area).unwrap();
        assert_eq!(r1.width + r2.width, 100);
        assert_eq!(r1.x, 0);
        assert_eq!(r2.x, r1.width);
        assert_eq!(r1.height, 40);
        assert_eq!(r2.height, 40);
    }

    #[test]
    fn test_vertical_split_rects() {
        let tree = PaneTree::Split {
            orientation: Orientation::Vertical,
            ratio: 0.5,
            first: Box::new(PaneTree::Leaf(1)),
            second: Box::new(PaneTree::Leaf(2)),
        };
        let area = rect(0, 0, 100, 40);
        let r1 = tree.rect_for(1, area).unwrap();
        let r2 = tree.rect_for(2, area).unwrap();
        assert_eq!(r1.height + r2.height, 40);
        assert_eq!(r1.y, 0);
        assert_eq!(r2.y, r1.height);
    }

    #[test]
    fn test_split_leaf_and_next() {
        let mut tree = PaneTree::Leaf(1);
        assert!(tree.split_leaf(1, Orientation::Horizontal, 2));
        let first = tree.first_leaf();
        assert_eq!(first, 1);
        let next = tree.next_after(1).unwrap();
        assert_eq!(next, 2);
        let next2 = tree.next_after(2).unwrap();
        assert_eq!(next2, 1); // wraps around
    }

    #[test]
    fn test_remove_leaf() {
        let mut tree = PaneTree::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneTree::Leaf(1)),
            second: Box::new(PaneTree::Leaf(2)),
        };
        assert!(tree.remove_leaf(1));
        // tree should now be Leaf(2)
        assert!(matches!(tree, PaneTree::Leaf(2)));
    }

    #[test]
    fn test_collect_rects_two_panes() {
        let tree = PaneTree::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneTree::Leaf(1)),
            second: Box::new(PaneTree::Leaf(2)),
        };
        let area = rect(0, 0, 100, 40);
        let mut rects = Vec::new();
        tree.collect_rects(area, &mut rects);
        assert_eq!(rects.len(), 2);
    }
}
