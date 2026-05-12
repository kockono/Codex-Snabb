//! Unicode helpers — char-boundary navigation sobre `&str`.
//!
//! `Position.col` se mantiene como **byte offset** (ver `cursor.rs:21`).
//! Estos helpers convierten "moverse 1 char" en "avanzar/retroceder
//! N bytes hasta el próximo char boundary", evitando los panics que
//! ocurren al hacer slicing en medio de un char multi-byte UTF-8.
//!
//! Diseño:
//! - Sin alocaciones — todas operan sobre `&str`.
//! - `#[inline]` para que sean cero-costo en hot paths (cursor movement).
//! - No usan `chars().nth(N)` — eso es O(N). Usan slicing + `.next()`.
//!
//! Invariante de uso: el `byte_idx` de entrada DEBE ser un char boundary
//! válido o el extremo `s.len()`. Si no lo es, `prev_char_boundary` y
//! `next_char_boundary` aún funcionan (caminan hasta el próximo boundary),
//! pero `char_len_at` retorna 0 — es responsabilidad del caller.

/// Avanza al próximo char boundary igual o posterior a `byte_idx`.
///
/// Si `byte_idx >= s.len()`, retorna `s.len()`.
/// Si `byte_idx` está EN un boundary, avanza al SIGUIENTE char (no se queda).
/// Si `byte_idx` está en medio de un char multi-byte, camina hasta el
/// próximo boundary válido (esto NO debería ocurrir si el invariante de
/// `Position.col` se respeta — pero el helper es defensivo).
///
/// Nota: Actualmente `move_right` y similares usan `char_len_at` (más directo
/// para el caso "avanzar 1 char desde un boundary conocido"). Esta función
/// se mantiene como helper general para casos donde `byte_idx` puede no estar
/// en boundary (recuperación defensiva).
#[allow(dead_code, reason = "helper público — caso de uso defensivo, futuro")]
#[inline]
pub fn next_char_boundary(s: &str, byte_idx: usize) -> usize {
    if byte_idx >= s.len() {
        return s.len();
    }
    // Si está en boundary, avanzar por el largo del char en esa posición.
    let bytes = s.as_bytes();
    let mut i = byte_idx;
    // Si NO está en boundary, caminar hasta uno.
    if !s.is_char_boundary(i) {
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        return i;
    }
    // Está en boundary — avanzar por el largo del char ahí.
    // Las reglas UTF-8: byte líder ASCII (0xxxxxxx) → 1 byte;
    // 110xxxxx → 2; 1110xxxx → 3; 11110xxx → 4.
    let lead = bytes[i];
    let len = if lead < 0x80 {
        1
    } else if lead < 0xC0 {
        // continuation byte — no debería pasar si i es boundary; defensivo
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    };
    i += len;
    if i > s.len() { s.len() } else { i }
}

/// Retrocede al char boundary inmediatamente ANTERIOR a `byte_idx`.
///
/// Si `byte_idx == 0`, retorna 0.
/// Si `byte_idx > s.len()`, primero clampea a `s.len()` y luego retrocede.
/// Si `byte_idx` está en medio de un char (no debería ocurrir), retrocede
/// hasta el primer boundary válido — defensivo.
#[inline]
pub fn prev_char_boundary(s: &str, byte_idx: usize) -> usize {
    if byte_idx == 0 {
        return 0;
    }
    let mut i = byte_idx.min(s.len());
    // Retroceder al menos 1 byte y luego seguir hasta encontrar boundary.
    i -= 1;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Retorna el largo en bytes UTF-8 del char que empieza en `byte_idx`.
///
/// Retorna 0 si `byte_idx >= s.len()` o si el byte ahí no es un líder
/// válido (caso defensivo — caller violó el invariante de boundary).
#[inline]
pub fn char_len_at(s: &str, byte_idx: usize) -> usize {
    if byte_idx >= s.len() || !s.is_char_boundary(byte_idx) {
        return 0;
    }
    s[byte_idx..].chars().next().map_or(0, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── next_char_boundary ──

    #[test]
    fn next_boundary_ascii_advances_one_byte() {
        let s = "hello";
        assert_eq!(next_char_boundary(s, 0), 1);
        assert_eq!(next_char_boundary(s, 1), 2);
        assert_eq!(next_char_boundary(s, 4), 5);
    }

    #[test]
    fn next_boundary_at_end_returns_len() {
        let s = "hello";
        assert_eq!(next_char_boundary(s, 5), 5);
        assert_eq!(next_char_boundary(s, 999), 5);
    }

    #[test]
    fn next_boundary_two_byte_char_advances_two() {
        // "ó" = 2 bytes (0xC3 0xB3), "código" = c-ó-d-i-g-o = 1+2+1+1+1+1 = 7 bytes
        let s = "código";
        assert_eq!(s.len(), 7);
        assert_eq!(next_char_boundary(s, 0), 1); // c → ó
        assert_eq!(next_char_boundary(s, 1), 3); // ó → d (saltó 2 bytes)
        assert_eq!(next_char_boundary(s, 3), 4); // d → i
        assert_eq!(next_char_boundary(s, 6), 7); // o → end
    }

    #[test]
    fn next_boundary_four_byte_emoji() {
        // 😀 = U+1F600 = 4 bytes (0xF0 0x9F 0x98 0x80)
        let s = "😀hi";
        assert_eq!(s.len(), 6);
        assert_eq!(next_char_boundary(s, 0), 4); // 😀 → h
        assert_eq!(next_char_boundary(s, 4), 5); // h → i
    }

    #[test]
    fn next_boundary_three_byte_cjk() {
        // "中" = 3 bytes (0xE4 0xB8 0xAD)
        let s = "中文";
        assert_eq!(s.len(), 6);
        assert_eq!(next_char_boundary(s, 0), 3);
        assert_eq!(next_char_boundary(s, 3), 6);
    }

    #[test]
    fn next_boundary_empty_string() {
        let s = "";
        assert_eq!(next_char_boundary(s, 0), 0);
        assert_eq!(next_char_boundary(s, 5), 0);
    }

    // ── prev_char_boundary ──

    #[test]
    fn prev_boundary_ascii_retreats_one_byte() {
        let s = "hello";
        assert_eq!(prev_char_boundary(s, 5), 4);
        assert_eq!(prev_char_boundary(s, 1), 0);
    }

    #[test]
    fn prev_boundary_at_zero_returns_zero() {
        let s = "hello";
        assert_eq!(prev_char_boundary(s, 0), 0);
    }

    #[test]
    fn prev_boundary_two_byte_char() {
        // "código" — para retroceder desde col 3 (después de ó) → col 1 (antes de ó)
        let s = "código";
        assert_eq!(prev_char_boundary(s, 3), 1);
        assert_eq!(prev_char_boundary(s, 1), 0); // c → start
    }

    #[test]
    fn prev_boundary_four_byte_emoji() {
        let s = "😀hi";
        assert_eq!(prev_char_boundary(s, 4), 0); // antes de h, retroceder al inicio (saltar 4 bytes del emoji)
        assert_eq!(prev_char_boundary(s, 5), 4); // antes de i → después del emoji
    }

    #[test]
    fn prev_boundary_three_byte_cjk() {
        let s = "中文";
        assert_eq!(prev_char_boundary(s, 6), 3);
        assert_eq!(prev_char_boundary(s, 3), 0);
    }

    #[test]
    fn prev_boundary_beyond_end_clamps() {
        let s = "ab";
        assert_eq!(prev_char_boundary(s, 999), 1);
    }

    // ── char_len_at ──

    #[test]
    fn char_len_ascii_is_one() {
        assert_eq!(char_len_at("hello", 0), 1);
        assert_eq!(char_len_at("hello", 4), 1);
    }

    #[test]
    fn char_len_two_byte() {
        assert_eq!(char_len_at("código", 1), 2); // ó
    }

    #[test]
    fn char_len_three_byte_cjk() {
        assert_eq!(char_len_at("中文", 0), 3);
        assert_eq!(char_len_at("中文", 3), 3);
    }

    #[test]
    fn char_len_four_byte_emoji() {
        assert_eq!(char_len_at("😀hi", 0), 4);
        assert_eq!(char_len_at("😀hi", 4), 1); // h
    }

    #[test]
    fn char_len_at_end_is_zero() {
        assert_eq!(char_len_at("hi", 2), 0);
        assert_eq!(char_len_at("", 0), 0);
    }

    #[test]
    fn char_len_in_middle_of_multibyte_is_zero() {
        // Defensivo: si col cae en medio de "ó" (col=2, dentro de "código")
        // — char_len_at retorna 0 porque no está en boundary.
        // Esto es para detectar invariante violada en debug.
        // "código" — byte 2 = 0xB3 (continuation de ó)
        let s = "código";
        assert_eq!(char_len_at(s, 2), 0);
    }
}
