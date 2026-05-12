//! Input: conversión pura de KeyEvent a bytes ANSI para PTY.
//!
//! Función pura sin estado que mapea `crossterm::event::KeyEvent` a la
//! secuencia de bytes correspondiente que se debe enviar al PTY.
//! Usa `SmallVec<[u8; 8]>` para evitar heap allocation en la mayoría de
//! los casos (las secuencias más largas son ~6 bytes).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use smallvec::SmallVec;

/// Convierte un `KeyEvent` de crossterm a la secuencia de bytes para enviar al PTY.
///
/// Retorna un `SmallVec` vacío para teclas no mapeadas (el caller ignora
/// bytes vacíos sin enviar nada al PTY).
pub fn key_to_bytes(key: KeyEvent) -> SmallVec<[u8; 8]> {
    let mut seq: SmallVec<[u8; 8]> = SmallVec::new();

    match (key.code, key.modifiers) {
        // Ctrl+<letter>: Ctrl+A=1, Ctrl+B=2, ..., Ctrl+Z=26
        (KeyCode::Char(c), mods) if mods.contains(KeyModifiers::CONTROL) => {
            if c.is_ascii_alphabetic() {
                seq.push((c.to_ascii_lowercase() as u8) - b'a' + 1);
            }
        }
        // Alt+<char>: ESC prefix + char UTF-8
        (KeyCode::Char(c), mods) if mods.contains(KeyModifiers::ALT) => {
            seq.push(0x1b);
            let mut buf = [0u8; 4];
            seq.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        // Char normal (incluyendo UTF-8 multibyte)
        (KeyCode::Char(c), _) => {
            let mut buf = [0u8; 4];
            seq.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        (KeyCode::Enter, _) => seq.push(b'\r'),
        (KeyCode::Backspace, _) => seq.push(0x7f),
        (KeyCode::Tab, _) => seq.push(b'\t'),
        (KeyCode::Esc, _) => seq.push(0x1b),
        // Arrow keys — standard CSI sequences
        (KeyCode::Up, _) => seq.extend_from_slice(b"\x1b[A"),
        (KeyCode::Down, _) => seq.extend_from_slice(b"\x1b[B"),
        (KeyCode::Right, _) => seq.extend_from_slice(b"\x1b[C"),
        (KeyCode::Left, _) => seq.extend_from_slice(b"\x1b[D"),
        // Navigation keys
        (KeyCode::Home, _) => seq.extend_from_slice(b"\x1b[H"),
        (KeyCode::End, _) => seq.extend_from_slice(b"\x1b[F"),
        (KeyCode::PageUp, _) => seq.extend_from_slice(b"\x1b[5~"),
        (KeyCode::PageDown, _) => seq.extend_from_slice(b"\x1b[6~"),
        (KeyCode::Delete, _) => seq.extend_from_slice(b"\x1b[3~"),
        (KeyCode::Insert, _) => seq.extend_from_slice(b"\x1b[2~"),
        // Function keys — F1-F4 use SS3, F5+ use CSI
        (KeyCode::F(n), _) => match n {
            1 => seq.extend_from_slice(b"\x1bOP"),
            2 => seq.extend_from_slice(b"\x1bOQ"),
            3 => seq.extend_from_slice(b"\x1bOR"),
            4 => seq.extend_from_slice(b"\x1bOS"),
            5 => seq.extend_from_slice(b"\x1b[15~"),
            6 => seq.extend_from_slice(b"\x1b[17~"),
            7 => seq.extend_from_slice(b"\x1b[18~"),
            8 => seq.extend_from_slice(b"\x1b[19~"),
            _ => {}
        },
        _ => {}
    }
    seq
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn test_enter_sends_cr() {
        let bytes = key_to_bytes(key(KeyCode::Enter));
        assert_eq!(bytes.as_slice(), b"\r");
    }

    #[test]
    fn test_backspace_sends_del() {
        let bytes = key_to_bytes(key(KeyCode::Backspace));
        assert_eq!(bytes.as_slice(), &[0x7f]);
    }

    #[test]
    fn test_tab_sends_tab() {
        let bytes = key_to_bytes(key(KeyCode::Tab));
        assert_eq!(bytes.as_slice(), b"\t");
    }

    #[test]
    fn test_ctrl_c() {
        let bytes = key_to_bytes(ctrl(KeyCode::Char('c')));
        assert_eq!(bytes.as_slice(), &[0x03]);
    }

    #[test]
    fn test_ctrl_d() {
        let bytes = key_to_bytes(ctrl(KeyCode::Char('d')));
        assert_eq!(bytes.as_slice(), &[0x04]);
    }

    #[test]
    fn test_ctrl_l() {
        let bytes = key_to_bytes(ctrl(KeyCode::Char('l')));
        assert_eq!(bytes.as_slice(), &[0x0c]);
    }

    #[test]
    fn test_arrow_up() {
        let bytes = key_to_bytes(key(KeyCode::Up));
        assert_eq!(bytes.as_slice(), b"\x1b[A");
    }

    #[test]
    fn test_arrow_down() {
        let bytes = key_to_bytes(key(KeyCode::Down));
        assert_eq!(bytes.as_slice(), b"\x1b[B");
    }

    #[test]
    fn test_home_end() {
        assert_eq!(key_to_bytes(key(KeyCode::Home)).as_slice(), b"\x1b[H");
        assert_eq!(key_to_bytes(key(KeyCode::End)).as_slice(), b"\x1b[F");
    }

    #[test]
    fn test_char_passthrough() {
        let bytes = key_to_bytes(key(KeyCode::Char('a')));
        assert_eq!(bytes.as_slice(), b"a");
    }

    #[test]
    fn test_alt_char() {
        let bytes = key_to_bytes(alt(KeyCode::Char('b')));
        assert_eq!(bytes.as_slice(), b"\x1bb");
    }

    #[test]
    fn test_unicode_char() {
        let bytes = key_to_bytes(key(KeyCode::Char('é')));
        assert_eq!(bytes.as_slice(), "é".as_bytes());
    }

    #[test]
    fn test_esc_sends_esc() {
        let bytes = key_to_bytes(key(KeyCode::Esc));
        assert_eq!(bytes.as_slice(), &[0x1b]);
    }
}
