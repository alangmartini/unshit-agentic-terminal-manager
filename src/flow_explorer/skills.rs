//! Personal skill copies. The saved original is the ownership receipt: a
//! customized file is never updated or removed by Terminal Manager.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const RECEIPT: &str = ".unshit-installed-skill";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillAgent {
    Codex,
    Claude,
    Cursor,
    Copilot,
}

impl SkillAgent {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Cursor, Self::Copilot];

    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Copilot => "GitHub Copilot",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.id() == id)
    }

    pub fn relative_dir(self) -> &'static str {
        match self {
            Self::Codex => ".agents/skills/flow-explorer",
            Self::Claude => ".claude/skills/flow-explorer",
            Self::Cursor => ".cursor/skills/flow-explorer",
            Self::Copilot => ".copilot/skills/flow-explorer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallStatus {
    Missing,
    Installed,
    UpdateAvailable,
    Conflict,
    Unavailable(String),
}

impl InstallStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Missing => "Not installed",
            Self::Installed => "Installed",
            Self::UpdateAvailable => "Update available",
            Self::Conflict => "Custom skill — preserved",
            Self::Unavailable(message) => message,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SkillInstallation {
    pub agent: SkillAgent,
    pub path: PathBuf,
    pub status: InstallStatus,
}

pub struct SkillInstaller {
    home: PathBuf,
    content: String,
}

impl SkillInstaller {
    pub fn for_current_user() -> io::Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::other("Cannot find your personal skills directory"))?;
        let executable = std::env::current_exe()?;
        let output = super::flows_dir()
            .ok_or_else(|| io::Error::other("Cannot find the Flow output directory"))?;
        Ok(Self::new(home, &executable, &output))
    }

    fn new(home: PathBuf, executable: &Path, output: &Path) -> Self {
        // JSON quoting makes spaces and Windows backslashes unambiguous without
        // embedding an executable shell command in the installed instructions.
        let exe = serde_json::to_string(&executable.to_string_lossy()).unwrap();
        let out = serde_json::to_string(&output.to_string_lossy()).unwrap();
        Self {
            home,
            content: format!(
                "{}\n## Local installation\n\nApplication executable (JSON string): {exe}\n\n\
                 Default output directory (JSON string): {out}\n\n\
                 Use these paths for standalone requests. Invoke the executable with\n\
                 `flow open <absolute-output-path>` using your shell's argument quoting.\n\
                 Keep any inherited `TM_NOTIFY_SOCKET` and `TM_WORKSPACE_ID` so the\n\
                 diagram opens in the calling workspace.\n",
                super::producer::skill()
            ),
        }
    }

    pub fn inspect(&self, agent: SkillAgent) -> SkillInstallation {
        let dir = self.home.join(agent.relative_dir());
        SkillInstallation {
            agent,
            path: dir.join("SKILL.md"),
            status: self
                .status(&dir)
                .unwrap_or_else(|error| InstallStatus::Unavailable(error.to_string())),
        }
    }

    fn status(&self, dir: &Path) -> io::Result<InstallStatus> {
        match fs::symlink_metadata(dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(InstallStatus::Missing),
            Err(e) => return Err(e),
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                return Ok(InstallStatus::Conflict);
            }
            Ok(_) => {}
        }
        // Refuse redirected files, including a dangling link. Only a regular
        // SKILL.md with its matching receipt belongs to this installer.
        for file in ["SKILL.md", RECEIPT] {
            match fs::symlink_metadata(dir.join(file)) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
                Ok(_) => return Ok(InstallStatus::Conflict),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Ok(InstallStatus::Conflict)
                }
                Err(e) => return Err(e),
            }
        }
        let current = fs::read(dir.join("SKILL.md"))?;
        if current != fs::read(dir.join(RECEIPT))? {
            return Ok(InstallStatus::Conflict);
        }
        Ok(if current == self.content.as_bytes() {
            InstallStatus::Installed
        } else {
            InstallStatus::UpdateAvailable
        })
    }

    pub fn install(&self, agent: SkillAgent) -> io::Result<()> {
        let dir = self.home.join(agent.relative_dir());
        match self.status(&dir)? {
            InstallStatus::Missing => {
                fs::create_dir_all(dir.parent().unwrap())?;
                // Exclusive creation prevents adopting an existing skill.
                fs::create_dir(&dir)?;
            }
            InstallStatus::Installed => return Ok(()),
            InstallStatus::UpdateAvailable => {}
            _ => return Err(io::Error::other("Existing or customized skill preserved")),
        }
        crate::persist::atomic_write(&dir.join("SKILL.md"), self.content.as_bytes())?;
        crate::persist::atomic_write(&dir.join(RECEIPT), self.content.as_bytes())
    }

    pub fn remove(&self, agent: SkillAgent) -> io::Result<()> {
        let dir = self.home.join(agent.relative_dir());
        match self.status(&dir)? {
            InstallStatus::Missing => return Ok(()),
            InstallStatus::Installed | InstallStatus::UpdateAvailable => {}
            _ => return Err(io::Error::other("Existing or customized skill preserved")),
        }
        fs::remove_file(dir.join("SKILL.md"))?;
        fs::remove_file(dir.join(RECEIPT))?;
        // Never recursively delete: user-added references and scripts survive.
        match fs::remove_dir(&dir) {
            Ok(()) => Ok(()),
            Err(_) if fs::read_dir(&dir)?.next().is_some() => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempHome(PathBuf);
    impl TempHome {
        fn new() -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tm-flow-skills-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn installer(&self) -> SkillInstaller {
            SkillInstaller::new(
                self.0.clone(),
                Path::new("C:/Program Files/Unshit/terminal-manager.exe"),
                Path::new("C:/My Flows"),
            )
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn installs_and_removes_each_personal_copy_idempotently() {
        let home = TempHome::new();
        let installer = home.installer();
        for agent in SkillAgent::ALL {
            assert_eq!(installer.inspect(agent).status, InstallStatus::Missing);
            installer.install(agent).unwrap();
            installer.install(agent).unwrap();
            let info = installer.inspect(agent);
            assert_eq!(info.status, InstallStatus::Installed);
            assert_eq!(fs::read_to_string(info.path).unwrap(), installer.content);
            installer.remove(agent).unwrap();
            installer.remove(agent).unwrap();
            assert_eq!(installer.inspect(agent).status, InstallStatus::Missing);
        }
    }

    #[test]
    fn updates_owned_copy_and_preserves_custom_edits() {
        let home = TempHome::new();
        let mut installer = home.installer();
        installer.install(SkillAgent::Codex).unwrap();
        installer.content.push_str("\nNew skill version\n");
        assert_eq!(
            installer.inspect(SkillAgent::Codex).status,
            InstallStatus::UpdateAvailable
        );
        installer.install(SkillAgent::Codex).unwrap();
        let path = installer.inspect(SkillAgent::Codex).path;
        fs::write(&path, "My own instructions").unwrap();
        assert_eq!(
            installer.inspect(SkillAgent::Codex).status,
            InstallStatus::Conflict
        );
        assert!(installer.install(SkillAgent::Codex).is_err());
        assert!(installer.remove(SkillAgent::Codex).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "My own instructions");
    }

    #[test]
    fn preserves_unmanaged_skills_and_extra_files() {
        let home = TempHome::new();
        let installer = home.installer();
        let path = installer.inspect(SkillAgent::Claude).path;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "Existing flow skill").unwrap();
        assert!(installer.install(SkillAgent::Claude).is_err());
        assert!(installer.remove(SkillAgent::Claude).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "Existing flow skill");

        installer.install(SkillAgent::Cursor).unwrap();
        let dir = installer
            .inspect(SkillAgent::Cursor)
            .path
            .parent()
            .unwrap()
            .to_path_buf();
        fs::write(dir.join("notes.md"), "Keep me").unwrap();
        installer.remove(SkillAgent::Cursor).unwrap();
        assert_eq!(fs::read_to_string(dir.join("notes.md")).unwrap(), "Keep me");
        assert!(!dir.join("SKILL.md").exists());
    }

    #[test]
    fn filesystem_failures_are_reported() {
        let home = TempHome::new();
        let installer = home.installer();
        fs::write(home.0.join(".agents"), "not a directory").unwrap();
        assert!(installer.install(SkillAgent::Codex).is_err());
        assert_ne!(
            installer.inspect(SkillAgent::Codex).status,
            InstallStatus::Installed
        );
    }
}
