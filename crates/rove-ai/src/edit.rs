//! Editing a text by replacing a passage, as the `edit_file` tool does, and showing the change.

/// Longest change shown in an approval.
const MAX_DIFF: usize = 8 * 1024;

/// `text` with `old` replaced by `new`, and how many places changed. `old` must occur exactly
/// once, or at least once when `replace_all`.
pub fn replace(
    text: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("the text to replace is empty".into());
    }
    if old == new {
        return Err("the replacement is the same as the text".into());
    }
    let count = text.matches(old).count();
    match count {
        0 => {
            Err("the text to replace was not found; read the file again and copy it exactly".into())
        }
        1 => Ok((text.replacen(old, new, 1), 1)),
        _ if replace_all => Ok((text.replace(old, new), count)),
        _ => Err(format!(
            "the text to replace occurs {count} times; include more of the surrounding lines, or set replace_all"
        )),
    }
}

/// The change as lines: the removed ones after `- `, the added ones after `+ `, cut short when long.
pub fn diff(old: &str, new: &str) -> String {
    let mut text: String = old
        .lines()
        .map(|line| format!("- {line}\n"))
        .chain(new.lines().map(|line| format!("+ {line}\n")))
        .collect();
    if text.len() > MAX_DIFF {
        let mut end = MAX_DIFF;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n…");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passage_is_replaced_only_when_it_is_unambiguous() {
        assert_eq!(
            replace("a\nb\nc\n", "b\n", "B\n", false).unwrap(),
            ("a\nB\nc\n".into(), 1)
        );
        assert!(
            replace("x x", "x", "y", false)
                .unwrap_err()
                .contains("2 times")
        );
        assert_eq!(replace("x x", "x", "y", true).unwrap(), ("y y".into(), 2));
        assert!(
            replace("abc", "z", "y", false)
                .unwrap_err()
                .contains("not found")
        );
        assert!(replace("abc", "", "y", false).is_err());
        assert!(replace("abc", "a", "a", false).is_err());
    }

    #[test]
    fn the_change_shows_removed_and_added_lines() {
        assert_eq!(
            diff("port 80\n", "port 8080\nhost x\n"),
            "- port 80\n+ port 8080\n+ host x\n"
        );
        assert!(diff(&"y\n".repeat(10_000), "").ends_with('…'));
    }
}
