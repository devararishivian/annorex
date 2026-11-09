//! Clean the text before Annorex pastes it, and wrap it in a bracketed paste.

/// The start of a bracketed paste.
const PASTE_START: &str = "\x1b[200~";
/// The end of a bracketed paste.
const PASTE_END: &str = "\x1b[201~";

/// Make the text safe to paste. CRLF and CR become LF. LF and TAB stay. All other control
/// characters (ESC, C0, DEL, and C1) are removed, so the text cannot end the paste early or
/// send a terminal command.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The cleaned text inside a bracketed paste.
pub fn bracketed(text: &str) -> String {
    let clean = sanitize(text);
    let mut out = String::with_capacity(clean.len() + PASTE_START.len() + PASTE_END.len());
    out.push_str(PASTE_START);
    out.push_str(&clean);
    out.push_str(PASTE_END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_cr_become_lf() {
        assert_eq!(sanitize("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(sanitize("\r\r\n"), "\n\n");
    }

    #[test]
    fn tabs_and_newlines_stay() {
        assert_eq!(sanitize("a\tb\n\n\tc"), "a\tb\n\n\tc");
    }

    #[test]
    fn paste_terminator_inside_payload_is_removed() {
        let clean = sanitize("a\x1b[201~b");
        assert!(!clean.contains('\x1b'));
        assert_eq!(clean, "a[201~b");
        let wrapped = bracketed("a\x1b[201~b");
        assert_eq!(wrapped.matches('\x1b').count(), 2);
        assert_eq!(wrapped, "\x1b[200~a[201~b\x1b[201~");
    }

    #[test]
    fn c0_c1_del_and_nul_are_removed() {
        assert_eq!(sanitize("a\x00b\x07c\x7fd\u{9b}e\x08f"), "abcdef");
    }

    #[test]
    fn unicode_text_stays() {
        let text = "日本語 テスト 🎉👩‍💻 e\u{301} café";
        assert_eq!(sanitize(text), text);
    }

    #[test]
    fn empty_payload_stays_empty() {
        assert_eq!(sanitize(""), "");
        assert_eq!(bracketed(""), "\x1b[200~\x1b[201~");
    }
}
