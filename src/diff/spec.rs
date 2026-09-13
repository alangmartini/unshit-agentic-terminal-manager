//! What to diff: the range a diff pane was opened for, and the exact
//! `git diff` arguments it becomes.
//!
//! Ranges reach this module from three places that must agree — the
//! `diff.open:<range>` dispatch, the "Diff against…" dialog, and a Flow
//! document's `diff_range { base, head }` — so the mapping lives here
//! once, with the validation that keeps user text from becoming git
//! flags.

/// A validated diff range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSpec {
    /// What we diff against. `HEAD` for "my uncommitted work".
    pub base: String,
    /// The other side, or `None` for "the working tree" (staged and
    /// unstaged changes included), which is what `git diff <base>` means.
    pub head: Option<String>,
    /// Whether the range was written with `...` (diff against the merge
    /// base), which git takes as a single argument rather than two.
    pub symmetric: bool,
}

/// Longest range text accepted. Ref names are short; anything longer is
/// a paste accident, and the value ends up in a tab title.
const MAX_RANGE_LEN: usize = 256;

impl DiffSpec {
    /// Parse the user-facing range syntax:
    ///
    /// | input          | meaning                                   |
    /// |----------------|-------------------------------------------|
    /// | empty, `HEAD`  | working tree vs `HEAD` (uncommitted work) |
    /// | `A`            | working tree vs `A`                       |
    /// | `A..B`         | `B` vs `A`                                |
    /// | `A...B`        | `B` vs the merge base of `A` and `B`      |
    pub fn parse(input: &str) -> Result<Self, String> {
        let text = input.trim();
        if text.len() > MAX_RANGE_LEN {
            return Err(format!(
                "range is too long (max {MAX_RANGE_LEN} characters)"
            ));
        }
        if text.is_empty() {
            return Ok(Self {
                base: "HEAD".to_string(),
                head: None,
                symmetric: false,
            });
        }
        // Check the three-dot form first: `A...B` also contains `..`.
        if let Some((base, head)) = text.split_once("...") {
            return Self::build(base, Some(head), true);
        }
        if let Some((base, head)) = text.split_once("..") {
            return Self::build(base, Some(head), false);
        }
        Self::build(text, None, false)
    }

    /// Map a Flow document's `diff_range` onto a spec. The Flow producer
    /// is told to request "merge base..HEAD including uncommitted
    /// changes", so a head of `HEAD` (or an empty / working-tree spelling)
    /// means "the working tree", not the `HEAD` commit — diffing the
    /// commit would hide exactly the uncommitted work the review is about.
    pub fn from_flow_range(base: &str, head: &str) -> Result<Self, String> {
        let head = head.trim();
        let working_tree = matches!(
            head.to_ascii_lowercase().as_str(),
            "" | "head" | "worktree" | "working tree" | "working-tree" | "workdir"
        );
        if working_tree {
            Self::build(base, None, false)
        } else {
            Self::build(base, Some(head), false)
        }
    }

    fn build(base: &str, head: Option<&str>, symmetric: bool) -> Result<Self, String> {
        let base = validate_ref(base, "base")?;
        let head = match head {
            Some(h) => Some(validate_ref(h, "head")?),
            None => None,
        };
        Ok(Self {
            base,
            head,
            symmetric,
        })
    }

    /// The arguments to append to `git diff`. Never contains a flag:
    /// [`validate_ref`] rejects anything starting with `-`, so a range
    /// can never turn into `--output=…` or another option.
    pub fn git_args(&self) -> Vec<String> {
        match (&self.head, self.symmetric) {
            (Some(head), true) => vec![format!("{}...{}", self.base, head)],
            (Some(head), false) => vec![self.base.clone(), head.clone()],
            (None, _) => vec![self.base.clone()],
        }
    }

    /// How the range reads in a tab title or a header.
    pub fn label(&self) -> String {
        match (&self.head, self.symmetric) {
            (Some(head), true) => format!("{}...{}", self.base, head),
            (Some(head), false) => format!("{}..{}", self.base, head),
            (None, _) => self.base.clone(),
        }
    }

    /// True when this is the "show me my uncommitted work" default, which
    /// deserves a friendlier empty state than a named range.
    pub fn is_working_tree(&self) -> bool {
        self.head.is_none()
    }
}

/// Reject anything that is not plausibly a git revision. This is the
/// boundary between user text and a process argument, so it is a
/// whitelist of shapes rather than a blacklist of characters: git itself
/// forbids most of these in ref names (`git check-ref-format`), and the
/// ones it allows in a revision expression (`~`, `^`, `@{…}`, `:`) are
/// kept because they are how people actually address commits.
fn validate_ref(raw: &str, role: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(format!("missing {role} revision"));
    }
    if text.starts_with('-') {
        // The one that matters: `--output=x` as a "range" would make git
        // write a file instead of printing a diff.
        return Err(format!("{role} revision may not start with '-'"));
    }
    if text.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(
                c,
                ';' | '|'
                    | '&'
                    | '`'
                    | '$'
                    | '\\'
                    | '"'
                    | '\''
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '<'
                    | '>'
                    | '('
                    | ')'
            )
    }) {
        return Err(format!("{role} revision contains an unsupported character"));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_head_mean_the_working_tree() {
        for input in ["", "   ", "HEAD"] {
            let spec = DiffSpec::parse(input).expect("parsed");
            assert_eq!(spec.base, "HEAD", "input {input:?}");
            assert_eq!(spec.head, None);
            assert!(spec.is_working_tree());
            assert_eq!(spec.git_args(), vec!["HEAD".to_string()]);
        }
    }

    #[test]
    fn a_single_ref_diffs_the_working_tree_against_it() {
        let spec = DiffSpec::parse("main").expect("parsed");
        assert_eq!(spec.git_args(), vec!["main".to_string()]);
        assert_eq!(spec.label(), "main");
        assert!(spec.is_working_tree());
    }

    #[test]
    fn two_dot_range_becomes_two_arguments() {
        let spec = DiffSpec::parse("main..feature").expect("parsed");
        assert_eq!(
            spec.git_args(),
            vec!["main".to_string(), "feature".to_string()]
        );
        assert_eq!(spec.label(), "main..feature");
        assert!(!spec.is_working_tree());
    }

    /// `A...B` must stay one argument: split into two it would diff the
    /// tips instead of diffing against the merge base, which is a
    /// different (and usually much noisier) answer.
    #[test]
    fn three_dot_range_stays_one_argument() {
        let spec = DiffSpec::parse("main...feature").expect("parsed");
        assert!(spec.symmetric);
        assert_eq!(spec.git_args(), vec!["main...feature".to_string()]);
        assert_eq!(spec.label(), "main...feature");
    }

    #[test]
    fn revision_syntax_people_actually_use_is_accepted() {
        for input in [
            "HEAD~1",
            "HEAD^",
            "origin/main",
            "v0.4.0",
            "@{u}",
            "abc1234",
            "HEAD~3..HEAD",
            "feat/rust-terminal-manager",
        ] {
            assert!(DiffSpec::parse(input).is_ok(), "should accept {input:?}");
        }
    }

    /// The security property: a range is user text that becomes a process
    /// argument. A leading dash would make it an option.
    #[test]
    fn leading_dash_is_rejected_on_both_sides() {
        let err = DiffSpec::parse("--output=/tmp/pwn").expect_err("must reject");
        assert!(err.contains('-'), "got {err:?}");
        assert!(DiffSpec::parse("main..--exec").is_err());
        assert!(DiffSpec::parse("-p").is_err());
    }

    #[test]
    fn shell_metacharacters_and_whitespace_are_rejected() {
        for input in [
            "main; rm -rf /",
            "main | cat",
            "main && echo",
            "main `id`",
            "main $(id)",
            "with space",
            "quote\"d",
            "new\nline",
        ] {
            assert!(DiffSpec::parse(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn missing_side_of_a_range_is_an_error() {
        assert!(DiffSpec::parse("main..").is_err());
        assert!(DiffSpec::parse("..main").is_err());
        assert!(DiffSpec::parse("...").is_err());
    }

    #[test]
    fn overlong_input_is_rejected_before_it_reaches_a_tab_title() {
        let long = "a".repeat(MAX_RANGE_LEN + 1);
        assert!(DiffSpec::parse(&long).is_err());
    }

    /// A Flow review asks for "merge base..HEAD including uncommitted
    /// changes". Treating its `head: "HEAD"` as the commit would drop
    /// exactly the uncommitted work under review.
    #[test]
    fn flow_range_with_head_means_the_working_tree() {
        for head in ["HEAD", "head", "", "  ", "worktree", "working tree"] {
            let spec = DiffSpec::from_flow_range("main", head).expect("parsed");
            assert!(
                spec.is_working_tree(),
                "head {head:?} should mean the working tree"
            );
            assert_eq!(spec.git_args(), vec!["main".to_string()]);
        }
    }

    #[test]
    fn flow_range_with_a_real_head_diffs_the_two_revisions() {
        let spec = DiffSpec::from_flow_range("main", "feat/prompt-restore").expect("parsed");
        assert_eq!(
            spec.git_args(),
            vec!["main".to_string(), "feat/prompt-restore".to_string()]
        );
    }

    #[test]
    fn flow_range_validates_its_refs_too() {
        assert!(DiffSpec::from_flow_range("-x", "main").is_err());
        assert!(DiffSpec::from_flow_range("main", "; rm").is_err());
        assert!(DiffSpec::from_flow_range("", "main").is_err());
    }
}
