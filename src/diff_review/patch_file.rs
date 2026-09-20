//! Read-only imports of Git-format patch files.
use super::git::{File, Line, Report};
use crate::diff::parse::{parse_unified_diff, DiffFileStatus, DiffRowKind};
use std::io::Read;
use std::path::Path;

const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub fn load(path: &Path) -> Result<Report, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open patch: {e}"))?;
    if !file
        .metadata()
        .map_err(|e| format!("Cannot inspect patch: {e}"))?
        .is_file()
    {
        return Err("Choose a regular patch file.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read patch: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Patch exceeds the 4 MiB preview limit.".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Patch is not UTF-8. Export it as a UTF-8 patch and try again.")?;
    parse(path, text.trim_start_matches('\u{feff}'))
}

fn parse(path: &Path, text: &str) -> Result<Report, String> {
    let doc = parse_unified_diff(text);
    if doc.files.is_empty() {
        return Err(
            "No Git-format diff found. Choose a patch containing diff --git file headers.".into(),
        );
    }
    if doc.truncated {
        return Err("Patch exceeds the preview row limit. Export a smaller patch.".into());
    }
    let mut files: Vec<File> = doc
        .files
        .into_iter()
        .map(|file| {
            let binary = file.status == DiffFileStatus::Binary;
            File {
                path: file.path,
                old_path: file.old_path,
                added: (!binary).then_some(0),
                removed: (!binary).then_some(0),
            }
        })
        .collect();
    let mut patches = vec![Vec::new(); files.len()];
    for (row, text) in doc.rows.into_iter().zip(doc.lines) {
        let index = row.file as usize;
        let (kind, marker) = match row.kind {
            DiffRowKind::Spacer => continue,
            DiffRowKind::Added => {
                if let Some(count) = &mut files[index].added {
                    *count += 1;
                }
                ("added", "+")
            }
            DiffRowKind::Removed => {
                if let Some(count) = &mut files[index].removed {
                    *count += 1;
                }
                ("removed", "-")
            }
            DiffRowKind::Context => ("context", " "),
            DiffRowKind::HunkHeader => ("hunk", ""),
            _ => ("meta", ""),
        };
        patches[index].push(Line {
            old: row.old_no.map(|n| n as usize),
            new: row.new_no.map(|n| n as usize),
            kind,
            text: format!("{marker}{text}"),
        });
    }
    Ok(Report {
        root: path.to_path_buf(),
        base: String::new(),
        head: String::new(),
        label: "Imported patch; counts reflect displayed changes".into(),
        files,
        patches: Some(patches),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_intellij_patch_without_a_repository() {
        let path =
            std::env::temp_dir().join(format!("review-intellij-{}.patch", std::process::id()));
        std::fs::write(&path, "Subject: [PATCH] Example\r\n---\r\nIndex: src/example.cpp\r\nIDEA additional info:\r\n<+>UTF-8\r\n===================================================================\r\ndiff --git a/src/example.cpp b/src/example.cpp\r\n--- a/src/example.cpp\t(revision abc)\r\n+++ b/src/example.cpp\t(date 123)\r\n@@ -10,2 +10,2 @@\r\n context\r\n-old\r\n+new\r\n\\ No newline at end of file\r\nIndex: image.png\r\ndiff --git a/image.png b/image.png\r\nBinary files a/image.png and b/image.png differ\r\n").unwrap();
        let result = load(&path);
        std::fs::remove_file(&path).unwrap();
        let report = result.unwrap();
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.files[0].path, "src/example.cpp");
        assert_eq!(report.files[0].added, Some(1));
        assert_eq!(report.files[0].removed, Some(1));
        assert_eq!(report.files[1].added, None);
        let patches = report.patches.unwrap();
        assert_eq!(patches[0][2].old, Some(10));
        assert_eq!(patches[0][3].text, "-old");
        assert_eq!(patches[0][4].new, Some(11));
        assert!(patches[0].last().unwrap().text.starts_with("\\ No newline"));
        assert_eq!(patches[1].last().unwrap().text, "Binary files differ");
    }

    #[test]
    fn rejects_unreadable_oversized_and_invalid_files() {
        let path =
            std::env::temp_dir().join(format!("review-invalid-{}.patch", std::process::id()));
        assert!(load(&path).is_err());
        for (bytes, error) in [
            (vec![b'x'; MAX_BYTES as usize + 1], "4 MiB"),
            (vec![0xff], "UTF-8"),
            (b"not a patch".to_vec(), "No Git-format diff"),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(&path).unwrap_err().contains(error));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preserves_rename_and_deletion_paths_and_hunk_markers() {
        let report = parse(Path::new("sample.patch"), "diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n--- heading\n+++ heading\ndiff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-removed\n").unwrap();
        assert_eq!(report.files[0].path, "new.txt");
        assert_eq!(report.files[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(report.files[1].path, "gone.txt");
        assert_eq!(report.files[1].removed, Some(1));
        assert_eq!(report.patches.unwrap()[0][2].text, "--- heading");
    }
}
