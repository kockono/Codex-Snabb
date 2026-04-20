//! Indent: detección de nivel de indentación y posiciones de indent guides.
//!
//! Calcula dónde dibujar líneas verticales `│` para visualizar
//! la estructura de indentación del código, estilo VS Code.
//! Funciones puras sin allocaciones — se llaman FUERA del render loop.

/// Calcula el nivel de indentación de una línea en columnas.
///
/// Cuenta espacios y tabs (convertidos a `tab_width` columnas) hasta
/// encontrar el primer carácter no-whitespace. Retorna el número de
/// columnas de indentación.
///
/// Líneas vacías retornan 0 — el contexto circundante determina
/// sus guides (ver `resolve_empty_line_indent`).
pub fn indent_level(line: &str, tab_width: usize) -> usize {
    let mut level = 0;
    for ch in line.chars() {
        match ch {
            ' ' => level += 1,
            '\t' => level += tab_width,
            _ => break,
        }
    }
    level
}

/// Determina las posiciones de columna donde dibujar indent guides.
///
/// Retorna las columnas donde va un `│`. Solo genera posiciones
/// en múltiplos de `tab_width` dentro del rango de indentación.
///
/// Ejemplo: `indent_level=8`, `tab_width=4` → guides en `[0, 4]`.
///
/// Capacidad pre-calculada: máximo `indent / tab_width` posiciones.
pub fn guide_positions(indent: usize, tab_width: usize) -> Vec<usize> {
    if tab_width == 0 || indent == 0 {
        return Vec::new();
    }
    let count = indent / tab_width;
    let mut positions = Vec::with_capacity(count);
    let mut col = 0;
    while col < indent {
        positions.push(col);
        col += tab_width;
    }
    positions
}

/// Resuelve el nivel de indent para una línea vacía usando contexto.
///
/// Las líneas vacías no tienen indentación propia — sus guides se
/// derivan del mínimo entre la línea no-vacía anterior y la siguiente.
/// Esto evita que los guides "desaparezcan" en líneas vacías entre
/// bloques indentados.
///
/// `prev_indent` y `next_indent` son los niveles de las líneas
/// no-vacías más cercanas (o 0 si no hay).
pub fn resolve_empty_line_indent(prev_indent: usize, next_indent: usize) -> usize {
    prev_indent.min(next_indent)
}

/// Pre-computa los niveles de indentación del viewport visible.
///
/// Retorna un `Vec<usize>` con el nivel de indent para cada línea
/// en el rango `[start..start+height]`. Las líneas vacías se resuelven
/// usando el contexto circundante.
///
/// Se llama UNA VEZ antes del render — no en el render loop.
pub fn compute_viewport_indents(lines: &[Option<&str>], tab_width: usize) -> Vec<usize> {
    let len = lines.len();
    let mut indents = Vec::with_capacity(len);

    // Primer pasada: calcular indent raw (líneas vacías = 0)
    for line in lines {
        match line {
            Some(l) if !l.is_empty() => indents.push(indent_level(l, tab_width)),
            _ => indents.push(0),
        }
    }

    // Segunda pasada: resolver líneas vacías con contexto
    // Para cada línea vacía, buscar la no-vacía anterior y siguiente.
    for i in 0..len {
        let is_empty = lines[i].is_none_or(|l| l.is_empty());
        if !is_empty {
            continue;
        }

        // Buscar indent anterior no-vacío
        let prev = (0..i)
            .rev()
            .find(|&j| lines[j].is_some_and(|l| !l.is_empty()))
            .map(|j| indents[j])
            .unwrap_or(0);

        // Buscar indent siguiente no-vacío
        let next = ((i + 1)..len)
            .find(|&j| lines[j].is_some_and(|l| !l.is_empty()))
            .map(|j| indents[j])
            .unwrap_or(0);

        indents[i] = resolve_empty_line_indent(prev, next);
    }

    indents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indent_level_spaces() {
        assert_eq!(indent_level("    hello", 4), 4);
        assert_eq!(indent_level("        world", 4), 8);
        assert_eq!(indent_level("hello", 4), 0);
        assert_eq!(indent_level("", 4), 0);
    }

    #[test]
    fn test_indent_level_tabs() {
        assert_eq!(indent_level("\thello", 4), 4);
        assert_eq!(indent_level("\t\tworld", 4), 8);
    }

    #[test]
    fn test_indent_level_mixed() {
        assert_eq!(indent_level("\t  hello", 4), 6);
    }

    #[test]
    fn test_guide_positions() {
        assert_eq!(guide_positions(8, 4), vec![0, 4]);
        assert_eq!(guide_positions(4, 4), vec![0]);
        assert_eq!(guide_positions(0, 4), Vec::<usize>::new());
        // 3 espacios de indent con tab_width=4: guide en col 0
        // (hay espacio de indentación desde col 0 hasta col 3)
        assert_eq!(guide_positions(3, 4), vec![0]);
        assert_eq!(guide_positions(12, 4), vec![0, 4, 8]);
    }

    #[test]
    fn test_guide_positions_zero_tab_width() {
        assert_eq!(guide_positions(8, 0), Vec::<usize>::new());
    }

    #[test]
    fn test_resolve_empty_line() {
        assert_eq!(resolve_empty_line_indent(8, 4), 4);
        assert_eq!(resolve_empty_line_indent(4, 8), 4);
        assert_eq!(resolve_empty_line_indent(0, 0), 0);
    }

    #[test]
    fn test_compute_viewport_indents() {
        let lines: Vec<Option<&str>> = vec![
            Some("fn main() {"),
            Some("    let x = 5;"),
            Some(""), // vacía — debería heredar min(4, 4) = 4
            Some("    if x > 3 {"),
            Some("        println!(\"hello\");"),
            Some("    }"),
            Some("}"),
        ];
        let indents = compute_viewport_indents(&lines, 4);
        assert_eq!(indents, vec![0, 4, 4, 4, 8, 4, 0]);
    }
}
