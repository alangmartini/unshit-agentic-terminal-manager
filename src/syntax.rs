//! Hand-rolled syntax scanner for source lines.
//!
//! Deliberately tiny: one pass per line, a keyword table per language, and
//! block-comment state carried between lines. It colours Flow snippets, the
//! editor grid and the diff pane, so the hot path is the *span* API
//! ([`tokenize_spans`]) — the editor re-tokenizes every visible line on every
//! repaint, and an allocation per token there would be felt. [`tokenize`] is
//! the owned-`String` convenience wrapper the snippet renderer still uses.
//!
//! There is exactly ONE scanner ([`tokenize_spans`]); everything else maps its
//! output. Two scanners would drift, and a colouriser that disagrees with
//! itself between two panes looks broken. The swap point for a real grammar
//! engine is [`tokenize_spans`] (same signature, richer kinds).
//!
//! The bar is "good enough and never wrong-looking", not correctness: this is
//! not a parser and must never be treated as one.

/// A source language, chosen from the file name.
///
/// Variants exist per *colouring behaviour*, not per language family: `C` and
/// `Cpp` are separate only because the keyword tables differ, while `css` and
/// `scss` share one because nothing we colour distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    CSharp,
    Json,
    Toml,
    Yaml,
    Markdown,
    Shell,
    PowerShell,
    C,
    Cpp,
    Java,
    Kotlin,
    Swift,
    Ruby,
    Php,
    Sql,
    Css,
    Lua,
    Zig,
    Dockerfile,
    Makefile,
    Plain,
}

impl Language {
    /// Pick a language from the file name; unknown names are
    /// [`Language::Plain`] (identifiers and punctuation only).
    ///
    /// Two languages are matched by *basename* because their files carry no
    /// extension (`Dockerfile`, `Makefile`); everything else is matched by
    /// extension. The basename is taken across both separators because paths
    /// reach us from git (`/`) and from the Windows file dialogs (`\`).
    pub fn from_path(path: &str) -> Language {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        match name.as_str() {
            "dockerfile" => return Language::Dockerfile,
            "makefile" | "gnumakefile" => return Language::Makefile,
            _ => {}
        }
        let ext = name.rsplit_once('.').map(|(_, ext)| ext);
        match ext {
            Some("rs") => Language::Rust,
            Some("ts" | "mts" | "cts") => Language::TypeScript,
            Some("tsx") => Language::Tsx,
            Some("js" | "mjs" | "cjs" | "jsx") => Language::JavaScript,
            Some("py" | "pyi") => Language::Python,
            Some("go") => Language::Go,
            Some("cs") => Language::CSharp,
            Some("json") => Language::Json,
            Some("toml") => Language::Toml,
            Some("yaml" | "yml") => Language::Yaml,
            Some("md" | "markdown") => Language::Markdown,
            Some("sh" | "bash" | "zsh") => Language::Shell,
            Some("ps1" | "psm1" | "psd1") => Language::PowerShell,
            Some("c" | "h") => Language::C,
            Some("cc" | "cpp" | "cxx" | "hpp" | "hxx") => Language::Cpp,
            Some("java") => Language::Java,
            Some("kt" | "kts") => Language::Kotlin,
            Some("swift") => Language::Swift,
            Some("rb") => Language::Ruby,
            Some("php") => Language::Php,
            Some("sql") => Language::Sql,
            Some("css" | "scss") => Language::Css,
            Some("lua") => Language::Lua,
            Some("zig") => Language::Zig,
            _ => Language::Plain,
        }
    }

    /// Stable lowercase name, used in telemetry and in the snippet DOM.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::CSharp => "csharp",
            Language::Json => "json",
            Language::Toml => "toml",
            Language::Yaml => "yaml",
            Language::Markdown => "markdown",
            Language::Shell => "shell",
            Language::PowerShell => "powershell",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::Swift => "swift",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Sql => "sql",
            Language::Css => "css",
            Language::Lua => "lua",
            Language::Zig => "zig",
            Language::Dockerfile => "dockerfile",
            Language::Makefile => "makefile",
            Language::Plain => "plain",
        }
    }

    /// The prefix that starts a comment running to end of line, if the
    /// language has one.
    ///
    /// This is both the scanner's rule and the contract for the
    /// toggle-comment editor command, on purpose: if the two disagreed, a
    /// `Ctrl+/` would produce a line the colouriser does not paint as a
    /// comment. JSON, CSS (which only has `/* */`), Markdown and Plain have
    /// none, so toggle-comment is a no-op there.
    pub fn line_comment_prefix(self) -> Option<&'static str> {
        match self {
            Language::Rust
            | Language::TypeScript
            | Language::Tsx
            | Language::JavaScript
            | Language::Go
            | Language::CSharp
            | Language::C
            | Language::Cpp
            | Language::Java
            | Language::Kotlin
            | Language::Swift
            | Language::Php
            | Language::Zig => Some("//"),
            Language::Python
            | Language::Shell
            | Language::PowerShell
            | Language::Ruby
            | Language::Yaml
            | Language::Toml
            | Language::Makefile
            | Language::Dockerfile => Some("#"),
            Language::Sql | Language::Lua => Some("--"),
            Language::Json | Language::Css | Language::Markdown | Language::Plain => None,
        }
    }

    /// Whether a line comment only starts at the beginning of a line or after
    /// whitespace.
    ///
    /// This exists for one very visible failure: `#` in shell and YAML is only
    /// a comment at a token boundary, so without this a CI config line like
    /// `url: https://example.com/#anchor` — exactly the kind of line the diff
    /// pane shows — paints half the line as a comment. `$#` and `${#arr[@]}`
    /// in shell have the same problem. Python is deliberately excluded: its
    /// `#` is unconditional today and the snippet renderer's behaviour must
    /// not change.
    fn line_comment_needs_boundary(self) -> bool {
        matches!(
            self,
            Language::Shell
                | Language::PowerShell
                | Language::Yaml
                | Language::Toml
                | Language::Ruby
                | Language::Makefile
                | Language::Dockerfile
        )
    }

    /// Whether this language has a block comment, i.e. whether tokenizing a
    /// line can depend on the lines above it.
    ///
    /// The editor's per-pane syntax cache exists only to carry
    /// `in_block_comment` forward; for a language that answers `false` here
    /// every line is independent, so the cache (and the walk from the first
    /// damaged line on every edit) can be skipped entirely.
    pub fn has_block_comments(self) -> bool {
        self.block_comment().is_some()
    }

    /// Open/close delimiters of a block comment, if the language has one.
    ///
    /// Checked *before* the line comment, which is what lets Lua's `--[[ ]]`
    /// win over its `--`, and PowerShell's `<# #>` over its `#`. Zig has no
    /// block comment at all, despite the `//` line comment.
    fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Language::Rust
            | Language::TypeScript
            | Language::Tsx
            | Language::JavaScript
            | Language::Go
            | Language::CSharp
            | Language::C
            | Language::Cpp
            | Language::Java
            | Language::Kotlin
            | Language::Swift
            | Language::Php
            | Language::Css
            | Language::Sql => Some(("/*", "*/")),
            Language::PowerShell => Some(("<#", "#>")),
            Language::Lua => Some(("--[[", "]]")),
            _ => None,
        }
    }

    /// Quote characters that open a string literal.
    ///
    /// Rust and Go keep `"` only because `'` is a lifetime / rune sigil there
    /// and painting `'a` as an unterminated string would swallow the rest of
    /// the line. Markdown gets none: apostrophes in prose are not strings.
    fn string_quotes(self) -> &'static [char] {
        match self {
            Language::Rust | Language::Go | Language::CSharp | Language::Swift => &['"'],
            Language::Json => &['"'],
            Language::Markdown | Language::Plain => &[],
            Language::Ruby => &['"', '\'', '`'],
            Language::Python
            | Language::C
            | Language::Cpp
            | Language::Java
            | Language::Kotlin
            | Language::Zig
            | Language::Php
            | Language::Sql
            | Language::Css
            | Language::Lua
            | Language::Yaml
            | Language::Toml
            | Language::Shell
            | Language::PowerShell
            | Language::Dockerfile
            | Language::Makefile => &['"', '\''],
            Language::TypeScript | Language::Tsx | Language::JavaScript => &['"', '\'', '`'],
        }
    }

    /// Whether keywords match without regard to case.
    ///
    /// SQL is written in both cases by convention, PowerShell's keywords are
    /// genuinely case-insensitive, and Dockerfile instructions are
    /// conventionally upper but legally either.
    fn keywords_ignore_case(self) -> bool {
        matches!(
            self,
            Language::Sql | Language::PowerShell | Language::Dockerfile
        )
    }

    /// Is `word` a keyword of this language?
    ///
    /// A predicate rather than a `&[&str]` getter so `Cpp` can be "C's table
    /// plus the C++ additions" without duplicating fifty entries, and so the
    /// case-insensitive languages can compare without allocating a lowercased
    /// copy per word (this runs per token per visible line per frame).
    fn is_keyword(self, word: &str) -> bool {
        let table: &[&str] = match self {
            Language::Rust => RUST,
            Language::TypeScript | Language::Tsx => TS,
            Language::JavaScript => JS,
            Language::Python => PYTHON,
            Language::Go => GO,
            Language::CSharp => CSHARP,
            Language::Json => JSON,
            Language::Toml => TOML,
            Language::Yaml => YAML,
            Language::Shell => SHELL,
            Language::PowerShell => POWERSHELL,
            Language::C => C,
            Language::Cpp => {
                return C.contains(&word) || CPP_EXTRA.contains(&word);
            }
            Language::Java => JAVA,
            Language::Kotlin => KOTLIN,
            Language::Swift => SWIFT,
            Language::Ruby => RUBY,
            Language::Php => PHP,
            Language::Sql => SQL,
            Language::Css => CSS,
            Language::Lua => LUA,
            Language::Zig => ZIG,
            Language::Dockerfile => DOCKERFILE,
            Language::Makefile => MAKEFILE,
            // Markdown's only "keyword" is a heading line, handled by the
            // scanner's special case, so there is no word table.
            Language::Markdown | Language::Plain => &[],
        };
        if self.keywords_ignore_case() {
            table.iter().any(|k| word.eq_ignore_ascii_case(k))
        } else {
            table.contains(&word)
        }
    }
}

// ---------------------------------------------------------------------------
// Keyword tables.
//
// Data only: adding a language should mean adding a table plus arms in
// `from_path` / `as_str` / the rule getters, never touching the scanner. Each
// table is sorted so a missing entry is easy to spot in review; lookup is a
// linear scan because these are tens of entries and the scan is on already-hot
// cache lines (a HashMap would be slower here and would need lazy init).
// ---------------------------------------------------------------------------

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
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const PYTHON: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
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

/// JSON has no keywords beyond the three literals; everything else is a string
/// or a number, which is why JSON reads well with almost no table.
const JSON: &[&str] = &["false", "null", "true"];

const TOML: &[&str] = &["false", "inf", "nan", "true"];

/// YAML's booleans and nulls have more spellings than anyone remembers; these
/// are the ones YAML 1.1 parsers actually accept and that people write.
const YAML: &[&str] = &[
    "False", "Null", "TRUE", "True", "false", "no", "null", "off", "on", "true", "yes",
];

/// Shell *control flow and builtins that change scope*, not commands: painting
/// every command name as a keyword would light up the whole script.
const SHELL: &[&str] = &[
    "alias", "case", "declare", "do", "done", "elif", "else", "esac", "eval", "exec", "exit",
    "export", "fi", "for", "function", "if", "in", "local", "readonly", "return", "select", "set",
    "shift", "source", "then", "trap", "typeset", "unset", "until", "while",
];

/// PowerShell keyword table. `$true`/`$false`/`$null` are listed with their
/// sigil because the identifier scanner treats `$` as a word character, so the
/// token it produces includes it.
const POWERSHELL: &[&str] = &[
    "$false",
    "$null",
    "$true",
    "begin",
    "break",
    "catch",
    "class",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "enum",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "function",
    "hidden",
    "if",
    "in",
    "param",
    "process",
    "return",
    "static",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "while",
    "workflow",
];

const C: &[&str] = &[
    "NULL", "auto", "break", "case", "char", "const", "continue", "default", "do", "double",
    "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while",
];

/// C++ is C plus these; kept separate so `Language::Cpp` can match both tables
/// without a duplicated sixty-entry array going stale on one side.
const CPP_EXTRA: &[&str] = &[
    "bool",
    "catch",
    "class",
    "constexpr",
    "consteval",
    "decltype",
    "delete",
    "explicit",
    "false",
    "friend",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typeid",
    "typename",
    "using",
    "virtual",
];

const JAVA: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

const KOTLIN: &[&str] = &[
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "const",
    "constructor",
    "continue",
    "crossinline",
    "data",
    "do",
    "else",
    "enum",
    "external",
    "false",
    "final",
    "finally",
    "for",
    "fun",
    "if",
    "import",
    "in",
    "infix",
    "init",
    "inline",
    "interface",
    "internal",
    "is",
    "lateinit",
    "null",
    "object",
    "open",
    "operator",
    "out",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "reified",
    "return",
    "sealed",
    "super",
    "suspend",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "vararg",
    "when",
    "while",
];

const SWIFT: &[&str] = &[
    "as",
    "associatedtype",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "mutating",
    "nil",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "weak",
    "where",
    "while",
];

const RUBY: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "require",
    "require_relative",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
];

const PHP: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "null",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "true",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

/// Matched case-insensitively (see [`Language::keywords_ignore_case`]), so the
/// table is lowercase and covers `SELECT` and `select` alike.
const SQL: &[&str] = &[
    "add",
    "all",
    "alter",
    "and",
    "as",
    "asc",
    "begin",
    "between",
    "boolean",
    "by",
    "case",
    "column",
    "commit",
    "constraint",
    "create",
    "cross",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "exists",
    "foreign",
    "from",
    "full",
    "group",
    "having",
    "in",
    "index",
    "inner",
    "insert",
    "int",
    "integer",
    "into",
    "is",
    "join",
    "key",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "or",
    "order",
    "outer",
    "primary",
    "references",
    "returning",
    "right",
    "rollback",
    "select",
    "set",
    "table",
    "text",
    "then",
    "timestamp",
    "union",
    "unique",
    "update",
    "values",
    "varchar",
    "view",
    "when",
    "where",
    "with",
];

/// CSS has no statements, so the "keywords" are the at-rule names (the `@` is
/// punctuation, the word after it lands here) and the handful of global values
/// that read as syntax rather than as data.
const CSS: &[&str] = &[
    "and",
    "charset",
    "font-face",
    "import",
    "inherit",
    "initial",
    "keyframes",
    "media",
    "not",
    "only",
    "revert",
    "supports",
    "unset",
];

const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const ZIG: &[&str] = &[
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "inline",
    "linksection",
    "noalias",
    "nosuspend",
    "null",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "undefined",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
];

/// Dockerfile instructions, matched case-insensitively.
const DOCKERFILE: &[&str] = &[
    "add",
    "arg",
    "as",
    "cmd",
    "copy",
    "entrypoint",
    "env",
    "expose",
    "from",
    "healthcheck",
    "label",
    "maintainer",
    "onbuild",
    "run",
    "shell",
    "stopsignal",
    "user",
    "volume",
    "workdir",
];

/// GNU make directives. Target and variable names stay identifiers.
const MAKEFILE: &[&str] = &[
    "define", "else", "endef", "endif", "export", "ifdef", "ifeq", "ifndef", "ifneq", "include",
    "override", "unexport", "vpath",
];

/// What a run of source text is, for colouring purposes.
///
/// Deliberately coarse — the editor maps every kind to one foreground colour
/// ([`crate::editor::colors`]), so a finer taxonomy would cost a repaint's
/// worth of work and show nothing.
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

/// One coloured run of a line, as **byte offsets** into that line.
///
/// Byte offsets rather than a borrowed `&str` so callers can keep a reusable
/// `Vec<Span>` across lines and frames without fighting the borrow checker
/// over the line they came from.
///
/// The spans [`tokenize_spans`] produces tile the line exactly: they are in
/// order, non-empty, gap-free and non-overlapping, the first starts at 0 and
/// the last ends at `line.len()`. A painter can therefore walk them and colour
/// every cell without a fallback branch. Consecutive `Punct` and `Ws` runs are
/// merged into one span, identically to [`tokenize`] (which is a pure map over
/// these spans, so the two can never disagree).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// One coloured run of a line as an owned `String`.
///
/// Only for the Flow snippet renderer, which builds DOM nodes and needs owned
/// text anyway. Anything painting a grid should use [`tokenize_spans`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

/// Append a span, merging into the previous one for the kinds that
/// [`tokenize`] has always merged (`Punct`, `Ws`).
///
/// The `last.end == start` guard keeps the tiling invariant honest: we only
/// ever extend a span that is physically adjacent.
#[inline]
fn push_span(out: &mut Vec<Span>, start: usize, end: usize, kind: TokenKind) {
    if start >= end {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.kind == kind
            && last.end == start
            && matches!(kind, TokenKind::Punct | TokenKind::Ws)
        {
            last.end = end;
            return;
        }
    }
    out.push(Span { start, end, kind });
}

/// Split one line into coloured runs, writing byte-offset spans into `out`.
///
/// `out` is cleared first and reused, so a caller that keeps one `Vec` alive
/// allocates nothing per line after the first few — this runs for every
/// visible line of every editor and diff pane on every repaint.
///
/// `in_block_comment` carries `/* … */` (or `<# … #>`, `--[[ … ]]`) state
/// across lines; the caller owns that state because only the caller knows
/// where the document starts. Strings never span lines: a template literal
/// that does is coloured per line as far as its opening quote, which is wrong
/// but is only ever wrong for one line and never eats a whole file.
pub fn tokenize_spans(
    line: &str,
    lang: Language,
    in_block_comment: &mut bool,
    out: &mut Vec<Span>,
) {
    out.clear();
    if line.is_empty() {
        return;
    }

    // Markdown is prose: the only thing worth colouring is a heading, and the
    // scanner below would paint apostrophes as strings and `_word_` as
    // identifiers-with-punctuation noise. Special-case the heading and leave
    // the rest to the generic pass with an empty rule set.
    if lang == Language::Markdown {
        let indent = line.len() - line.trim_start().len();
        if line.as_bytes().get(indent) == Some(&b'#') {
            push_span(out, 0, indent, TokenKind::Ws);
            push_span(out, indent, line.len(), TokenKind::Keyword);
            return;
        }
    }

    let bytes = line.as_bytes();
    let mut i = 0;

    while i < line.len() {
        let rest = &line[i..];

        // Continuation of a block comment opened on an earlier line. The
        // `"*/"` fallback matters when the caller carries state across a
        // language change (a diff document stacks files of different kinds).
        if *in_block_comment {
            let close = lang.block_comment().map(|(_, close)| close).unwrap_or("*/");
            match rest.find(close) {
                Some(pos) => {
                    let stop = i + pos + close.len();
                    push_span(out, i, stop, TokenKind::Comment);
                    *in_block_comment = false;
                    i = stop;
                }
                None => {
                    push_span(out, i, line.len(), TokenKind::Comment);
                    i = line.len();
                }
            }
            continue;
        }

        if let Some((open, close)) = lang.block_comment() {
            if rest.starts_with(open) {
                // Search for the close *after* the opener, otherwise `/*/`
                // and PowerShell's `<#>` close themselves on their own
                // delimiter and the rest of the line loses its comment colour.
                let body = i + open.len();
                match line[body..].find(close) {
                    Some(pos) => {
                        let stop = body + pos + close.len();
                        push_span(out, i, stop, TokenKind::Comment);
                        i = stop;
                    }
                    None => {
                        push_span(out, i, line.len(), TokenKind::Comment);
                        *in_block_comment = true;
                        i = line.len();
                    }
                }
                continue;
            }
        }

        if let Some(marker) = lang.line_comment_prefix() {
            let at_boundary =
                !lang.line_comment_needs_boundary() || i == 0 || bytes[i - 1].is_ascii_whitespace();
            if at_boundary && rest.starts_with(marker) {
                push_span(out, i, line.len(), TokenKind::Comment);
                i = line.len();
                continue;
            }
        }

        let c = rest.chars().next().unwrap_or(' ');
        if lang.string_quotes().contains(&c) {
            let mut j = i + c.len_utf8();
            let mut escaped = false;
            while j < line.len() {
                let ch = line[j..].chars().next().unwrap_or(' ');
                j += ch.len_utf8();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == c {
                    break;
                }
            }
            push_span(out, i, j, TokenKind::Str);
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
            push_span(out, i, j, TokenKind::Number);
            i = j;
            continue;
        }

        // `$` is a word character so shell/PowerShell/PHP `$var` and JS
        // `$foo` come out as one identifier instead of punctuation plus a
        // word. `-` is one for CSS so `font-face` and custom properties stay
        // whole.
        let dash_is_word = lang == Language::Css;
        if c.is_alphabetic() || c == '_' || c == '$' {
            let mut j = i;
            while j < line.len() {
                let ch = line[j..].chars().next().unwrap_or(' ');
                if ch.is_alphanumeric() || ch == '_' || ch == '$' || (dash_is_word && ch == '-') {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &line[i..j];
            let kind = if lang.is_keyword(word) {
                TokenKind::Keyword
            } else {
                TokenKind::Ident
            };
            push_span(out, i, j, kind);
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
            push_span(out, i, j, TokenKind::Ws);
            i = j;
            continue;
        }

        let j = i + c.len_utf8();
        push_span(out, i, j, TokenKind::Punct);
        i = j;
    }
}

/// Split one line into owned coloured runs.
///
/// A thin map over [`tokenize_spans`] — there is one scanner, so the snippet
/// renderer and the editor can never colour the same line differently.
pub fn tokenize(line: &str, lang: Language, in_block_comment: &mut bool) -> Vec<Token> {
    let mut spans = Vec::new();
    tokenize_spans(line, lang, in_block_comment, &mut spans);
    spans
        .into_iter()
        .map(|s| Token {
            text: line[s.start..s.end].to_string(),
            kind: s.kind,
        })
        .collect()
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

    /// Spans for one line, as `(text, kind)` pairs, so a test reads like the
    /// `tokenize` tests but exercises the span path.
    fn spans(line: &str, lang: Language) -> Vec<(String, TokenKind)> {
        let mut block = false;
        let mut out = Vec::new();
        tokenize_spans(line, lang, &mut block, &mut out);
        out.into_iter()
            .map(|s| (line[s.start..s.end].to_string(), s.kind))
            .collect()
    }

    fn has(toks: &[(String, TokenKind)], text: &str, kind: TokenKind) -> bool {
        toks.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn language_from_extension() {
        assert_eq!(Language::from_path("a/b/Editor.tsx"), Language::Tsx);
        assert_eq!(Language::from_path("src/main.rs"), Language::Rust);
        assert_eq!(Language::from_path("x.PY"), Language::Python);
        // Changed from the pre-span version, which asserted `Plain`: Makefile
        // is now matched by basename per the editor-v2 spec.
        assert_eq!(Language::from_path("Makefile"), Language::Makefile);
        assert_eq!(Language::from_path("dir.v2/notes"), Language::Plain);
    }

    /// Pins every extension and basename the spec lists, because a typo in a
    /// `from_path` arm is invisible (it just silently falls through to Plain).
    #[test]
    fn language_from_path_covers_every_declared_name() {
        for (path, want) in [
            ("a.rs", Language::Rust),
            ("a.ts", Language::TypeScript),
            ("a.mts", Language::TypeScript),
            ("a.cts", Language::TypeScript),
            ("a.tsx", Language::Tsx),
            ("a.js", Language::JavaScript),
            ("a.mjs", Language::JavaScript),
            ("a.cjs", Language::JavaScript),
            ("a.jsx", Language::JavaScript),
            ("a.py", Language::Python),
            ("a.pyi", Language::Python),
            ("a.go", Language::Go),
            ("a.cs", Language::CSharp),
            ("a.json", Language::Json),
            ("a.toml", Language::Toml),
            ("a.yaml", Language::Yaml),
            ("a.yml", Language::Yaml),
            ("a.md", Language::Markdown),
            ("a.markdown", Language::Markdown),
            ("a.sh", Language::Shell),
            ("a.bash", Language::Shell),
            ("a.zsh", Language::Shell),
            ("a.ps1", Language::PowerShell),
            ("a.psm1", Language::PowerShell),
            ("a.psd1", Language::PowerShell),
            ("a.c", Language::C),
            ("a.h", Language::C),
            ("a.cc", Language::Cpp),
            ("a.cpp", Language::Cpp),
            ("a.cxx", Language::Cpp),
            ("a.hpp", Language::Cpp),
            ("a.hxx", Language::Cpp),
            ("a.java", Language::Java),
            ("a.kt", Language::Kotlin),
            ("a.kts", Language::Kotlin),
            ("a.swift", Language::Swift),
            ("a.rb", Language::Ruby),
            ("a.php", Language::Php),
            ("a.sql", Language::Sql),
            ("a.css", Language::Css),
            ("a.scss", Language::Css),
            ("a.lua", Language::Lua),
            ("a.zig", Language::Zig),
            ("Dockerfile", Language::Dockerfile),
            ("docker/Dockerfile", Language::Dockerfile),
            ("Makefile", Language::Makefile),
            ("GNUmakefile", Language::Makefile),
            ("a.exe", Language::Plain),
            ("LICENSE", Language::Plain),
        ] {
            assert_eq!(Language::from_path(path), want, "for {path}");
        }
    }

    /// Paths reach us from the Windows file dialogs with backslashes; a
    /// basename match that only splits on `/` would miss `C:\repo\Dockerfile`.
    #[test]
    fn from_path_handles_windows_separators() {
        assert_eq!(
            Language::from_path("C:\\repo\\Dockerfile"),
            Language::Dockerfile
        );
        assert_eq!(
            Language::from_path("C:\\repo\\src\\main.rs"),
            Language::Rust
        );
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

    // ------------------------------------------------------------------
    // Span API
    // ------------------------------------------------------------------

    /// The painter walks spans and colours cells with no fallback branch, so a
    /// gap, an overlap, an empty span or a short final span would leave
    /// uncoloured or double-coloured cells. Pins the tiling invariant across
    /// one line per language plus the two degenerate inputs (empty line, line
    /// wholly inside a block comment).
    #[test]
    fn spans_tile_the_line_exactly_for_every_language() {
        let samples: &[(Language, &str)] = &[
            (Language::Rust, "pub fn f(x: u32) -> u32 { x + 1 } // hi"),
            (Language::TypeScript, "const a: string = `x${y}`; /* c */"),
            (Language::Tsx, "return <Foo bar={1} />;"),
            (Language::JavaScript, "let s = 'a\\'b'; // t"),
            (Language::Python, "def f(x):  # note"),
            (Language::Go, "func main() { _ = 1 }"),
            (Language::CSharp, "var x = new List<int>();"),
            (Language::Json, "{\"a\": [1, true, null]}"),
            (Language::Toml, "key = \"value\"  # c"),
            (Language::Yaml, "name: value  # c"),
            (Language::Markdown, "## Heading with `code`"),
            (Language::Markdown, "plain prose, it's fine"),
            (Language::Shell, "for f in *.txt; do echo \"$f\"; done"),
            (Language::PowerShell, "$x = Get-Item -Path 'a' <# c #>"),
            (Language::C, "int main(void) { return 0; /* ok */ }"),
            (Language::Cpp, "std::vector<int> v; // c++"),
            (Language::Java, "public static void main(String[] a) {}"),
            (Language::Kotlin, "fun f(): Int = 1 // k"),
            (Language::Swift, "let x: Int = 1 // s"),
            (Language::Ruby, "def f(x) = x + 1 # r"),
            (Language::Php, "$a = array(1, 2); // p"),
            (Language::Sql, "SELECT * FROM t WHERE a = 1; -- q"),
            (Language::Css, ".a { color: #fff; /* c */ }"),
            (Language::Lua, "local t = {} -- l"),
            (Language::Zig, "const std = @import(\"std\"); // z"),
            (Language::Dockerfile, "RUN apt-get update # d"),
            (Language::Makefile, "build: ## help\n"),
            (Language::Plain, "anything at all 123"),
            (Language::Rust, ""),
            (Language::Rust, "   "),
            (Language::Yaml, "\u{e9}: \u{2603}"),
        ];
        let mut out = Vec::new();
        for (lang, line) in samples {
            for start_in_block in [false, true] {
                let mut block = start_in_block;
                tokenize_spans(line, *lang, &mut block, &mut out);
                if line.is_empty() {
                    assert!(out.is_empty(), "{lang:?} empty line produced spans");
                    continue;
                }
                assert_eq!(out[0].start, 0, "{lang:?} {line:?} does not start at 0");
                assert_eq!(
                    out.last().unwrap().end,
                    line.len(),
                    "{lang:?} {line:?} does not reach the end"
                );
                for w in out.windows(2) {
                    assert_eq!(w[0].end, w[1].start, "{lang:?} {line:?} gap or overlap");
                }
                for s in out.iter() {
                    assert!(s.start < s.end, "{lang:?} {line:?} empty span");
                    // Every boundary must be a char boundary or slicing panics.
                    assert!(line.is_char_boundary(s.start) && line.is_char_boundary(s.end));
                }
            }
        }
    }

    /// `tokenize` must be exactly `tokenize_spans` + slicing, including the
    /// block-comment state it leaves behind. Pins the "one scanner" property:
    /// if someone re-implements either side, this fails.
    #[test]
    fn tokenize_is_a_pure_map_over_spans() {
        for (line, lang) in [
            ("let a = 1; /* open", Language::JavaScript),
            ("a   ==  b", Language::Plain),
            ("SELECT a FROM t -- x", Language::Sql),
        ] {
            let mut b1 = false;
            let toks = tokenize(line, lang, &mut b1);
            let mut b2 = false;
            let mut sp = Vec::new();
            tokenize_spans(line, lang, &mut b2, &mut sp);
            assert_eq!(b1, b2, "block state diverged for {line:?}");
            assert_eq!(toks.len(), sp.len(), "token count diverged for {line:?}");
            for (t, s) in toks.iter().zip(sp.iter()) {
                assert_eq!(t.text, line[s.start..s.end]);
                assert_eq!(t.kind, s.kind);
            }
        }
    }

    /// `out` is caller-owned and reused across lines; a stale tail from the
    /// previous (longer) line would paint garbage past the end of this one.
    #[test]
    fn tokenize_spans_clears_the_output_buffer() {
        let mut out = Vec::new();
        let mut block = false;
        tokenize_spans(
            "a very long line of identifiers",
            Language::Plain,
            &mut block,
            &mut out,
        );
        let long = out.len();
        tokenize_spans("x", Language::Plain, &mut block, &mut out);
        assert!(out.len() < long);
        assert_eq!(
            out,
            vec![Span {
                start: 0,
                end: 1,
                kind: TokenKind::Ident
            }]
        );
    }

    // ------------------------------------------------------------------
    // New languages
    // ------------------------------------------------------------------

    #[test]
    fn json_colours_literals_strings_and_numbers() {
        let toks = spans("{\"n\": 12, \"ok\": true}", Language::Json);
        assert!(has(&toks, "\"n\"", TokenKind::Str));
        assert!(has(&toks, "12", TokenKind::Number));
        assert!(has(&toks, "true", TokenKind::Keyword));
        // JSON has no comments: `//` must stay punctuation.
        let toks = spans("// nope", Language::Json);
        assert!(toks.iter().all(|(_, k)| *k != TokenKind::Comment));
    }

    #[test]
    fn toml_and_yaml_hash_comments_and_both_quote_styles() {
        let toks = spans("name = 'x'  # why", Language::Toml);
        assert!(has(&toks, "'x'", TokenKind::Str));
        assert!(has(&toks, "# why", TokenKind::Comment));
        let toks = spans("key: \"v\" # why", Language::Yaml);
        assert!(has(&toks, "\"v\"", TokenKind::Str));
        assert!(has(&toks, "# why", TokenKind::Comment));
        assert!(has(&toks, "key", TokenKind::Ident));
    }

    /// The regression this guards: in YAML and shell a `#` that is not at a
    /// token boundary is not a comment. Without the boundary rule, a CI config
    /// URL and `${#arr[@]}` paint half the line as a comment.
    #[test]
    fn hash_mid_token_is_not_a_comment_in_yaml_or_shell() {
        let toks = spans("url: https://example.com/#anchor", Language::Yaml);
        assert!(
            toks.iter().all(|(_, k)| *k != TokenKind::Comment),
            "yaml url got a comment: {toks:?}"
        );
        let toks = spans("n=${#arr[@]}", Language::Shell);
        assert!(
            toks.iter().all(|(_, k)| *k != TokenKind::Comment),
            "shell array length got a comment: {toks:?}"
        );
        // …but a real comment after whitespace still works.
        let toks = spans("echo hi  # real", Language::Shell);
        assert!(has(&toks, "# real", TokenKind::Comment));
    }

    #[test]
    fn markdown_heading_is_a_keyword_and_prose_is_plain() {
        let toks = spans("  ## Title `x`", Language::Markdown);
        assert_eq!(
            toks,
            vec![
                ("  ".to_string(), TokenKind::Ws),
                ("## Title `x`".to_string(), TokenKind::Keyword),
            ]
        );
        // Prose: no strings from apostrophes, no comments, no keywords.
        let toks = spans("it's fine // really /* yes", Language::Markdown);
        assert!(toks
            .iter()
            .all(|(_, k)| matches!(k, TokenKind::Ident | TokenKind::Punct | TokenKind::Ws)));
    }

    #[test]
    fn shell_keywords_and_strings() {
        let toks = spans("for f in \"$dir\"; do done", Language::Shell);
        assert!(has(&toks, "for", TokenKind::Keyword));
        assert!(has(&toks, "in", TokenKind::Keyword));
        assert!(has(&toks, "\"$dir\"", TokenKind::Str));
        assert!(has(&toks, "done", TokenKind::Keyword));
    }

    #[test]
    fn powershell_sigils_block_comments_and_case_insensitive_keywords() {
        let toks = spans(
            "if ($x -eq $true) { Write-Host 'hi' }",
            Language::PowerShell,
        );
        assert!(has(&toks, "if", TokenKind::Keyword));
        assert!(has(&toks, "$true", TokenKind::Keyword));
        assert!(has(&toks, "$x", TokenKind::Ident));
        assert!(has(&toks, "'hi'", TokenKind::Str));
        // Case-insensitive: `IF` and `$True` are the same keywords.
        let toks = spans("IF ($True) {}", Language::PowerShell);
        assert!(has(&toks, "IF", TokenKind::Keyword));
        assert!(has(&toks, "$True", TokenKind::Keyword));
        // `<# … #>` beats the `#` line comment.
        let mut block = false;
        let mut out = Vec::new();
        tokenize_spans("<# doc", Language::PowerShell, &mut block, &mut out);
        assert!(block, "block comment should stay open");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, TokenKind::Comment);
        tokenize_spans("more #> $x", Language::PowerShell, &mut block, &mut out);
        assert!(!block);
        assert_eq!(out[0].kind, TokenKind::Comment);
    }

    #[test]
    fn c_family_keywords_and_char_literals() {
        let toks = spans("int n = 'a'; /* c */", Language::C);
        assert!(has(&toks, "int", TokenKind::Keyword));
        assert!(has(&toks, "'a'", TokenKind::Str));
        assert!(has(&toks, "/* c */", TokenKind::Comment));
        // C++ sees both its own additions and C's table; C does not see C++'s.
        let toks = spans("class Foo : public Bar { bool b; }", Language::Cpp);
        assert!(has(&toks, "class", TokenKind::Keyword));
        assert!(has(&toks, "public", TokenKind::Keyword));
        assert!(has(&toks, "bool", TokenKind::Keyword));
        let toks = spans("class Foo", Language::C);
        assert!(has(&toks, "class", TokenKind::Ident));
    }

    #[test]
    fn java_kotlin_swift_ruby_php() {
        let toks = spans("public class A { int x = 1; }", Language::Java);
        assert!(has(&toks, "public", TokenKind::Keyword));
        assert!(has(&toks, "A", TokenKind::Ident));
        let toks = spans("fun f(): Int = 1 // k", Language::Kotlin);
        assert!(has(&toks, "fun", TokenKind::Keyword));
        assert!(has(&toks, "// k", TokenKind::Comment));
        let toks = spans("guard let x = y else { return }", Language::Swift);
        assert!(has(&toks, "guard", TokenKind::Keyword));
        assert!(has(&toks, "let", TokenKind::Keyword));
        let toks = spans("def run; @x = 'v'; end # r", Language::Ruby);
        assert!(has(&toks, "def", TokenKind::Keyword));
        assert!(has(&toks, "'v'", TokenKind::Str));
        assert!(has(&toks, "# r", TokenKind::Comment));
        let toks = spans("function f() { return $a; } // p", Language::Php);
        assert!(has(&toks, "function", TokenKind::Keyword));
        assert!(has(&toks, "$a", TokenKind::Ident));
        assert!(has(&toks, "// p", TokenKind::Comment));
    }

    #[test]
    fn sql_is_case_insensitive_and_uses_double_dash_comments() {
        let toks = spans("SELECT id FROM t WHERE n = 1 -- why", Language::Sql);
        assert!(has(&toks, "SELECT", TokenKind::Keyword));
        assert!(has(&toks, "from", TokenKind::Ident) || has(&toks, "FROM", TokenKind::Keyword));
        assert!(has(&toks, "id", TokenKind::Ident));
        assert!(has(&toks, "1", TokenKind::Number));
        assert!(has(&toks, "-- why", TokenKind::Comment));
        let toks = spans("select 'a' from t", Language::Sql);
        assert!(has(&toks, "select", TokenKind::Keyword));
        assert!(has(&toks, "'a'", TokenKind::Str));
    }

    #[test]
    fn css_has_only_block_comments_and_hyphenated_idents() {
        let toks = spans("a { font-face: x; } /* c */", Language::Css);
        assert!(has(&toks, "font-face", TokenKind::Keyword));
        assert!(has(&toks, "/* c */", TokenKind::Comment));
        // No line comment in CSS: `//` must stay punctuation.
        let toks = spans("// not a comment", Language::Css);
        assert!(toks.iter().all(|(_, k)| *k != TokenKind::Comment));
    }

    #[test]
    fn lua_block_comment_beats_its_line_comment() {
        let toks = spans("local a = 1 --[[ inline ]] + 2", Language::Lua);
        assert!(has(&toks, "local", TokenKind::Keyword));
        assert!(has(&toks, "--[[ inline ]]", TokenKind::Comment));
        assert!(has(&toks, "2", TokenKind::Number));
        let toks = spans("local a -- tail", Language::Lua);
        assert!(has(&toks, "-- tail", TokenKind::Comment));
    }

    #[test]
    fn zig_has_line_comments_but_no_block_comments() {
        let toks = spans("const x = try f(); // z", Language::Zig);
        assert!(has(&toks, "const", TokenKind::Keyword));
        assert!(has(&toks, "try", TokenKind::Keyword));
        assert!(has(&toks, "// z", TokenKind::Comment));
        // `/*` is not a comment opener in Zig; it must not swallow the line.
        let mut block = false;
        let mut out = Vec::new();
        tokenize_spans("a /* b", Language::Zig, &mut block, &mut out);
        assert!(!block, "zig must never enter a block comment");
    }

    #[test]
    fn dockerfile_and_makefile_by_basename() {
        let toks = spans("FROM rust:1 AS build # c", Language::Dockerfile);
        assert!(has(&toks, "FROM", TokenKind::Keyword));
        assert!(has(&toks, "AS", TokenKind::Keyword));
        assert!(has(&toks, "# c", TokenKind::Comment));
        let toks = spans("ifeq ($(OS),Windows_NT) # c", Language::Makefile);
        assert!(has(&toks, "ifeq", TokenKind::Keyword));
        assert!(has(&toks, "# c", TokenKind::Comment));
    }

    /// The toggle-comment command and the scanner must agree, or `Ctrl+/`
    /// would insert a prefix the colouriser does not treat as a comment.
    #[test]
    fn line_comment_prefix_matches_the_spec_table() {
        for lang in [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Go,
            Language::CSharp,
            Language::C,
            Language::Cpp,
            Language::Java,
            Language::Kotlin,
            Language::Swift,
            Language::Php,
            Language::Zig,
        ] {
            assert_eq!(lang.line_comment_prefix(), Some("//"), "{lang:?}");
        }
        for lang in [
            Language::Python,
            Language::Shell,
            Language::PowerShell,
            Language::Ruby,
            Language::Yaml,
            Language::Toml,
            Language::Makefile,
            Language::Dockerfile,
        ] {
            assert_eq!(lang.line_comment_prefix(), Some("#"), "{lang:?}");
        }
        for lang in [Language::Sql, Language::Lua] {
            assert_eq!(lang.line_comment_prefix(), Some("--"), "{lang:?}");
        }
        for lang in [
            Language::Json,
            Language::Css,
            Language::Markdown,
            Language::Plain,
        ] {
            assert_eq!(lang.line_comment_prefix(), None, "{lang:?}");
        }
    }

    /// `as_str` feeds telemetry and the snippet DOM; a duplicate would make
    /// two languages indistinguishable in a log.
    #[test]
    fn as_str_is_unique_per_language() {
        let all = [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::CSharp,
            Language::Json,
            Language::Toml,
            Language::Yaml,
            Language::Markdown,
            Language::Shell,
            Language::PowerShell,
            Language::C,
            Language::Cpp,
            Language::Java,
            Language::Kotlin,
            Language::Swift,
            Language::Ruby,
            Language::Php,
            Language::Sql,
            Language::Css,
            Language::Lua,
            Language::Zig,
            Language::Dockerfile,
            Language::Makefile,
            Language::Plain,
        ];
        let mut names: Vec<&str> = all.iter().map(|l| l.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate Language::as_str");
    }
}
