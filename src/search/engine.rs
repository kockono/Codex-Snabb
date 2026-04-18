//! Motor de búsqueda global en workspace.
//!
//! Recorre recursivamente el workspace, respeta ignore patterns
//! (`.git`, `target`, `node_modules`), aplica filtros (case, whole word,
//! regex, include/exclude globs) y retorna resultados acotados.
//!
//! La búsqueda se ejecuta FUERA del render loop. Los resultados se cachean
//! en `SearchState` y se consultan por referencia durante render.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Directorios siempre excluidos — sin importar patrones del usuario.
const ALWAYS_IGNORED: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".hg",
    ".svn",
    "__pycache__",
];

/// Opciones de búsqueda configurables por el usuario.
///
/// Los campos `String` son propiedad del `SearchState` — acá se toman por
/// referencia desde `SearchState::options` via `&SearchOptions`.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Texto o patrón a buscar.
    pub query: String,
    /// Si la búsqueda distingue mayúsculas/minúsculas.
    pub case_sensitive: bool,
    /// Si el match debe ser una palabra completa.
    pub whole_word: bool,
    /// Si `query` se interpreta como regex.
    pub use_regex: bool,
    /// Glob para incluir archivos (ej: "*.rs,*.toml"). Vacío = todo.
    pub include_pattern: String,
    /// Glob para excluir archivos (ej: "target/**,node_modules/**"). Vacío = nada extra.
    pub exclude_pattern: String,
}

/// Un match individual dentro de un archivo.
#[derive(Debug, Clone)]
pub struct SearchMatch {
    /// Path relativo al workspace root.
    pub path: PathBuf,
    /// Número de línea (1-indexed).
    pub line_number: usize,
    /// Contenido de la línea (truncado a 500 chars para no explotar RAM).
    pub line_content: String,
    /// Byte offset del inicio del match dentro de la línea.
    pub match_start: usize,
    /// Byte offset del fin del match dentro de la línea.
    pub match_end: usize,
}

/// Resultados agregados de una búsqueda global.
#[derive(Debug)]
pub struct SearchResults {
    /// Todos los matches encontrados (hasta `max_results`).
    pub matches: Vec<SearchMatch>,
    /// Cantidad de archivos recorridos.
    pub files_searched: usize,
    /// Cantidad de archivos con al menos un match.
    pub files_matched: usize,
    /// Total de matches (puede ser == matches.len() si no truncó).
    pub total_matches: usize,
    /// Si se cortó por alcanzar `max_results`.
    pub truncated: bool,
}

/// Ejecuta búsqueda global en el workspace.
///
/// Recorre `root` recursivamente, respetando ignore y filtros.
/// Corta en `max_results` y marca `truncated`.
///
/// # Errors
/// - Regex inválida si `use_regex` está activo
/// - Glob pattern inválido en include/exclude
pub fn search_workspace(
    root: &Path,
    options: &SearchOptions,
    max_results: usize,
) -> anyhow::Result<SearchResults> {
    if options.query.is_empty() {
        return Ok(SearchResults {
            matches: Vec::new(),
            files_searched: 0,
            files_matched: 0,
            total_matches: 0,
            truncated: false,
        });
    }

    // Compilar regex si corresponde
    let compiled_regex = if options.use_regex {
        let pattern = if options.case_sensitive {
            options.query.as_str().to_string()
        } else {
            // Prepend case-insensitive flag
            format!("(?i){}", options.query)
        };
        Some(
            regex::Regex::new(&pattern)
                .with_context(|| format!("regex inválida: '{}'", options.query))?,
        )
    } else {
        None
    };

    // Compilar globs de include/exclude
    let include_matcher =
        build_glob_set(&options.include_pattern).context("glob de include inválido")?;
    let exclude_matcher =
        build_glob_set(&options.exclude_pattern).context("glob de exclude inválido")?;

    // Query en lowercase para búsqueda case-insensitive (solo si no regex)
    let query_lower = if !options.case_sensitive && !options.use_regex {
        options.query.to_lowercase()
    } else {
        String::new()
    };

    let mut results = SearchResults {
        matches: Vec::with_capacity(max_results.min(256)),
        files_searched: 0,
        files_matched: 0,
        total_matches: 0,
        truncated: false,
    };

    // Recolectar archivos a buscar
    let mut file_stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut files_to_search: Vec<PathBuf> = Vec::with_capacity(256);

    while let Some(dir) = file_stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();

            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            // Ignorar directorios/archivos siempre excluidos
            if ALWAYS_IGNORED
                .iter()
                .any(|&ignored| name.as_ref() == ignored)
            {
                continue;
            }

            if path.is_dir() {
                // Verificar exclude en directorio
                if let Some(ref matcher) = exclude_matcher {
                    let relative = path.strip_prefix(root).unwrap_or(&path);
                    if matcher.is_match(relative) {
                        continue;
                    }
                }
                file_stack.push(path);
            } else if path.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(&path);

                // Filtro include: si hay patrón, el archivo debe matchear
                if let Some(ref matcher) = include_matcher
                    && !matcher.is_match(relative)
                {
                    continue;
                }
                // Filtro exclude: si hay patrón, el archivo no debe matchear
                if let Some(ref matcher) = exclude_matcher
                    && matcher.is_match(relative)
                {
                    continue;
                }

                files_to_search.push(path);
            }
        }
    }

    // Buffer reutilizable para lectura línea por línea
    let mut line_buf = String::with_capacity(512);

    for file_path in &files_to_search {
        if results.truncated {
            break;
        }

        let file = match fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        results.files_searched += 1;
        let mut reader = BufReader::with_capacity(8192, file);
        let mut line_number: usize = 0;
        let mut file_has_match = false;

        loop {
            line_buf.clear();
            let bytes_read = match reader.read_line(&mut line_buf) {
                Ok(n) => n,
                Err(_) => break, // Archivo binario o encoding raro — skip
            };

            if bytes_read == 0 {
                break; // EOF
            }

            // Detectar binario: si hay null bytes, skip el archivo entero
            if line_number == 0 && line_buf.contains('\0') {
                break;
            }

            line_number += 1;
            let line = line_buf.trim_end_matches('\n').trim_end_matches('\r');

            // Buscar matches en la línea
            let line_matches = find_matches_in_line(
                line,
                &options.query,
                &query_lower,
                options.case_sensitive,
                options.whole_word,
                compiled_regex.as_ref(),
            );

            for (start, end) in line_matches {
                if !file_has_match {
                    file_has_match = true;
                    results.files_matched += 1;
                }
                results.total_matches += 1;

                if results.matches.len() >= max_results {
                    results.truncated = true;
                    break;
                }

                let relative = file_path.strip_prefix(root).unwrap_or(file_path);
                // Truncar línea para display — no guardar líneas enormes
                let display_line = if line.len() > 500 { &line[..500] } else { line };

                results.matches.push(SearchMatch {
                    path: relative.to_path_buf(),
                    line_number,
                    line_content: display_line.to_string(),
                    match_start: start,
                    match_end: end,
                });
            }

            if results.truncated {
                break;
            }
        }
    }

    Ok(results)
}

/// Encuentra posiciones de match en una línea.
///
/// Retorna vector de (start_byte, end_byte) para cada match.
/// Reutiliza el vector interno — en el futuro se podría pasar por &mut.
fn find_matches_in_line(
    line: &str,
    query: &str,
    query_lower: &str,
    case_sensitive: bool,
    whole_word: bool,
    compiled_regex: Option<&regex::Regex>,
) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();

    if let Some(re) = compiled_regex {
        // Modo regex
        for m in re.find_iter(line) {
            let (start, end) = (m.start(), m.end());
            if whole_word && !is_word_boundary(line, start, end) {
                continue;
            }
            matches.push((start, end));
        }
    } else if case_sensitive {
        // Búsqueda literal case-sensitive
        let mut search_from = 0;
        while let Some(pos) = line[search_from..].find(query) {
            let start = search_from + pos;
            let end = start + query.len();
            if !whole_word || is_word_boundary(line, start, end) {
                matches.push((start, end));
            }
            search_from = end;
        }
    } else {
        // Búsqueda literal case-insensitive
        let line_lower = line.to_lowercase();
        let mut search_from = 0;
        while let Some(pos) = line_lower[search_from..].find(query_lower) {
            let start = search_from + pos;
            let end = start + query_lower.len();
            if !whole_word || is_word_boundary(line, start, end) {
                matches.push((start, end));
            }
            search_from = end;
        }
    }

    matches
}

/// Verifica si el match en (start, end) está rodeado de word boundaries.
///
/// Word boundary = inicio/fin de string, o carácter no alfanumérico/_
fn is_word_boundary(line: &str, start: usize, end: usize) -> bool {
    let bytes = line.as_bytes();

    // Check antes del match
    let before_ok = start == 0 || !is_word_char(bytes[start - 1]);
    // Check después del match
    let after_ok = end >= bytes.len() || !is_word_char(bytes[end]);

    before_ok && after_ok
}

/// Determina si un byte es un "word character" (alphanumeric o underscore).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Construye un `GlobSet` desde un string de patrones separados por comas.
///
/// Retorna `None` si el string está vacío (no hay filtro).
fn build_glob_set(pattern_str: &str) -> anyhow::Result<Option<globset::GlobSet>> {
    let trimmed = pattern_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut builder = globset::GlobSetBuilder::new();
    for pat in trimmed.split(',') {
        let pat = pat.trim();
        if !pat.is_empty() {
            let glob = globset::GlobBuilder::new(pat)
                .literal_separator(false)
                .build()
                .with_context(|| format!("glob pattern inválido: '{pat}'"))?;
            builder.add(glob);
        }
    }

    let set = builder.build().context("no se pudo construir GlobSet")?;
    Ok(Some(set))
}

/// Reemplaza un match específico en un archivo en disco.
///
/// Lee el archivo, reemplaza la ocurrencia en la línea indicada,
/// y reescribe el archivo. Operación sincrónica (se llama desde reduce,
/// no desde render).
///
/// # Errors
/// - Si el archivo no existe o no se puede leer/escribir
/// - Si la línea no existe o el match no se encuentra
pub fn replace_in_file(
    file_path: &Path,
    line_number: usize,
    match_start: usize,
    match_end: usize,
    replacement: &str,
) -> anyhow::Result<()> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("no se pudo leer: {}", file_path.display()))?;

    let lines: Vec<&str> = content.lines().collect();
    let line_idx = line_number
        .checked_sub(1)
        .context("line_number debe ser >= 1")?;

    let line = lines.get(line_idx).context("línea fuera de rango")?;

    if match_end > line.len() || match_start > match_end {
        anyhow::bail!(
            "offsets de match inválidos: {}..{} en línea de {} bytes",
            match_start,
            match_end,
            line.len()
        );
    }

    // Construir nueva línea con el reemplazo
    let mut new_line = String::with_capacity(line.len() + replacement.len());
    new_line.push_str(&line[..match_start]);
    new_line.push_str(replacement);
    new_line.push_str(&line[match_end..]);

    // Reemplazar en la colección. Necesitamos un owned Vec<String>.
    let mut owned_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    owned_lines[line_idx] = new_line;

    // Reconstruir archivo preservando newlines originales
    let new_content = if content.ends_with('\n') {
        let mut s = owned_lines.join("\n");
        s.push('\n');
        s
    } else {
        owned_lines.join("\n")
    };

    fs::write(file_path, &new_content)
        .with_context(|| format!("no se pudo escribir: {}", file_path.display()))?;

    Ok(())
}
