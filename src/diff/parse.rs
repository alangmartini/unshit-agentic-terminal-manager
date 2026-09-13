//! Hand-rolled unified-diff parser.
//!
//! Why hand-rolled: the spec forbids new crate dependencies, and the shape we
//! need is not "a patch to apply" but "a buffer the editor pane can render".
//! So the output is deliberately *flat* — one text line plus one
//! [`DiffRowInfo`] per rendered row — instead of a tree of files → hunks →
//! lines. The pane then reuses every editor mechanism (viewport, selection,
//! find, copy) unchanged and only consults `rows[i]` to decide how to tint
//! row `i` and what to put in the gutter.
//!
//! Two invariants hold for every document this module produces, and the
//! renderer depends on both:
//!
//! 1. `lines.len() == rows.len()`.
//! 2. Content rows (`Context`/`Added`/`Removed`) carry the source text
//!    *without* git's leading `+`/`-`/space marker, so syntax highlighting
//!    and copy-to-clipboard see real code. The marker is drawn from
//!    [`DiffRowKind`] instead.
//!
//! The parser is a small state machine rather than a per-line `match` on
//! prefixes, because the same bytes mean different things depending on where
//! they appear: `--- a/x` is a header line at the top of a file section but a
//! *removed line* whose text is `-- a/x` inside a hunk body. Garbage in never
//! panics; the worst case is a document with zero files, which the caller
//! renders as "no changes".

use crate::syntax::Language;

/// Row budget for one document. A diff this large is already unreviewable;
/// the cap exists so a `git diff` of a vendored tree cannot take the UI
/// thread down when the result is applied. Chosen in the spec (§4).
pub const MAX_DIFF_ROWS: usize = 200_000;

/// Longest line we keep verbatim. Minified bundles and generated lock files
/// routinely carry multi-megabyte single lines; the editor's per-line layout
/// is O(line length), so one such line would stall a frame.
const MAX_LINE_CHARS: usize = 4096;

/// Appended to a line clipped by [`MAX_LINE_CHARS`] so the truncation is
/// visible rather than silently changing the content.
const CLAMP_SUFFIX: &str = " …";

/// Text of the trailing [`DiffRowKind::Meta`] row added when the row budget
/// runs out.
const TRUNCATION_NOTICE: &str = "… diff truncated: too many rows to display";

/// Text of the [`DiffRowKind::Meta`] row appended when git wrote more than
/// the stdout cap. Separate from [`TRUNCATION_NOTICE`] because the cause
/// is different: the row budget was fine, git's output was not.
const OUTPUT_CUT_NOTICE: &str =
    "… diff truncated: git produced more output than can be shown";

/// Text of the [`DiffRowKind::Meta`] row that stands in for a combined
/// (merge) diff. We refuse to parse `@@@` hunks rather than mis-attribute
/// their two marker columns to the wrong side.
const COMBINED_NOTICE: &str = "combined diff (merge commit) is not shown";

/// Text of the [`DiffRowKind::Meta`] row for a binary file. Normalised
/// instead of echoing git's `Binary files a/x and b/y differ`, because the
/// `FileHeader` row directly above already carries the path.
const BINARY_NOTICE: &str = "Binary files differ";

/// What a rendered row *is*, which decides its tint, its gutter and whether
/// its text is source code worth syntax-highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRowKind {
    /// `M src/state.rs`, or `R old/path -> new/path` for a rename.
    FileHeader,
    /// git's `@@ -a,b +c,d @@ section` line, verbatim.
    HunkHeader,
    /// Unchanged source line; present on both sides.
    Context,
    /// Source line present only in the new file.
    Added,
    /// Source line present only in the old file.
    Removed,
    /// `\ No newline at end of file`, binary/combined notices, truncation.
    Meta,
    /// Blank separator between two file sections.
    Spacer,
}

impl DiffRowKind {
    /// The gutter marker column for this row. Header/meta rows have none, so
    /// the gutter stays blank there and the file header reads as a heading.
    pub fn marker(self) -> char {
        match self {
            DiffRowKind::Added => '+',
            DiffRowKind::Removed => '-',
            _ => ' ',
        }
    }

    /// True when the row's text is source code from the file, i.e. the only
    /// rows the syntax tokenizer should run over.
    pub fn is_content(self) -> bool {
        matches!(
            self,
            DiffRowKind::Context | DiffRowKind::Added | DiffRowKind::Removed
        )
    }
}

/// Per-file verdict, used for the header letter and (eventually) an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    /// Binary payloads are never rendered; the file gets a header row and a
    /// single [`DiffRowKind::Meta`] row.
    Binary,
}

impl DiffFileStatus {
    /// Single-letter prefix on the file header row. `B` for binary is our
    /// own (git's `--name-status` has no binary letter) but reads naturally
    /// next to A/D/M/R.
    pub fn letter(self) -> char {
        match self {
            DiffFileStatus::Added => 'A',
            DiffFileStatus::Deleted => 'D',
            DiffFileStatus::Modified => 'M',
            DiffFileStatus::Renamed => 'R',
            DiffFileStatus::Binary => 'B',
        }
    }
}

/// Decoration for one rendered row. Deliberately `Copy` and 16 bytes-ish:
/// there is one of these per line of a 200 000-row document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRowInfo {
    pub kind: DiffRowKind,
    /// Index into [`DiffDocument::files`]. Never optional: separator and
    /// notice rows are attributed to the file they belong to (or the
    /// preceding one, for a spacer), so `]`/`[` navigation from any row has
    /// a well-defined current file.
    pub file: u32,
    /// 1-based line number in the old file, when the row exists there.
    pub old_no: Option<u32>,
    /// 1-based line number in the new file, when the row exists there.
    pub new_no: Option<u32>,
}

/// One file section, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFileInfo {
    /// The new path, or the old path for a deletion — i.e. the path the user
    /// would open. `diff.open_file` uses exactly this.
    pub path: String,
    /// Only set when it differs from `path` (renames), so the UI can show
    /// `old -> new` without re-deriving it.
    pub old_path: Option<String>,
    pub status: DiffFileStatus,
    /// Row index of this file's `FileHeader`. `]`/`[` jump here.
    pub first_row: usize,
    /// Row index of every `HunkHeader` in this file, ascending. `n`/`p`
    /// binary-search this.
    pub hunk_rows: Vec<usize>,
    /// Language for `path`, resolved once so the renderer does not re-derive
    /// it per visible row.
    pub lang: Language,
}

/// The whole review buffer for one git range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffDocument {
    /// Buffer text, one entry per row. Never contains `\n` or `\r`.
    pub lines: Vec<String>,
    /// Decorations, parallel to `lines` (`lines.len() == rows.len()`).
    pub rows: Vec<DiffRowInfo>,
    pub files: Vec<DiffFileInfo>,
    /// True when the row budget ran out; the last row is then a `Meta` row
    /// saying so. Reported in the `diff.ready` telemetry event.
    pub truncated: bool,
}

impl DiffDocument {
    /// Append the "git wrote more than we read" row.
    ///
    /// The row budget has its own notice; this one is for the byte cap,
    /// which used to be reported to telemetry and nowhere else, so a diff
    /// whose tail git never got to write rendered as a complete one.
    /// Dropped when there is no file at all, so `rows[i].file` stays a
    /// valid index into `files`.
    pub fn mark_output_truncated(&mut self) {
        self.truncated = true;
        if self.files.is_empty() {
            return;
        }
        let file = self
            .rows
            .last()
            .map(|row| row.file)
            .unwrap_or(0)
            .min(self.files.len() as u32 - 1);
        self.lines.push(OUTPUT_CUT_NOTICE.to_string());
        self.rows.push(DiffRowInfo {
            kind: DiffRowKind::Meta,
            file,
            old_no: None,
            new_no: None,
        });
    }

    /// Total hunks across all files — the `hunks` field of `diff.ready`.
    pub fn hunk_count(&self) -> usize {
        self.files.iter().map(|f| f.hunk_rows.len()).sum()
    }

    /// True when git reported nothing; the caller shows "No changes for
    /// <range>" instead of opening an empty buffer.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Parse the stdout of
/// `git diff --no-color --no-ext-diff --find-renames -U3` into a review
/// document. Never fails: unparseable input yields a document with no files.
pub fn parse_unified_diff(input: &str) -> DiffDocument {
    parse_unified_diff_with_cap(input, MAX_DIFF_ROWS)
}

/// [`parse_unified_diff`] with an explicit row budget. Exists so the
/// truncation path is testable without building a 200 000-row fixture; the
/// app always uses [`MAX_DIFF_ROWS`].
pub(crate) fn parse_unified_diff_with_cap(input: &str, cap: usize) -> DiffDocument {
    let mut doc = Builder::new(cap);
    let mut cur: Option<FileCtx> = None;
    let mut mode = Mode::Outside;

    for raw in input.lines() {
        // git on Windows can hand us CRLF even with --no-color; `str::lines`
        // already strips `\r\n`, this catches a lone `\r` at the end of a
        // line that was split on a bare `\n`.
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        if doc.truncated {
            break;
        }

        // A file section header is unambiguous: every line inside a hunk
        // body carries a marker prefix, so an unprefixed `diff --git` at
        // column 0 is never source text.
        if let Some(start) = file_start(line) {
            doc.finish_file(cur.take());
            match doc.begin_file(start) {
                Some(ctx) => {
                    cur = Some(ctx);
                    mode = Mode::Header;
                }
                // Budget exhausted mid-document: stop before we record a
                // file whose header row does not exist.
                None => break,
            }
            continue;
        }

        let Some(ctx) = cur.as_mut() else {
            // Anything before the first `diff --git` (a `commit` header from
            // `git show`, a stray blank line, outright garbage) is skipped.
            continue;
        };

        // Binary payloads and combined-diff bodies are skipped wholesale
        // until the next file section.
        if mode == Mode::Skip {
            continue;
        }

        // `Mode` is `Copy`, so the remaining counts are edited on local
        // copies and written back once: taking `ref mut` here would keep
        // `mode` borrowed across the `mode = Mode::Header` fallthrough.
        if let Mode::Hunk { old_rem, new_rem } = mode {
            let (mut old_rem, mut new_rem) = (old_rem, new_rem);
            let mut in_body = true;

            // `\ No newline at end of file` can arrive *after* the counted
            // lines are exhausted (it follows the last `+` line), so it is
            // accepted regardless of the remaining counts and consumes
            // neither a count nor a line number.
            if line.starts_with('\\') {
                doc.push(DiffRowKind::Meta, ctx.index, None, None, line.to_string());
            } else if old_rem > 0 || new_rem > 0 {
                match body_row(line) {
                    Some((kind, text)) => {
                        let (old_no, new_no) = match kind {
                            DiffRowKind::Context => {
                                let pair = (Some(ctx.old_no), Some(ctx.new_no));
                                ctx.old_no += 1;
                                ctx.new_no += 1;
                                old_rem = old_rem.saturating_sub(1);
                                new_rem = new_rem.saturating_sub(1);
                                pair
                            }
                            DiffRowKind::Added => {
                                let pair = (None, Some(ctx.new_no));
                                ctx.new_no += 1;
                                new_rem = new_rem.saturating_sub(1);
                                pair
                            }
                            _ => {
                                let pair = (Some(ctx.old_no), None);
                                ctx.old_no += 1;
                                old_rem = old_rem.saturating_sub(1);
                                pair
                            }
                        };
                        doc.push(kind, ctx.index, old_no, new_no, text);
                    }
                    // A line with no marker inside a counted hunk means the
                    // hunk header lied (truncated output, or a tool that
                    // reflowed the patch). End the hunk and let the line be
                    // reconsidered as a header below.
                    None => in_body = false,
                }
            } else {
                in_body = false;
            }

            if in_body {
                mode = Mode::Hunk { old_rem, new_rem };
                continue;
            }
            mode = Mode::Header;
        }

        if line.starts_with("@@") {
            // `@@@` marks a combined (merge) diff; so does any hunk inside a
            // `diff --cc` section. Two marker columns would make every row
            // ambiguous, so we degrade the whole file to one notice.
            if ctx.combined || line.starts_with("@@@") {
                doc.push(
                    DiffRowKind::Meta,
                    ctx.index,
                    None,
                    None,
                    COMBINED_NOTICE.to_string(),
                );
                mode = Mode::Skip;
                continue;
            }
            if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(line) {
                ctx.old_no = old_start;
                ctx.new_no = new_start;
                let row = doc.lines.len();
                if doc.push(
                    DiffRowKind::HunkHeader,
                    ctx.index,
                    None,
                    None,
                    line.to_string(),
                ) {
                    doc.files[ctx.index as usize].hunk_rows.push(row);
                }
                mode = Mode::Hunk {
                    old_rem: old_count,
                    new_rem: new_count,
                };
            }
            // A malformed `@@` line is dropped: better a missing hunk than a
            // body parsed against bogus line numbers.
            continue;
        }

        // Extended header lines. Only reachable in `Header` mode, which is
        // why `--- a/x` here is a path and not a removed line.
        header_line(ctx, line, &mut doc, &mut mode);
    }

    doc.finish_file(cur.take());
    doc.into_document()
}

/// Where the line-by-line loop currently is inside a file section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Before the first `diff --git`.
    Outside,
    /// Inside a file section, before/between hunks: `index`, `--- `, `+++ `,
    /// `rename from`, … all mean what they say here.
    Header,
    /// Inside a hunk body, with the number of old/new lines git still owes
    /// us. Both reaching zero ends the hunk.
    Hunk { old_rem: u32, new_rem: u32 },
    /// Swallowing a binary payload or a combined-diff body until the next
    /// file section.
    Skip,
}

/// Mutable facts about the file section being parsed. The `DiffFileInfo` is
/// pushed (with placeholder path/status) as soon as the section starts, so
/// `first_row` is recorded before anything else; the identity is only known
/// once the header lines have all arrived, and is patched in by
/// [`Builder::finish_file`].
struct FileCtx {
    index: u32,
    /// Paths from the `diff --git a/X b/Y` line — the fallback for files
    /// with no `---`/`+++` (pure renames, mode changes, binaries).
    git_old: Option<String>,
    git_new: Option<String>,
    /// `---` side: `None` when absent, `Some(None)` for `/dev/null`.
    minus: Option<Option<String>>,
    /// `+++` side, same encoding as `minus`.
    plus: Option<Option<String>>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    is_rename: bool,
    is_new: bool,
    is_deleted: bool,
    is_binary: bool,
    /// Set for `diff --cc` / `diff --combined` sections.
    combined: bool,
    /// Next 1-based line numbers inside the current hunk.
    old_no: u32,
    new_no: u32,
}

/// Recognised forms of a file-section header line.
enum FileStart {
    /// `diff --git a/X b/Y`
    Git(String),
    /// `diff --cc X` / `diff --combined X`
    Combined(String),
}

/// Detect the start of a file section. Returns the text after the marker.
fn file_start(line: &str) -> Option<FileStart> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        return Some(FileStart::Git(rest.to_string()));
    }
    if let Some(rest) = line.strip_prefix("diff --cc ") {
        return Some(FileStart::Combined(rest.to_string()));
    }
    if let Some(rest) = line.strip_prefix("diff --combined ") {
        return Some(FileStart::Combined(rest.to_string()));
    }
    None
}

/// Classify a hunk-body line and strip its marker. `None` means the line has
/// no marker at all and therefore is not part of the body.
fn body_row(line: &str) -> Option<(DiffRowKind, String)> {
    // git emits a bare empty line for an unchanged empty source line when
    // the trailing space was stripped somewhere in transit (mail, a copied
    // patch). Treating it as an empty context line keeps the hunk aligned.
    if line.is_empty() {
        return Some((DiffRowKind::Context, String::new()));
    }
    let mut chars = line.chars();
    let marker = chars.next()?;
    let text = chars.as_str().to_string();
    match marker {
        ' ' => Some((DiffRowKind::Context, text)),
        '+' => Some((DiffRowKind::Added, text)),
        '-' => Some((DiffRowKind::Removed, text)),
        _ => None,
    }
}

/// Apply one extended-header line to the file context.
fn header_line(ctx: &mut FileCtx, line: &str, doc: &mut Builder, mode: &mut Mode) {
    if let Some(rest) = line.strip_prefix("--- ") {
        ctx.minus = Some(path_side(rest));
    } else if let Some(rest) = line.strip_prefix("+++ ") {
        ctx.plus = Some(path_side(rest));
    } else if let Some(rest) = line.strip_prefix("rename from ") {
        ctx.is_rename = true;
        ctx.rename_from = Some(unquote_path(rest));
    } else if let Some(rest) = line.strip_prefix("rename to ") {
        ctx.is_rename = true;
        ctx.rename_to = Some(unquote_path(rest));
    } else if line.starts_with("new file mode ") {
        ctx.is_new = true;
    } else if line.starts_with("deleted file mode ") {
        ctx.is_deleted = true;
    } else if line.starts_with("Binary files ") || line.starts_with("Files ") {
        ctx.is_binary = true;
        doc.push(
            DiffRowKind::Meta,
            ctx.index,
            None,
            None,
            BINARY_NOTICE.to_string(),
        );
    } else if line.starts_with("GIT binary patch") {
        // The payload that follows is base85; skipping it is the whole point
        // of recognising this line.
        ctx.is_binary = true;
        doc.push(
            DiffRowKind::Meta,
            ctx.index,
            None,
            None,
            BINARY_NOTICE.to_string(),
        );
        *mode = Mode::Skip;
    }
    // Everything else (`index`, `old mode`, `new mode`, `similarity index`,
    // `dissimilarity index`, `copy from`/`copy to` — we never pass `-C` —
    // and any future header) carries no information this document shows, so
    // it is ignored rather than rendered as noise.
}

/// Decode one `---`/`+++` operand: `None` for `/dev/null`, otherwise the
/// path with its `a/`-style prefix stripped.
fn path_side(rest: &str) -> Option<String> {
    // git appends a tab plus a timestamp in some patch dialects; a real tab
    // inside a filename would have been C-quoted, so cutting at the first
    // literal tab is safe.
    let rest = rest.split('\t').next().unwrap_or(rest);
    if rest == "/dev/null" {
        return None;
    }
    Some(strip_side_prefix(&unquote_path(rest)))
}

/// Split `a/X b/Y` from a `diff --git` line into its two operands.
///
/// git does not quote spaces, so this line is genuinely ambiguous for a path
/// containing one — git's own parser resolves it from the `---`/`+++` lines,
/// and so do we; this is only the fallback for the files that have none
/// (binary, mode-only). Those never rename, so both operands name the same
/// path: splitting down the middle and checking the halves agree resolves
/// every case we can actually hit, and the ` b/` scan catches the rest.
fn split_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    let bytes = rest.as_bytes();
    if bytes.len() % 2 == 1 {
        let mid = bytes.len() / 2;
        // An ASCII space is always a char boundary, so slicing here is safe.
        if bytes[mid] == b' ' {
            let old = strip_side_prefix(&unquote_path(&rest[..mid]));
            let new = strip_side_prefix(&unquote_path(&rest[mid + 1..]));
            if old == new {
                return (Some(old), Some(new));
            }
        }
    }
    if let Some(idx) = rest.rfind(" b/") {
        let old = &rest[..idx];
        let new = &rest[idx + 1..];
        return (
            Some(strip_side_prefix(&unquote_path(old))),
            Some(strip_side_prefix(&unquote_path(new))),
        );
    }
    // `--no-prefix`, an exotic `diff.mnemonicPrefix`, or a path we cannot
    // split: fall back to the last space, and accept being wrong for a
    // filename with a space (the `---`/`+++` lines usually rescue us).
    match rest.rsplit_once(' ') {
        Some((old, new)) => (
            Some(strip_side_prefix(&unquote_path(old))),
            Some(strip_side_prefix(&unquote_path(new))),
        ),
        None => (None, Some(strip_side_prefix(&unquote_path(rest)))),
    }
}

/// Drop git's one-letter side prefix. Only the letters git actually emits
/// are accepted (`a`/`b` by default, `i`/`w`/`c`/`o` under
/// `diff.mnemonicPrefix`) so a genuine top-level directory named `x` is not
/// silently eaten.
fn strip_side_prefix(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() > 2
        && bytes[1] == b'/'
        && matches!(bytes[0], b'a' | b'b' | b'i' | b'w' | b'c' | b'o')
    {
        return path[2..].to_string();
    }
    path.to_string()
}

/// Undo git's C-style quoting (`core.quotepath`). We pass
/// `-c core.quotepath=false`, so this is a robustness net rather than the
/// common path; anything we cannot decode is kept verbatim, because showing
/// an escaped path beats showing none.
fn unquote_path(raw: &str) -> String {
    let inner = raw
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'));
    match inner.and_then(decode_c_quoted) {
        Some(decoded) => decoded,
        None => raw.to_string(),
    }
}

/// Decode the body of a C-quoted string into UTF-8. `None` when an escape is
/// unknown or the resulting bytes are not UTF-8 (git quotes byte-wise, so a
/// path in a non-UTF-8 encoding genuinely cannot be recovered here).
fn decode_c_quoted(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        i += 1;
        let escape = *bytes.get(i)?;
        match escape {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'0'..=b'7' => {
                // git always emits exactly three octal digits.
                let mut value: u32 = 0;
                for offset in 0..3 {
                    let digit = *bytes.get(i + offset)?;
                    if !matches!(digit, b'0'..=b'7') {
                        return None;
                    }
                    value = value * 8 + u32::from(digit - b'0');
                }
                if value > 0xff {
                    return None;
                }
                out.push(value as u8);
                i += 2; // the loop below advances past the third digit
            }
            _ => return None,
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Parse `@@ -a[,b] +c[,d] @@[ section]`. A missing count means 1, per the
/// unified-diff format.
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let mut parts = rest[..end].split_whitespace();
    let (old_start, old_count) = parse_range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_count) = parse_range(parts.next()?.strip_prefix('+')?)?;
    Some((old_start, old_count, new_start, new_count))
}

/// `start[,count]` with count defaulting to 1.
fn parse_range(text: &str) -> Option<(u32, u32)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// Clip a line that would blow up the editor's per-line layout, marking the
/// clip so the text is not silently misrepresented.
fn clamp_line(text: String) -> String {
    // Bytes are an upper bound on chars, so short lines skip the scan.
    if text.len() <= MAX_LINE_CHARS {
        return text;
    }
    match text.char_indices().nth(MAX_LINE_CHARS) {
        Some((idx, _)) => {
            let mut clipped = text[..idx].to_string();
            clipped.push_str(CLAMP_SUFFIX);
            clipped
        }
        None => text,
    }
}

/// Accumulates the document and owns the row budget.
struct Builder {
    cap: usize,
    lines: Vec<String>,
    rows: Vec<DiffRowInfo>,
    files: Vec<DiffFileInfo>,
    truncated: bool,
}

impl Builder {
    fn new(cap: usize) -> Builder {
        Builder {
            cap,
            lines: Vec::new(),
            rows: Vec::new(),
            files: Vec::new(),
            truncated: false,
        }
    }

    /// Push one row. Returns false once the budget is spent, at which point
    /// the trailing truncation notice has already been written and the
    /// caller should stop parsing.
    fn push(
        &mut self,
        kind: DiffRowKind,
        file: u32,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: String,
    ) -> bool {
        if self.truncated {
            return false;
        }
        // One slot is always reserved for the truncation notice, so the
        // final document never exceeds `cap`.
        if self.lines.len() + 1 >= self.cap {
            self.mark_truncated(file);
            return false;
        }
        self.lines.push(clamp_line(text));
        self.rows.push(DiffRowInfo {
            kind,
            file,
            old_no,
            new_no,
        });
        true
    }

    fn mark_truncated(&mut self, file: u32) {
        if self.truncated {
            return;
        }
        self.truncated = true;
        // The budget can run out while opening a file section, i.e. before
        // that file's `DiffFileInfo` exists. Attribute the notice to the
        // last file that does exist, and drop it entirely when there is no
        // file at all, so `rows[i].file` is *always* a valid index into
        // `files` and the caller can subscript it without a guard.
        if self.files.is_empty() {
            return;
        }
        let file = file.min(self.files.len() as u32 - 1);
        if self.lines.len() < self.cap {
            self.lines.push(TRUNCATION_NOTICE.to_string());
            self.rows.push(DiffRowInfo {
                kind: DiffRowKind::Meta,
                file,
                old_no: None,
                new_no: None,
            });
        }
    }

    /// Open a file section: blank separator, placeholder header row, and the
    /// `DiffFileInfo` whose identity is filled in by [`Builder::finish_file`].
    /// `None` when the budget ran out.
    fn begin_file(&mut self, start: FileStart) -> Option<FileCtx> {
        let index = self.files.len() as u32;
        if !self.rows.is_empty() {
            let previous = index.saturating_sub(1);
            if !self.push(DiffRowKind::Spacer, previous, None, None, String::new()) {
                return None;
            }
        }
        let first_row = self.lines.len();
        if !self.push(DiffRowKind::FileHeader, index, None, None, String::new()) {
            return None;
        }
        self.files.push(DiffFileInfo {
            path: String::new(),
            old_path: None,
            status: DiffFileStatus::Modified,
            first_row,
            hunk_rows: Vec::new(),
            lang: Language::Plain,
        });
        let (combined, git_old, git_new) = match start {
            FileStart::Git(rest) => {
                let (old, new) = split_git_paths(&rest);
                (false, old, new)
            }
            // `diff --cc <path>` carries no `a/`-style prefix, so stripping
            // one here would eat a real top-level directory named `a`…`o`.
            FileStart::Combined(rest) => (true, None, Some(unquote_path(&rest))),
        };
        Some(FileCtx {
            index,
            git_old,
            git_new,
            minus: None,
            plus: None,
            rename_from: None,
            rename_to: None,
            is_rename: false,
            is_new: false,
            is_deleted: false,
            is_binary: false,
            combined,
            old_no: 1,
            new_no: 1,
        })
    }

    /// Resolve a finished file section's identity and rewrite its header row.
    /// Deferred to the end of the section because the facts arrive in any
    /// order and some (a 100 %-similarity rename, a binary, a mode-only
    /// change) never produce `---`/`+++` lines at all.
    fn finish_file(&mut self, ctx: Option<FileCtx>) {
        let Some(ctx) = ctx else { return };
        let index = ctx.index as usize;
        let Some(info) = self.files.get_mut(index) else {
            return;
        };

        // Prefer the `---`/`+++` operands: they are the authoritative pair
        // and are unambiguous even for paths with spaces.
        let minus_devnull = matches!(ctx.minus, Some(None));
        let plus_devnull = matches!(ctx.plus, Some(None));
        let old_path = ctx
            .minus
            .clone()
            .flatten()
            .or(ctx.rename_from.clone())
            .or(ctx.git_old.clone());
        let new_path = ctx
            .plus
            .clone()
            .flatten()
            .or(ctx.rename_to.clone())
            .or(ctx.git_new.clone());

        // Binary wins because its row content is a notice either way; a
        // rename is next because a renamed file can also be modified.
        let status = if ctx.is_binary {
            DiffFileStatus::Binary
        } else if ctx.is_rename {
            DiffFileStatus::Renamed
        } else if ctx.is_new || minus_devnull {
            DiffFileStatus::Added
        } else if ctx.is_deleted || plus_devnull {
            DiffFileStatus::Deleted
        } else {
            DiffFileStatus::Modified
        };

        // A deletion has no new path; everything else opens the new one.
        let path = match status {
            DiffFileStatus::Deleted => old_path.clone().or(new_path.clone()),
            _ => new_path.clone().or(old_path.clone()),
        }
        .unwrap_or_default();

        let old_path = old_path.filter(|old| *old != path);
        let header = match (&old_path, status) {
            // Plain ASCII arrow on purpose: the gutter/monospace grid has no
            // guarantee about the width of `→`.
            (Some(old), DiffFileStatus::Renamed) => format!("R {old} -> {path}"),
            _ => format!("{} {}", status.letter(), path),
        };

        info.lang = Language::from_path(&path);
        info.path = path;
        info.old_path = old_path;
        info.status = status;
        let first_row = info.first_row;
        if let Some(slot) = self.lines.get_mut(first_row) {
            *slot = clamp_line(header);
        }
    }

    fn into_document(self) -> DiffDocument {
        debug_assert_eq!(self.lines.len(), self.rows.len());
        DiffDocument {
            lines: self.lines,
            rows: self.rows,
            files: self.files,
            truncated: self.truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build diff text from one string per line. Fixtures are written as
    /// arrays rather than as one `\n\`-continued literal on purpose: the
    /// continuation escape swallows the following line's leading whitespace,
    /// which is exactly the byte that distinguishes a context line from a
    /// header line. An earlier draft of these tests was silently testing the
    /// wrong input because of it.
    fn fixture(lines: &[&str]) -> String {
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Re-encode a fixture the way git on Windows can hand it to us.
    fn crlf(input: &str) -> String {
        input.replace('\n', "\r\n")
    }

    fn kinds(doc: &DiffDocument) -> Vec<DiffRowKind> {
        doc.rows.iter().map(|r| r.kind).collect()
    }

    const SIMPLE: &[&str] = &[
        "diff --git a/src/state.rs b/src/state.rs",
        "index 1111111..2222222 100644",
        "--- a/src/state.rs",
        "+++ b/src/state.rs",
        "@@ -10,4 +10,5 @@ fn dispatch(cmd: &str) {",
        "     let x = 1;",
        "-    let y = 2;",
        "+    let y = 3;",
        "+    let z = 4;",
        "     done()",
    ];

    const MULTI: &[&str] = &[
        "diff --git a/one.rs b/one.rs",
        "--- a/one.rs",
        "+++ b/one.rs",
        "@@ -1,1 +1,1 @@",
        "-a",
        "+b",
        "diff --git a/two.ts b/two.ts",
        "--- a/two.ts",
        "+++ b/two.ts",
        "@@ -5,2 +5,2 @@",
        " keep",
        "-old",
        "+new",
        "@@ -20,1 +20,2 @@ tail",
        " ctx",
        "+more",
    ];

    // Pins the core shape: header row, verbatim hunk header, and content
    // rows whose text has lost the marker but kept its indentation.
    #[test]
    fn parses_a_simple_modification() {
        let doc = parse_unified_diff(&fixture(SIMPLE));
        assert_eq!(doc.files.len(), 1);
        assert_eq!(doc.files[0].path, "src/state.rs");
        assert_eq!(doc.files[0].old_path, None);
        assert_eq!(doc.files[0].status, DiffFileStatus::Modified);
        assert_eq!(doc.files[0].lang, Language::Rust);
        assert_eq!(doc.files[0].first_row, 0);
        assert_eq!(doc.files[0].hunk_rows, vec![1]);
        assert_eq!(
            kinds(&doc),
            vec![
                DiffRowKind::FileHeader,
                DiffRowKind::HunkHeader,
                DiffRowKind::Context,
                DiffRowKind::Removed,
                DiffRowKind::Added,
                DiffRowKind::Added,
                DiffRowKind::Context,
            ]
        );
        assert_eq!(doc.lines[0], "M src/state.rs");
        assert_eq!(doc.lines[1], "@@ -10,4 +10,5 @@ fn dispatch(cmd: &str) {");
        assert_eq!(doc.lines[2], "    let x = 1;");
        assert_eq!(doc.lines[3], "    let y = 2;");
        assert_eq!(doc.lines[4], "    let y = 3;");
        assert!(!doc.truncated);
        assert_eq!(doc.hunk_count(), 1);
        assert!(!doc.is_empty());
    }

    // `--- /dev/null` must read as Added, not as a modification of a file
    // literally called `/dev/null`.
    #[test]
    fn parses_a_file_addition() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/src/new.rs b/src/new.rs",
            "new file mode 100644",
            "index 0000000..3333333",
            "--- /dev/null",
            "+++ b/src/new.rs",
            "@@ -0,0 +1,2 @@",
            "+fn main() {}",
            "+// end",
        ]));
        assert_eq!(doc.files[0].status, DiffFileStatus::Added);
        assert_eq!(doc.files[0].path, "src/new.rs");
        assert_eq!(doc.files[0].old_path, None);
        assert_eq!(doc.lines[0], "A src/new.rs");
        // An added-only hunk leaves every old number empty.
        assert_eq!(doc.rows[2].old_no, None);
        assert_eq!(doc.rows[2].new_no, Some(1));
        assert_eq!(doc.rows[3].old_no, None);
        assert_eq!(doc.rows[3].new_no, Some(2));
    }

    // A deletion keeps the *old* path as `path`, because that is the file
    // the user would look for.
    #[test]
    fn parses_a_file_deletion() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/src/old.py b/src/old.py",
            "deleted file mode 100644",
            "index 3333333..0000000",
            "--- a/src/old.py",
            "+++ /dev/null",
            "@@ -1,2 +0,0 @@",
            "-import os",
            "-print(os)",
        ]));
        assert_eq!(doc.files[0].status, DiffFileStatus::Deleted);
        assert_eq!(doc.files[0].path, "src/old.py");
        assert_eq!(doc.files[0].old_path, None);
        assert_eq!(doc.files[0].lang, Language::Python);
        assert_eq!(doc.lines[0], "D src/old.py");
        assert_eq!(doc.rows[2].old_no, Some(1));
        assert_eq!(doc.rows[2].new_no, None);
        assert_eq!(doc.rows[3].old_no, Some(2));
    }

    // A 100 %-similarity rename has no `---`/`+++` and no hunks at all, so
    // the identity can only come from `rename from`/`rename to`.
    #[test]
    fn parses_a_pure_rename_without_content_changes() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/src/a.rs b/src/b.rs",
            "similarity index 100%",
            "rename from src/a.rs",
            "rename to src/b.rs",
        ]));
        assert_eq!(doc.files.len(), 1);
        assert_eq!(doc.files[0].status, DiffFileStatus::Renamed);
        assert_eq!(doc.files[0].path, "src/b.rs");
        assert_eq!(doc.files[0].old_path.as_deref(), Some("src/a.rs"));
        assert_eq!(doc.lines, vec!["R src/a.rs -> src/b.rs"]);
        assert_eq!(kinds(&doc), vec![DiffRowKind::FileHeader]);
    }

    // A rename *with* edits still renders as `R old -> new` and keeps its
    // hunks; the rename must win over "modified".
    #[test]
    fn parses_a_rename_with_content_changes() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/src/a.rs b/src/b.rs",
            "similarity index 84%",
            "rename from src/a.rs",
            "rename to src/b.rs",
            "index 1111111..2222222 100644",
            "--- a/src/a.rs",
            "+++ b/src/b.rs",
            "@@ -1,2 +1,2 @@",
            " fn a() {}",
            "-// old",
            "+// new",
        ]));
        assert_eq!(doc.files[0].status, DiffFileStatus::Renamed);
        assert_eq!(doc.files[0].path, "src/b.rs");
        assert_eq!(doc.files[0].old_path.as_deref(), Some("src/a.rs"));
        assert_eq!(doc.lines[0], "R src/a.rs -> src/b.rs");
        assert_eq!(doc.files[0].hunk_rows, vec![1]);
        assert_eq!(doc.rows[2].kind, DiffRowKind::Context);
        assert_eq!(doc.lines[2], "fn a() {}");
    }

    // Binary payloads are never rendered; one notice row stands in for them.
    #[test]
    fn parses_a_binary_file() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/assets/icon.png b/assets/icon.png",
            "index 1111111..2222222 100644",
            "Binary files a/assets/icon.png and b/assets/icon.png differ",
        ]));
        assert_eq!(doc.files[0].status, DiffFileStatus::Binary);
        assert_eq!(doc.files[0].path, "assets/icon.png");
        assert_eq!(doc.lines[0], "B assets/icon.png");
        assert_eq!(doc.lines[1], "Binary files differ");
        assert_eq!(
            kinds(&doc),
            vec![DiffRowKind::FileHeader, DiffRowKind::Meta]
        );
    }

    // `GIT binary patch` is followed by base85 payload lines that would
    // otherwise be parsed as hunk content for the *next* file.
    #[test]
    fn git_binary_patch_payload_is_skipped() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/a.bin b/a.bin",
            "index 1111111..2222222 100644",
            "GIT binary patch",
            "literal 12",
            "zcmZQzU|?WoBqB;+VBk9",
            "literal 0",
            "HcmV?d00001",
            "diff --git a/b.txt b/b.txt",
            "--- a/b.txt",
            "+++ b/b.txt",
            "@@ -1 +1 @@",
            "-x",
            "+y",
        ]));
        assert_eq!(doc.files.len(), 2);
        assert_eq!(doc.files[0].status, DiffFileStatus::Binary);
        assert_eq!(
            kinds(&doc),
            vec![
                DiffRowKind::FileHeader,
                DiffRowKind::Meta,
                DiffRowKind::Spacer,
                DiffRowKind::FileHeader,
                DiffRowKind::HunkHeader,
                DiffRowKind::Removed,
                DiffRowKind::Added,
            ]
        );
        assert_eq!(doc.files[1].first_row, 3);
        assert_eq!(doc.files[1].status, DiffFileStatus::Modified);
    }

    // File indices, `first_row`, the spacer between sections and per-file
    // `hunk_rows` all have to line up, or `]`/`[`/`n`/`p` navigate to the
    // wrong row.
    #[test]
    fn parses_a_multi_file_diff_with_correct_indices() {
        let doc = parse_unified_diff(&fixture(MULTI));
        assert_eq!(doc.files.len(), 2);
        assert_eq!(doc.files[0].path, "one.rs");
        assert_eq!(doc.files[1].path, "two.ts");
        assert_eq!(doc.files[1].lang, Language::TypeScript);
        assert_eq!(doc.files[0].first_row, 0);
        assert_eq!(doc.files[0].hunk_rows, vec![1]);
        // 0 header, 1 hunk, 2 removed, 3 added, 4 spacer, 5 header, …
        assert_eq!(doc.rows[4].kind, DiffRowKind::Spacer);
        assert_eq!(doc.lines[4], "");
        // The spacer belongs to the file it follows, so `[`/`]` from it
        // resolves sanely.
        assert_eq!(doc.rows[4].file, 0);
        assert_eq!(doc.files[1].first_row, 5);
        assert_eq!(doc.rows[5].file, 1);
        assert_eq!(doc.rows[12].file, 1);
        // Both hunk headers of the second file are listed, in order.
        assert_eq!(doc.files[1].hunk_rows, vec![6, 10]);
        assert_eq!(doc.rows[10].kind, DiffRowKind::HunkHeader);
        assert_eq!(doc.hunk_count(), 3);
        // No trailing spacer after the last file.
        assert_ne!(doc.rows.last().unwrap().kind, DiffRowKind::Spacer);
    }

    // Numbering is what a reviewer actually reads; this pins all three
    // content kinds advancing the right side, across two hunks.
    #[test]
    fn numbers_old_and_new_lines_across_row_kinds() {
        let doc = parse_unified_diff(&fixture(MULTI));
        let hunk = doc.files[1].hunk_rows[0];
        assert_eq!(doc.rows[hunk + 1].old_no, Some(5)); // context
        assert_eq!(doc.rows[hunk + 1].new_no, Some(5));
        assert_eq!(doc.rows[hunk + 2].old_no, Some(6)); // removed
        assert_eq!(doc.rows[hunk + 2].new_no, None);
        assert_eq!(doc.rows[hunk + 3].old_no, None); // added
        assert_eq!(doc.rows[hunk + 3].new_no, Some(6));
        // The second hunk restarts from its own header, not from wherever
        // the first one stopped.
        let hunk = doc.files[1].hunk_rows[1];
        assert_eq!(doc.rows[hunk + 1].old_no, Some(20));
        assert_eq!(doc.rows[hunk + 1].new_no, Some(20));
        assert_eq!(doc.rows[hunk + 2].old_no, None);
        assert_eq!(doc.rows[hunk + 2].new_no, Some(21));
        // Header rows never carry numbers; the gutter is blank there.
        assert_eq!(doc.rows[hunk].old_no, None);
        assert_eq!(doc.rows[hunk].new_no, None);
        assert_eq!(doc.rows[0].old_no, None);
    }

    // `\ No newline at end of file` is a marker, not content: it must not
    // consume a line number or a hunk slot, and the second one arrives
    // after the hunk's counts are already exhausted.
    #[test]
    fn no_newline_marker_is_meta_and_advances_nothing() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/a.txt b/a.txt",
            "--- a/a.txt",
            "+++ b/a.txt",
            "@@ -1,2 +1,2 @@",
            " keep",
            "-old",
            "\\ No newline at end of file",
            "+new",
            "\\ No newline at end of file",
        ]));
        assert_eq!(
            kinds(&doc),
            vec![
                DiffRowKind::FileHeader,
                DiffRowKind::HunkHeader,
                DiffRowKind::Context,
                DiffRowKind::Removed,
                DiffRowKind::Meta,
                DiffRowKind::Added,
                DiffRowKind::Meta,
            ]
        );
        assert_eq!(doc.lines[4], "\\ No newline at end of file");
        assert_eq!(doc.rows[4].old_no, None);
        assert_eq!(doc.rows[4].new_no, None);
        // The added line that follows still gets new line 2.
        assert_eq!(doc.rows[5].new_no, Some(2));
    }

    // git on Windows can hand us CRLF; a stray `\r` would land inside the
    // rendered text, the copied selection and the tab title.
    #[test]
    fn crlf_input_parses_identically_to_lf() {
        let text = fixture(SIMPLE);
        let lf = parse_unified_diff(&text);
        let from_crlf = parse_unified_diff(&crlf(&text));
        assert_eq!(lf, from_crlf);
        assert!(from_crlf.lines.iter().all(|l| !l.contains('\r')));
    }

    // A merge commit's `@@@` hunks have two marker columns; parsing them as
    // single-column rows would silently attribute changes to the wrong side.
    #[test]
    fn combined_diff_degrades_to_a_meta_row() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --cc src/state.rs",
            "index 1111111,2222222..3333333",
            "--- a/src/state.rs",
            "+++ b/src/state.rs",
            "@@@ -1,3 -1,3 +1,4 @@@",
            "  keep",
            " -theirs",
            "+ ours",
            "++merged",
        ]));
        assert_eq!(doc.files.len(), 1);
        assert_eq!(doc.files[0].path, "src/state.rs");
        assert_eq!(doc.files[0].hunk_rows, Vec::<usize>::new());
        assert_eq!(
            kinds(&doc),
            vec![DiffRowKind::FileHeader, DiffRowKind::Meta]
        );
        assert!(doc.lines[1].contains("combined diff"));
        assert_eq!(doc.hunk_count(), 0);
    }

    #[test]
    fn empty_input_yields_an_empty_document() {
        let doc = parse_unified_diff("");
        assert!(doc.lines.is_empty());
        assert!(doc.rows.is_empty());
        assert!(doc.files.is_empty());
        assert!(doc.is_empty());
        assert!(!doc.truncated);
    }

    // The caller pipes git stdout in unconditionally; an error message or a
    // stray log line must not panic or invent a file.
    #[test]
    fn garbage_input_yields_no_files() {
        let doc = parse_unified_diff("fatal: bad revision 'nope'\n@@ -1 +1 @@\n+orphan\n");
        assert!(doc.files.is_empty());
        assert!(doc.rows.is_empty());
        assert!(doc.is_empty());
        // Binary noise must not panic either.
        let doc = parse_unified_diff("\u{0}\u{1}\u{2}--- +++ @@@@\n");
        assert!(doc.files.is_empty());
    }

    // `git show` output starts with commit/author/message lines before the
    // first `diff --git`.
    #[test]
    fn leading_noise_before_the_first_file_is_skipped() {
        let doc = parse_unified_diff(&format!(
            "commit abcdef\nAuthor: Someone\n\n    a message\n\n{}",
            fixture(SIMPLE)
        ));
        assert_eq!(doc.files.len(), 1);
        assert_eq!(doc.rows[0].kind, DiffRowKind::FileHeader);
        assert_eq!(doc.files[0].first_row, 0);
        assert_eq!(doc.lines[0], "M src/state.rs");
    }

    // The renderer indexes `rows[i]` for every visible `lines[i]`; a drift
    // of one would panic or mis-tint the whole viewport.
    #[test]
    fn lines_and_rows_stay_in_lockstep() {
        for fixture_text in [
            fixture(SIMPLE),
            fixture(MULTI),
            String::new(),
            "garbage\n".to_string(),
        ] {
            let doc = parse_unified_diff(&fixture_text);
            assert_eq!(
                doc.lines.len(),
                doc.rows.len(),
                "lines/rows drifted for {fixture_text:?}"
            );
        }
        // …and every row index a file records must point at the right kind.
        let doc = parse_unified_diff(&fixture(MULTI));
        for file in &doc.files {
            assert_eq!(doc.rows[file.first_row].kind, DiffRowKind::FileHeader);
            for &row in &file.hunk_rows {
                assert_eq!(doc.rows[row].kind, DiffRowKind::HunkHeader);
            }
        }
        // Every row points at a real file entry.
        assert!(doc
            .rows
            .iter()
            .all(|row| (row.file as usize) < doc.files.len()));
    }

    // An empty line inside a hunk body (a context line whose trailing space
    // was stripped in transit) keeps the hunk aligned instead of ending it.
    #[test]
    fn empty_line_inside_a_hunk_is_an_empty_context_row() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/a.rs b/a.rs",
            "--- a/a.rs",
            "+++ b/a.rs",
            "@@ -1,3 +1,3 @@",
            " fn a() {}",
            "",
            "-old",
            "+new",
        ]));
        assert_eq!(
            kinds(&doc),
            vec![
                DiffRowKind::FileHeader,
                DiffRowKind::HunkHeader,
                DiffRowKind::Context,
                DiffRowKind::Context,
                DiffRowKind::Removed,
                DiffRowKind::Added,
            ]
        );
        assert_eq!(doc.lines[3], "");
        assert_eq!(doc.rows[3].old_no, Some(2));
        assert_eq!(doc.rows[3].new_no, Some(2));
        assert_eq!(doc.rows[4].old_no, Some(3));
    }

    // Inside a hunk, `--- x` is a removed line whose text is `-- x`; only
    // the header state machine may read it as a path. This is the reason
    // the parser is a state machine at all.
    #[test]
    fn dashes_inside_a_hunk_body_are_content_not_headers() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/a.md b/a.md",
            "--- a/a.md",
            "+++ b/a.md",
            "@@ -1,2 +1,2 @@",
            "---- heading",
            "++++ heading",
        ]));
        assert_eq!(doc.files[0].path, "a.md");
        assert_eq!(doc.rows[2].kind, DiffRowKind::Removed);
        assert_eq!(doc.lines[2], "--- heading");
        assert_eq!(doc.rows[3].kind, DiffRowKind::Added);
        assert_eq!(doc.lines[3], "+++ heading");
    }

    // `core.quotepath` is disabled by the caller, but a patch pasted from
    // elsewhere can still carry quoted paths; they must decode, not leak
    // octal escapes into the tab title and the file list.
    #[test]
    fn quoted_paths_are_decoded_to_utf8() {
        let doc = parse_unified_diff(&fixture(&[
            r#"diff --git "a/src/w\303\244.rs" "b/src/w\303\244.rs""#,
            r#"--- "a/src/w\303\244.rs""#,
            r#"+++ "b/src/w\303\244.rs""#,
            "@@ -1 +1 @@",
            "-a",
            "+b",
        ]));
        assert_eq!(doc.files[0].path, "src/wä.rs");
        assert_eq!(doc.lines[0], "M src/wä.rs");
        assert_eq!(doc.files[0].lang, Language::Rust);
    }

    // An escape we cannot decode must fall back to the raw text rather than
    // drop the path on the floor.
    #[test]
    fn undecodable_quoted_path_falls_back_to_raw_text() {
        let raw = r#""a/we\xird""#;
        assert_eq!(unquote_path(raw), raw);
        // Valid escapes still decode when mixed with plain bytes.
        assert_eq!(unquote_path(r#""a/x\ty""#), "a/x\ty");
        // Unquoted paths pass through untouched.
        assert_eq!(unquote_path("a/plain.rs"), "a/plain.rs");
        // Only the prefixes git actually emits are stripped.
        assert_eq!(strip_side_prefix("b/src/x.rs"), "src/x.rs");
        assert_eq!(strip_side_prefix("zz/src/x.rs"), "zz/src/x.rs");
    }

    // A single absurd line (a minified bundle, a lock file) makes the
    // editor's per-line layout quadratic; it is clipped on a char boundary
    // with a visible mark so the text is not silently misrepresented.
    #[test]
    fn absurdly_long_lines_are_clamped_on_a_char_boundary() {
        // Multi-byte on purpose: a byte-wise cut would panic here.
        let long = format!("+{}", "ä".repeat(MAX_LINE_CHARS + 500));
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/big.js b/big.js",
            "--- a/big.js",
            "+++ b/big.js",
            "@@ -1 +1 @@",
            "-x",
            &long,
        ]));
        let clipped = &doc.lines[3];
        assert!(clipped.ends_with(CLAMP_SUFFIX));
        assert_eq!(
            clipped.chars().count(),
            MAX_LINE_CHARS + CLAMP_SUFFIX.chars().count()
        );
        // Short lines are untouched.
        assert_eq!(doc.lines[2], "x");
    }

    // The cap keeps a pathological diff from taking the UI thread down.
    // Tested through the `_with_cap` seam so the fixture stays readable
    // instead of being 200 000 rows long.
    #[test]
    fn row_budget_truncates_and_flags_the_document() {
        let doc = parse_unified_diff_with_cap(&fixture(MULTI), 6);
        assert!(doc.truncated);
        assert_eq!(doc.lines.len(), 6);
        assert_eq!(doc.lines.len(), doc.rows.len());
        assert_eq!(doc.rows.last().unwrap().kind, DiffRowKind::Meta);
        assert!(doc.lines.last().unwrap().contains("truncated"));
        // Parsing stopped: the second file never made it in.
        assert_eq!(doc.files.len(), 1);
        // The first file's header was still finalised, not left blank.
        assert_eq!(doc.lines[0], "M one.rs");
        // An ample budget leaves the flag clear.
        assert!(!parse_unified_diff_with_cap(&fixture(MULTI), 1000).truncated);
        assert_eq!(MAX_DIFF_ROWS, 200_000);
    }

    // Truncating exactly at a file boundary must not register a file whose
    // header row was never pushed: `first_row` would dangle and the
    // renderer indexes it directly.
    #[test]
    fn truncation_at_any_boundary_keeps_every_index_valid() {
        let text = fixture(MULTI);
        for cap in 1..=20 {
            let doc = parse_unified_diff_with_cap(&text, cap);
            assert_eq!(doc.lines.len(), doc.rows.len(), "cap {cap}");
            assert!(doc.lines.len() <= cap, "cap {cap}");
            // The wiring agent subscripts `files[row.file]` directly, so
            // this has to hold even for a budget that dies mid-header.
            assert!(
                doc.rows
                    .iter()
                    .all(|row| (row.file as usize) < doc.files.len()),
                "cap {cap}"
            );
            for file in &doc.files {
                assert!(file.first_row < doc.lines.len(), "cap {cap}");
                assert_eq!(
                    doc.rows[file.first_row].kind,
                    DiffRowKind::FileHeader,
                    "cap {cap}"
                );
                for &row in &file.hunk_rows {
                    assert_eq!(doc.rows[row].kind, DiffRowKind::HunkHeader, "cap {cap}");
                }
            }
        }
    }

    // Mode-only changes carry no `---`/`+++` and no hunks, so the path has
    // to come from the `diff --git` line.
    #[test]
    fn mode_only_change_falls_back_to_the_diff_git_paths() {
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/scripts/run.sh b/scripts/run.sh",
            "old mode 100644",
            "new mode 100755",
        ]));
        assert_eq!(doc.files[0].path, "scripts/run.sh");
        assert_eq!(doc.files[0].status, DiffFileStatus::Modified);
        assert_eq!(doc.lines, vec!["M scripts/run.sh"]);
    }

    // The `diff --git` fallback has to split `a/X b/Y` at the *last* ` b/`,
    // or a path that itself contains ` b/` splits in the wrong place.
    #[test]
    fn diff_git_paths_split_at_the_last_side_prefix() {
        assert_eq!(
            split_git_paths("a/src/x.rs b/src/x.rs"),
            (Some("src/x.rs".to_string()), Some("src/x.rs".to_string()))
        );
        assert_eq!(
            split_git_paths("a/x b/y.rs b/x b/y.rs"),
            (Some("x b/y.rs".to_string()), Some("x b/y.rs".to_string()))
        );
    }

    // A missing count in a hunk header means exactly one line; a malformed
    // header is dropped rather than parsed against bogus numbers.
    #[test]
    fn hunk_header_counts_default_to_one_and_reject_garbage() {
        assert_eq!(parse_hunk_header("@@ -3 +7 @@"), Some((3, 1, 7, 1)));
        assert_eq!(
            parse_hunk_header("@@ -3,0 +7,2 @@ fn x()"),
            Some((3, 0, 7, 2))
        );
        assert_eq!(parse_hunk_header("@@ nonsense @@"), None);
        assert_eq!(parse_hunk_header("@@ -a,b +c,d @@"), None);
        assert_eq!(parse_hunk_header("not a hunk"), None);
        // A header with no counts still renders a hunk row and one line.
        let doc = parse_unified_diff(&fixture(&[
            "diff --git a/a.rs b/a.rs",
            "--- a/a.rs",
            "+++ b/a.rs",
            "@@ -3 +7 @@",
            "-gone",
            "+here",
        ]));
        assert_eq!(doc.rows[2].old_no, Some(3));
        assert_eq!(doc.rows[3].new_no, Some(7));
    }

    // The gutter marker and the highlight decision are read off the kind,
    // since the row text no longer carries the marker.
    #[test]
    fn row_kind_exposes_marker_and_content_flag() {
        assert_eq!(DiffRowKind::Added.marker(), '+');
        assert_eq!(DiffRowKind::Removed.marker(), '-');
        assert_eq!(DiffRowKind::Context.marker(), ' ');
        assert_eq!(DiffRowKind::FileHeader.marker(), ' ');
        assert!(DiffRowKind::Context.is_content());
        assert!(DiffRowKind::Added.is_content());
        assert!(!DiffRowKind::HunkHeader.is_content());
        assert!(!DiffRowKind::Spacer.is_content());
        assert!(!DiffRowKind::Meta.is_content());
        assert_eq!(DiffFileStatus::Added.letter(), 'A');
        assert_eq!(DiffFileStatus::Deleted.letter(), 'D');
        assert_eq!(DiffFileStatus::Modified.letter(), 'M');
        assert_eq!(DiffFileStatus::Renamed.letter(), 'R');
        assert_eq!(DiffFileStatus::Binary.letter(), 'B');
    }
}
