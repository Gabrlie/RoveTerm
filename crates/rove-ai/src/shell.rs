//! A conservative POSIX shell lexer for risk assessment. It does not run or expand anything; it
//! recovers enough structure — pipelines, simple commands, words with quotes removed, redirects and
//! nested command text — to see which programs a command line would run and what it would write.
//! Anything it cannot resolve statically is marked dynamic so the caller can treat it as unknown.

/// One word of a simple command, with quoting removed (`r''m` reads `rm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// Part of the word comes from an expansion (`$X`, `$(…)`, `${…}`, `$'…'`), so its real value
    /// is unknown here.
    pub dynamic: bool,
    /// Quoting or escaping prevents a word from being a shell reserved word.
    pub quoted: bool,
    /// The word began with a literal variable name followed by an unquoted `=`.
    pub assignment: bool,
}

pub(crate) fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An output or input redirection of a simple command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// Writes to the target (`>`, `>>`, `>|`, `&>`, `<>`), as opposed to reading it.
    pub writes: bool,
    pub target: Word,
}

/// A simple command: words and redirects between pipeline operators.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segment {
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
    /// Standard input comes from an inline here-document (`<<`) or here-string (`<<<`).
    pub inline_stdin: bool,
}

/// The structure of a command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Pipelines (separated by `;`, `&&`, `||`, `&` or newlines), each a list of segments joined
    /// by `|`.
    pub pipelines: Vec<Vec<Segment>>,
    /// Source text of command substitutions, subshells and arithmetic, to assess on their own.
    pub nested: Vec<String>,
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    parsed: Parsed,
    pipeline: Vec<Segment>,
    segment: Segment,
    word: Option<Word>,
    /// The next word completes a redirect (`writes`), rather than joining the command.
    pending_redirect: Option<bool>,
    /// Here-document delimiters waiting for the end of the line.
    heredocs: Vec<String>,
    _source: &'a str,
}

/// Splits `line` into pipelines and simple commands.
pub fn parse(line: &str) -> Parsed {
    let mut lexer = Lexer {
        chars: line.chars().collect(),
        pos: 0,
        parsed: Parsed::default(),
        pipeline: Vec::new(),
        segment: Segment::default(),
        word: None,
        pending_redirect: None,
        heredocs: Vec::new(),
        _source: line,
    };
    lexer.run();
    lexer.parsed
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn word(&mut self) -> &mut Word {
        self.word.get_or_insert_with(|| Word {
            text: String::new(),
            dynamic: false,
            quoted: false,
            assignment: false,
        })
    }

    fn end_word(&mut self) {
        let Some(word) = self.word.take() else {
            return;
        };
        match self.pending_redirect.take() {
            Some(writes) => self.segment.redirects.push(Redirect {
                writes,
                target: word,
            }),
            None => self.segment.words.push(word),
        }
    }

    fn end_segment(&mut self) {
        self.end_word();
        // A redirect operator without a target is a syntax error; nothing to record.
        self.pending_redirect = None;
        let segment = std::mem::take(&mut self.segment);
        if !segment.words.is_empty() || !segment.redirects.is_empty() {
            self.pipeline.push(segment);
        }
    }

    fn end_pipeline(&mut self) {
        self.end_segment();
        let pipeline = std::mem::take(&mut self.pipeline);
        if !pipeline.is_empty() {
            self.parsed.pipelines.push(pipeline);
        }
    }

    /// Text up to the bracket closing the one just consumed, honouring nesting and quotes.
    fn balanced(&mut self, open: char, close: char) -> String {
        let mut depth = 1;
        let mut text = String::new();
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '\\' => {
                    text.push(c);
                    if let Some(next) = self.peek(0) {
                        text.push(next);
                        self.pos += 1;
                    }
                    continue;
                }
                '\'' | '"' | '`' => {
                    text.push(c);
                    while let Some(inner) = self.peek(0) {
                        self.pos += 1;
                        text.push(inner);
                        if inner == '\\' && c != '\'' {
                            if let Some(next) = self.peek(0) {
                                text.push(next);
                                self.pos += 1;
                            }
                        } else if inner == c {
                            break;
                        }
                    }
                    continue;
                }
                _ if c == open => depth += 1,
                _ if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return text;
                    }
                }
                _ => {}
            }
            text.push(c);
        }
        text
    }

    /// Handles `$…` after the `$` has been consumed, inside or outside double quotes.
    fn dollar(&mut self) {
        match self.peek(0) {
            Some('(') if self.peek(1) == Some('(') => {
                self.pos += 2;
                let inner = self.balanced('(', ')');
                // Arithmetic `$(( … ))`: the second `)` closes the outer bracket.
                if self.peek(0) == Some(')') {
                    self.pos += 1;
                }
                self.parsed.nested.push(inner);
                self.word().dynamic = true;
            }
            Some('(') => {
                self.pos += 1;
                let inner = self.balanced('(', ')');
                self.parsed.nested.push(inner);
                self.word().dynamic = true;
            }
            Some('{') => {
                self.pos += 1;
                let inner = self.balanced('{', '}');
                // `${x:-$(cmd)}` may hide a substitution.
                if inner.contains("$(") || inner.contains('`') {
                    self.parsed.nested.push(inner);
                }
                self.word().dynamic = true;
            }
            Some(c) if c.is_ascii_alphanumeric() || "_@*#?$!-".contains(c) => {
                self.pos += 1;
                if c.is_ascii_alphabetic() || c == '_' {
                    while self
                        .peek(0)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        self.pos += 1;
                    }
                }
                self.word().dynamic = true;
            }
            _ => self.word().text.push('$'),
        }
    }

    fn backtick(&mut self) {
        let mut inner = String::new();
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '\\' => {
                    if let Some(next) = self.peek(0) {
                        inner.push(next);
                        self.pos += 1;
                    }
                }
                '`' => break,
                _ => inner.push(c),
            }
        }
        self.parsed.nested.push(inner);
        self.word().dynamic = true;
    }

    fn single_quoted(&mut self) {
        self.word().quoted = true;
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            if c == '\'' {
                return;
            }
            self.word().text.push(c);
        }
    }

    fn double_quoted(&mut self) {
        self.word().quoted = true;
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '"' => return,
                '\\' => match self.peek(0) {
                    Some(next) if "\"\\$`\n".contains(next) => {
                        self.pos += 1;
                        if next != '\n' {
                            self.word().text.push(next);
                        }
                    }
                    _ => self.word().text.push('\\'),
                },
                '$' => self.dollar(),
                '`' => self.backtick(),
                _ => self.word().text.push(c),
            }
        }
    }

    /// Skips here-document bodies after a newline, up to each delimiter line.
    fn skip_heredocs(&mut self) {
        for delimiter in std::mem::take(&mut self.heredocs) {
            loop {
                let start = self.pos;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
                let line: String = self.chars[start..self.pos].iter().collect();
                if self.peek(0) == Some('\n') {
                    self.pos += 1;
                }
                if line.trim_start_matches('\t') == delimiter || self.peek(0).is_none() {
                    break;
                }
            }
        }
    }

    /// Reads a redirect operator starting at the current `<` or `>`; `fd_word` tells whether the
    /// word so far was a file descriptor number (`2>`), which then is not a command word.
    fn redirect(&mut self) {
        if self
            .word
            .as_ref()
            .is_some_and(|word| !word.dynamic && word.text.chars().all(|c| c.is_ascii_digit()))
        {
            self.word = None;
        } else {
            self.end_word();
        }
        let first = self.chars[self.pos];
        self.pos += 1;
        if first == '<' {
            match self.peek(0) {
                Some('<') if self.peek(1) == Some('<') => {
                    // Here-string: the word is data, not a file.
                    self.pos += 2;
                    self.pending_redirect = None;
                    self.segment.inline_stdin = true;
                    self.skip_blanks();
                    self.discard_word();
                    return;
                }
                Some('<') => {
                    self.pos += 1;
                    if self.peek(0) == Some('-') {
                        self.pos += 1;
                    }
                    self.segment.inline_stdin = true;
                    self.skip_blanks();
                    let delimiter = self.discard_word();
                    self.heredocs.push(delimiter);
                    return;
                }
                Some('>') => {
                    self.pos += 1;
                    self.pending_redirect = Some(true);
                    return;
                }
                Some('&') => {
                    self.pos += 1;
                    self.skip_blanks();
                    self.discard_word();
                    return;
                }
                _ => {
                    self.pending_redirect = Some(false);
                    return;
                }
            }
        }
        // `>`, `>>`, `>|`, `>&N`.
        if matches!(self.peek(0), Some('>') | Some('|')) {
            self.pos += 1;
        }
        if self.peek(0) == Some('&') {
            self.pos += 1;
            // `>&2` duplicates a descriptor; `>&file` would write a file.
            self.skip_blanks();
            if self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '-') {
                self.discard_word();
                return;
            }
        }
        self.pending_redirect = Some(true);
    }

    fn skip_blanks(&mut self) {
        while matches!(self.peek(0), Some(' ') | Some('\t')) {
            self.pos += 1;
        }
    }

    /// Consumes one plain word (as a delimiter or descriptor) and returns its text.
    fn discard_word(&mut self) -> String {
        let mut text = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_whitespace() || ";|&<>()".contains(c) {
                break;
            }
            self.pos += 1;
            if c != '\'' && c != '"' && c != '\\' {
                text.push(c);
            }
        }
        text
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' => {
                    self.pos += 1;
                    self.end_word();
                }
                '\n' => {
                    self.pos += 1;
                    self.end_pipeline();
                    self.skip_heredocs();
                }
                ';' => {
                    self.pos += 1;
                    self.end_pipeline();
                }
                '&' if self.peek(1) == Some('&') => {
                    self.pos += 2;
                    self.end_pipeline();
                }
                '&' if self.peek(1) == Some('>') => {
                    // `&>file` / `&>>file`: stdout and stderr to a file.
                    self.end_word();
                    self.pos += 2;
                    if self.peek(0) == Some('>') {
                        self.pos += 1;
                    }
                    self.pending_redirect = Some(true);
                }
                '&' => {
                    self.pos += 1;
                    self.end_pipeline();
                }
                '|' if self.peek(1) == Some('|') => {
                    self.pos += 2;
                    self.end_pipeline();
                }
                '|' => {
                    self.pos += 1;
                    if self.peek(0) == Some('&') {
                        self.pos += 1;
                    }
                    self.end_segment();
                }
                '<' | '>' => self.redirect(),
                '(' if self.word.is_none() => {
                    self.pos += 1;
                    let inner = self.balanced('(', ')');
                    self.parsed.nested.push(inner);
                }
                ')' => {
                    // Unbalanced; ignore.
                    self.pos += 1;
                    self.end_word();
                }
                '#' if self.word.is_none() => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                '\\' => {
                    self.pos += 1;
                    match self.peek(0) {
                        Some('\n') => self.pos += 1,
                        Some(next) => {
                            self.pos += 1;
                            let word = self.word();
                            word.quoted = true;
                            word.text.push(next);
                        }
                        None => {}
                    }
                }
                '\'' => {
                    self.pos += 1;
                    self.single_quoted();
                }
                '"' => {
                    self.pos += 1;
                    self.double_quoted();
                }
                '$' if self.peek(1) == Some('\'') => {
                    // ANSI-C quoting can spell anything with escapes: treat as unknown.
                    self.pos += 2;
                    self.single_quoted();
                    self.word().dynamic = true;
                }
                '$' => {
                    self.pos += 1;
                    self.dollar();
                }
                '`' => {
                    self.pos += 1;
                    self.backtick();
                }
                '=' => {
                    self.pos += 1;
                    let word = self.word();
                    word.assignment |=
                        !word.quoted && !word.dynamic && is_variable_name(&word.text);
                    word.text.push(c);
                }
                _ => {
                    self.pos += 1;
                    self.word().text.push(c);
                }
            }
        }
        self.end_pipeline();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(segment: &Segment) -> Vec<&str> {
        segment
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect()
    }

    #[test]
    fn splits_pipelines_and_segments() {
        let parsed = parse("ls -la | grep x && echo 'a b'; cat f");
        assert_eq!(parsed.pipelines.len(), 3);
        assert_eq!(words(&parsed.pipelines[0][0]), ["ls", "-la"]);
        assert_eq!(words(&parsed.pipelines[0][1]), ["grep", "x"]);
        assert_eq!(words(&parsed.pipelines[1][0]), ["echo", "a b"]);
        assert_eq!(words(&parsed.pipelines[2][0]), ["cat", "f"]);
    }

    #[test]
    fn removes_quotes_and_escapes() {
        let parsed = parse(r#"r''m -r"f" \/ "a\"b""#);
        assert_eq!(words(&parsed.pipelines[0][0]), ["rm", "-rf", "/", "a\"b"]);
    }

    #[test]
    fn records_redirects_and_ignores_descriptor_duplication() {
        let parsed = parse("cmd > out.txt 2>&1 < in >> log &>all");
        let segment = &parsed.pipelines[0][0];
        assert_eq!(words(segment), ["cmd"]);
        let targets: Vec<(&str, bool)> = segment
            .redirects
            .iter()
            .map(|r| (r.target.text.as_str(), r.writes))
            .collect();
        assert_eq!(
            targets,
            [
                ("out.txt", true),
                ("in", false),
                ("log", true),
                ("all", true)
            ]
        );
    }

    #[test]
    fn collects_nested_commands_and_marks_expansions() {
        let parsed = parse("echo $(rm -rf /) `id` $HOME ${X} $'\\x72m' (cd /; ls)");
        assert_eq!(parsed.nested, ["rm -rf /", "id", "cd /; ls"]);
        let segment = &parsed.pipelines[0][0];
        assert!(segment.words[1].dynamic);
        assert!(segment.words[3].dynamic);
        assert!(segment.words[5].dynamic);
    }

    #[test]
    fn skips_heredoc_bodies_and_comments() {
        let parsed = parse("cat <<EOF > f\nrm -rf /\nEOF\nls # rm -rf /");
        assert_eq!(parsed.pipelines.len(), 2);
        assert_eq!(words(&parsed.pipelines[0][0]), ["cat"]);
        assert!(parsed.pipelines[0][0].inline_stdin);
        assert_eq!(words(&parsed.pipelines[1][0]), ["ls"]);
        assert!(!parsed.pipelines[1][0].inline_stdin);
        let here_string = parse("python3 - <<< 'print(1)'");
        assert!(here_string.pipelines[0][0].inline_stdin);
    }
}
