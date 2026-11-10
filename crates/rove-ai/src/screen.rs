//! Plain text of what a terminal showed, from its raw output: escape sequences removed, carriage
//! returns and backspaces applied to the line, the last lines kept.

/// The last `lines` lines of `output` as plain text.
pub fn recent_text(output: &[u8], lines: usize) -> String {
    let text = String::from_utf8_lossy(output);
    let mut chars = text.chars().peekable();
    let mut done: Vec<String> = Vec::new();
    let mut line = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters up to a final byte.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC and other strings: up to BEL or ESC \.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Charset selection takes one more character.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\n' => done.push(std::mem::take(&mut line)),
            '\r' => {
                // `\r\n` ends the line; a lone `\r` starts it over.
                if chars.peek() != Some(&'\n') {
                    line.clear();
                }
            }
            '\x08' => {
                line.pop();
            }
            '\t' => line.push(c),
            c if c.is_control() => {}
            c => line.push(c),
        }
    }
    if !line.is_empty() {
        done.push(line);
    }
    let mut kept: Vec<&str> = done.iter().map(|line| line.trim_end()).collect();
    while kept.last() == Some(&"") {
        kept.pop();
    }
    let start = kept.len().saturating_sub(lines);
    kept[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes_and_applies_line_edits() {
        let raw = b"\x1b]0;user@host: ~\x07\x1b[1;32muser@host\x1b[0m:~$ ls\r\na  b\r\nprogress 10%\rprogress 100%\r\nab\x08c\r\n\x1b(B$ \r\n\r\n";
        assert_eq!(
            recent_text(raw, 100),
            "user@host:~$ ls\na  b\nprogress 100%\nac\n$"
        );
        assert_eq!(recent_text(raw, 2), "ac\n$");
    }
}
