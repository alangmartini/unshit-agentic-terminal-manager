//! Hand-rolled syntax scanner for inline source snippets.
//!
//! Deliberately tiny: one pass per line, a keyword table per language, and
//! block-comment state carried between lines. Good enough to colour a
//! ten-line excerpt; the swap point for a real grammar engine is
//! [`tokenize`] (same signature, richer kinds).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    CSharp,
    Plain,
}

impl Language {
    /// Pick a language from the file extension; unknown extensions are
    /// [`Language::Plain`] (identifiers and punctuation only).
    pub fn from_path(path: &str) -> Language {
        let ext = path
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| ext.to_ascii_lowercase());
        match ext.as_deref() {
            Some("rs") => Language::Rust,
            Some("ts" | "mts" | "cts") => Language::TypeScript,
            Some("tsx") => Language::Tsx,
            Some("js" | "mjs" | "cjs" | "jsx") => Language::JavaScript,
            Some("py") => Language::Python,
            Some("go") => Language::Go,
            Some("cs") => Language::CSharp,
            _ => Language::Plain,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::CSharp => "csharp",
            Language::Plain => "plain",
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        const JS: &[&str] = &[
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "null",
            "of",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "yield",
        ];
        const TS: &[&str] = &[
            "abstract",
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "declare",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "implements",
            "import",
            "in",
            "instanceof",
            "interface",
            "keyof",
            "let",
            "namespace",
            "new",
            "null",
            "of",
            "private",
            "protected",
            "public",
            "readonly",
            "return",
            "satisfies",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "type",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "yield",
        ];
        const RUST: &[&str] = &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while",
        ];
        const PYTHON: &[&str] = &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield",
        ];
        const GO: &[&str] = &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "false",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "nil",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "true",
            "type",
            "var",
        ];
        const CSHARP: &[&str] = &[
            "abstract",
            "as",
            "async",
            "await",
            "base",
            "bool",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delegate",
            "do",
            "else",
            "enum",
            "event",
            "false",
            "finally",
            "for",
            "foreach",
            "get",
            "if",
            "in",
            "int",
            "interface",
            "internal",
            "is",
            "lock",
            "namespace",
            "new",
            "null",
            "object",
            "out",
            "override",
            "private",
            "protected",
            "public",
            "readonly",
            "ref",
            "return",
            "sealed",
            "set",
            "static",
            "string",
            "struct",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "using",
            "var",
            "virtual",
            "void",
            "while",
            "yield",
        ];
        match self {
            Language::Rust => RUST,
            Language::TypeScript | Language::Tsx => TS,
            Language::JavaScript => JS,
            Language::Python => PYTHON,
            Language::Go => GO,
            Language::CSharp => CSHARP,
            Language::Plain => &[],
        }
    }

    fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Python => Some("#"),
            Language::Plain => None,
            _ => Some("//"),
        }
    }

    fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Language::Python | Language::Plain => None,
            _ => Some(("/*", "*/")),
        }
    }

    fn string_quotes(self) -> &'static [char] {
        match self {
            Language::Rust | Language::Go | Language::CSharp => &['"'],
            Language::Python => &['"', '\''],
            Language::Plain => &[],
            _ => &['"', '\'', '`'],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    Str,
    Number,
    Keyword,
    Ident,
    Punct,
    Ws,
}

impl TokenKind {
    /// CSS class for the token's span.
    pub fn class(self) -> &'static str {
        match self {
            TokenKind::Comment => "tok-comment",
            TokenKind::Str => "tok-str",
            TokenKind::Number => "tok-number",
            TokenKind::Keyword => "tok-keyword",
            TokenKind::Ident => "tok-ident",
            TokenKind::Punct => "tok-punct",
            TokenKind::Ws => "tok-ws",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

/// Split one line into coloured runs. `in_block_comment` carries `/* … */`
/// state across lines; strings never span lines (a template literal that
/// does is coloured per line as far as its opening quote).
pub fn tokenize(line: &str, lang: Language, in_block_comment: &mut bool) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;

    let push = |out: &mut Vec<Token>, text: &str, kind: TokenKind| {
        if text.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut() {
            if last.kind == kind && matches!(kind, TokenKind::Punct | TokenKind::Ws) {
                last.text.push_str(text);
                return;
            }
        }
        out.push(Token {
            text: text.to_string(),
            kind,
        });
    };

    while i < line.len() {
        let rest = &line[i..];

        if *in_block_comment {
            let end = lang.block_comment().map(|(_, close)| close).unwrap_or("*/");
            match rest.find(end) {
                Some(pos) => {
                    let stop = i + pos + end.len();
                    push(&mut out, &line[i..stop], TokenKind::Comment);
                    *in_block_comment = false;
                    i = stop;
                }
                None => {
                    push(&mut out, rest, TokenKind::Comment);
                    i = line.len();
                }
            }
            continue;
        }

        if let Some((open, _)) = lang.block_comment() {
            if rest.starts_with(open) {
                *in_block_comment = true;
                // Loop back into the block-comment arm to find the close.
                continue;
            }
        }
        if let Some(marker) = lang.line_comment() {
            if rest.starts_with(marker) {
                push(&mut out, rest, TokenKind::Comment);
                i = line.len();
                continue;
            }
        }

        let c = rest.chars().next().unwrap_or(' ');
        if lang.string_quotes().contains(&c) {
            let mut j = i + c.len_utf8();
            let mut escaped = false;
            let mut closed = false;
            while j < line.len() {
                let ch = line[j..].chars().next().unwrap_or(' ');
                j += ch.len_utf8();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == c {
                    closed = true;
                    break;
                }
            }
            let _ = closed;
            push(&mut out, &line[i..j], TokenKind::Str);
            i = j;
            continue;
        }

        if c.is_ascii_digit() {
            let mut j = i;
            while j < line.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_')
            {
                j += 1;
            }
            push(&mut out, &line[i..j], TokenKind::Number);
            i = j;
            continue;
        }

        if c.is_alphabetic() || c == '_' || c == '$' {
            let mut j = i;
            while j < line.len() {
                let ch = line[j..].chars().next().unwrap_or(' ');
                if ch.is_alphanumeric() || ch == '_' || ch == '$' {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &line[i..j];
            let kind = if lang.keywords().contains(&word) {
                TokenKind::Keyword
            } else {
                TokenKind::Ident
            };
            push(&mut out, word, kind);
            i = j;
            continue;
        }

        if c.is_whitespace() {
            let mut j = i;
            while j < line.len() {
                let ch = line[j..].chars().next().unwrap_or('x');
                if ch.is_whitespace() {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            push(&mut out, &line[i..j], TokenKind::Ws);
            i = j;
            continue;
        }

        let j = i + c.len_utf8();
        push(&mut out, &line[i..j], TokenKind::Punct);
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str, lang: Language) -> Vec<(String, TokenKind)> {
        let mut block = false;
        tokenize(line, lang, &mut block)
            .into_iter()
            .map(|t| (t.text, t.kind))
            .collect()
    }

    #[test]
    fn language_from_extension() {
        assert_eq!(Language::from_path("a/b/Editor.tsx"), Language::Tsx);
        assert_eq!(Language::from_path("src/main.rs"), Language::Rust);
        assert_eq!(Language::from_path("x.PY"), Language::Python);
        assert_eq!(Language::from_path("Makefile"), Language::Plain);
        assert_eq!(Language::from_path("dir.v2/notes"), Language::Plain);
    }

    #[test]
    fn typescript_line_has_keywords_strings_numbers_and_comment() {
        let toks = kinds(
            "const x = fetch('/api', { retries: 3 }); // go",
            Language::TypeScript,
        );
        assert_eq!(toks[0], ("const".into(), TokenKind::Keyword));
        assert!(toks.contains(&("'/api'".into(), TokenKind::Str)));
        assert!(toks.contains(&("3".into(), TokenKind::Number)));
        assert_eq!(toks.last().unwrap(), &("// go".into(), TokenKind::Comment));
        assert!(toks.contains(&("fetch".into(), TokenKind::Ident)));
    }

    #[test]
    fn rust_line() {
        let toks = kinds("pub fn open(&self) -> Result<(), E> {", Language::Rust);
        assert_eq!(toks[0], ("pub".into(), TokenKind::Keyword));
        assert_eq!(toks[2], ("fn".into(), TokenKind::Keyword));
        assert!(toks.contains(&("Result".into(), TokenKind::Ident)));
        assert!(toks
            .iter()
            .any(|(t, k)| t == "->" && *k == TokenKind::Punct));
    }

    #[test]
    fn python_hash_comment_and_single_quotes() {
        let toks = kinds("if x is None:  # guard 'q'", Language::Python);
        assert_eq!(toks[0], ("if".into(), TokenKind::Keyword));
        assert_eq!(
            toks.last().unwrap(),
            &("# guard 'q'".into(), TokenKind::Comment)
        );
        let toks = kinds("s = 'it\\'s'", Language::Python);
        assert!(toks.contains(&("'it\\'s'".into(), TokenKind::Str)));
    }

    #[test]
    fn block_comment_carries_across_lines() {
        let mut block = false;
        let first = tokenize("let a = 1; /* start", Language::JavaScript, &mut block);
        assert!(block);
        assert_eq!(first.last().unwrap().kind, TokenKind::Comment);
        assert_eq!(first.last().unwrap().text, "/* start");
        let middle = tokenize("still inside", Language::JavaScript, &mut block);
        assert_eq!(middle.len(), 1);
        assert_eq!(middle[0].kind, TokenKind::Comment);
        assert!(block);
        let last = tokenize("end */ let b = 2;", Language::JavaScript, &mut block);
        assert!(!block);
        assert_eq!(
            last[0],
            Token {
                text: "end */".into(),
                kind: TokenKind::Comment
            }
        );
        assert!(last
            .iter()
            .any(|t| t.text == "let" && t.kind == TokenKind::Keyword));
    }

    #[test]
    fn go_and_csharp_and_plain() {
        let toks = kinds("func main() { return nil }", Language::Go);
        assert_eq!(toks[0], ("func".into(), TokenKind::Keyword));
        assert!(toks.contains(&("nil".into(), TokenKind::Keyword)));
        let toks = kinds("public async Task Run() => await x;", Language::CSharp);
        assert_eq!(toks[0], ("public".into(), TokenKind::Keyword));
        assert!(toks.contains(&("await".into(), TokenKind::Keyword)));
        let toks = kinds("fn if // x", Language::Plain);
        assert!(toks.iter().all(|(_, k)| *k != TokenKind::Keyword));
        assert!(toks.iter().all(|(_, k)| *k != TokenKind::Comment));
    }

    #[test]
    fn unterminated_string_runs_to_end_of_line() {
        let toks = kinds("x = `hello ${name}", Language::JavaScript);
        assert_eq!(
            toks.last().unwrap(),
            &("`hello ${name}".into(), TokenKind::Str)
        );
    }

    #[test]
    fn whitespace_and_punct_runs_merge() {
        let toks = kinds("a   ==  b", Language::Plain);
        assert_eq!(
            toks,
            vec![
                ("a".into(), TokenKind::Ident),
                ("   ".into(), TokenKind::Ws),
                ("==".into(), TokenKind::Punct),
                ("  ".into(), TokenKind::Ws),
                ("b".into(), TokenKind::Ident),
            ]
        );
    }

    #[test]
    fn round_trip_preserves_text() {
        for (line, lang) in [
            ("const x = 'a' + `b${c}` // d", Language::TypeScript),
            ("  def f(x): return x  # ok", Language::Python),
            (
                "\u{e9}t\u{e9} = \"caf\u{e9}\"; /* \u{2603} */",
                Language::JavaScript,
            ),
        ] {
            let mut block = false;
            let joined: String = tokenize(line, lang, &mut block)
                .iter()
                .map(|t| t.text.as_str())
                .collect();
            assert_eq!(joined, line);
        }
    }
}
