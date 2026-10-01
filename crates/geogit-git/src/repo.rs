use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// A GeoGit repository wrapping a git repository.
pub struct Repository {
    /// Path to the repository root (working directory).
    pub workdir: PathBuf,
}

impl Repository {
    /// Initialize a new GeoGit repository at the given path.
    pub fn init(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path).context("failed to create repository directory")?;

        let output = Command::new("git")
            .args(["init", "--template="])
            .current_dir(path)
            .output()
            .context("failed to run git init")?;

        if !output.status.success() {
            bail!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(Self {
            workdir: path.to_path_buf(),
        })
    }

    /// Open an existing GeoGit repository.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.join(".git").exists() {
            bail!("not a git repository: {}", path.display());
        }
        Ok(Self {
            workdir: path.to_path_buf(),
        })
    }

    /// Get the HEAD commit hash, if any.
    pub fn head_commit(&self) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git rev-parse")?;

        if output.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    /// List all branches.
    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let output = Command::new("git")
            .args(["branch", "--format=%(refname:short) %(objectname:short)"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git branch")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let mut parts = line.splitn(2, ' ');
                let name = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                BranchInfo { name, target }
            })
            .collect();
        Ok(branches)
    }

    /// Get the current branch name.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git branch --show-current")?;

        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// Create a new branch at the given target.
    pub fn create_branch(&self, name: &str, target: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["branch", name, target])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git branch")?;

        if !output.status.success() {
            bail!(
                "git branch failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Delete a branch.
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["branch", "-d", name])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git branch -d")?;

        if !output.status.success() {
            bail!(
                "git branch -d failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Stage all changes and commit.
    pub fn commit(&self, message: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git add")?;

        if !output.status.success() {
            bail!(
                "git add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git commit")?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stdout.contains("nothing to commit") || stderr.contains("nothing to commit") {
                return Ok("nothing to commit".into());
            }
            let reason = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            bail!("git commit failed: {reason}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Push to a remote.
    pub fn push(&self, remote: &str, branch: Option<&str>) -> Result<String> {
        let mut args = vec!["push", remote];
        if let Some(b) = branch {
            args.push(b);
        }
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git push")?;

        if !output.status.success() {
            bail!("push failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        // git push often writes to stderr even on success
        let out = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(out.trim().to_string())
    }

    /// Pull from a remote.
    pub fn pull(&self, remote: &str, branch: Option<&str>) -> Result<String> {
        let mut args = vec!["pull", remote];
        if let Some(b) = branch {
            args.push(b);
        }
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git pull")?;

        if !output.status.success() {
            bail!("pull failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Clone a remote repository.
    pub fn clone_repo(url: &str, dest: &Path) -> Result<Self> {
        let output = Command::new("git")
            .args(["clone", url, &dest.to_string_lossy()])
            .output()
            .context("failed to run git clone")?;

        if !output.status.success() {
            bail!("clone failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(Self {
            workdir: dest.to_path_buf(),
        })
    }

    /// Add a remote.
    pub fn remote_add(&self, name: &str, url: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["remote", "add", name, url])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git remote add")?;

        if !output.status.success() {
            bail!(
                "remote add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Remove a remote.
    pub fn remote_remove(&self, name: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["remote", "remove", name])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git remote remove")?;

        if !output.status.success() {
            bail!(
                "remote remove failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// List remotes.
    pub fn remotes(&self) -> Result<Vec<RemoteInfo>> {
        let output = Command::new("git")
            .args(["remote", "-v"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git remote -v")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut remotes = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && seen.insert(parts[0].to_string()) {
                remotes.push(RemoteInfo {
                    name: parts[0].to_string(),
                    url: parts[1].to_string(),
                });
            }
        }
        Ok(remotes)
    }

    /// Show a specific commit (full log message + stat).
    pub fn show_commit(&self, commit: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["show", "--stat", commit])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git show")?;

        if !output.status.success() {
            bail!("show failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Get diff between two tree-ish references (file-level).
    pub fn diff_tree(&self, base: &str, target: &str) -> Result<Vec<DiffEntry>> {
        let output = Command::new("git")
            .args(["diff", "--name-status", "--no-renames", base, target, "--"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git diff")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let status = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("").to_string();
            let kind = match status {
                "A" => DiffStatus::Added,
                "D" => DiffStatus::Deleted,
                "M" => DiffStatus::Modified,
                _ => DiffStatus::Modified,
            };
            entries.push(DiffEntry { status: kind, path });
        }
        Ok(entries)
    }

    /// List files changed in the working tree vs index (unstaged changes).
    pub fn diff_working(&self) -> Result<Vec<DiffEntry>> {
        // Untracked + modified + deleted
        let output = Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git status")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            if line.len() < 4 {
                continue;
            }
            let status_chars = &line[..2];
            let path = line[3..].to_string();
            let kind = match status_chars.trim() {
                "M" | "MM" => DiffStatus::Modified,
                "A" | "??" => DiffStatus::Added,
                "D" => DiffStatus::Deleted,
                _ => DiffStatus::Modified,
            };
            entries.push(DiffEntry { status: kind, path });
        }
        Ok(entries)
    }

    /// Hard reset the working tree to HEAD (or a specific ref).
    pub fn reset_hard(&self, target: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["reset", "--hard", target])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git reset")?;

        if !output.status.success() {
            bail!("reset failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    /// Checkout a specific file/path from a ref.
    pub fn checkout_path(&self, reference: &str, path: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["checkout", reference, "--", path])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git checkout")?;

        if !output.status.success() {
            bail!(
                "checkout failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Read a file at a specific commit.
    pub fn read_file_at(&self, commit: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let spec = format!("{commit}:{path}");
        let output = Command::new("git")
            .args(["show", &spec])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git show")?;

        if output.status.success() {
            Ok(Some(output.stdout))
        } else {
            Ok(None)
        }
    }

    pub fn read_files_at(&self, commit: &str, paths: &[String]) -> Result<Vec<Option<Vec<u8>>>> {
        // paths with spaces work because the default format reads the whole line as the object name
        let mut child = Command::new("git")
            .args(["cat-file", "--batch"])
            .current_dir(&self.workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to run git cat-file --batch")?;

        let requests: String = paths
            .iter()
            .map(|path| format!("{commit}:{path}\n"))
            .collect();
        let mut stdin = child.stdin.take().context("git cat-file has no stdin")?;
        let writer = std::thread::spawn(move || stdin.write_all(requests.as_bytes()));
        let output = child
            .wait_with_output()
            .context("failed to read git cat-file output")?;
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("git cat-file writer thread panicked"))?
            .context("failed to write to git cat-file")?;

        if !output.status.success() {
            bail!(
                "git cat-file failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let mut remaining = output.stdout.as_slice();
        let mut blobs = Vec::with_capacity(paths.len());
        for path in paths {
            let header_end = remaining
                .iter()
                .position(|&byte| byte == b'\n')
                .with_context(|| format!("git cat-file reply for {path} has no header"))?;
            let header = std::str::from_utf8(&remaining[..header_end])
                .with_context(|| format!("git cat-file header for {path} is not utf-8"))?;
            remaining = &remaining[header_end + 1..];
            if header.ends_with(" missing") {
                blobs.push(None);
                continue;
            }
            let size: usize = header
                .rsplit(' ')
                .next()
                .and_then(|size| size.parse().ok())
                .with_context(|| format!("unexpected git cat-file header for {path}: {header}"))?;
            if remaining.len() < size + 1 {
                bail!("git cat-file reply for {path} is shorter than {size} bytes");
            }
            blobs.push(Some(remaining[..size].to_vec()));
            remaining = &remaining[size + 1..];
        }
        Ok(blobs)
    }

    /// List tree entries at a path within a commit.
    pub fn ls_tree(&self, commit: &str, path: &str) -> Result<Vec<String>> {
        let spec = format!("{commit}:{path}");
        let output = Command::new("git")
            .args(["ls-tree", "--name-only", &spec])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git ls-tree")?;

        if !output.status.success() {
            return Ok(vec![]);
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Recursively list all blobs under a tree path.
    pub fn ls_tree_recursive(&self, commit: &str, path: &str) -> Result<Vec<String>> {
        let spec = format!("{commit}:{path}");
        let output = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", &spec])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git ls-tree -r")?;

        if !output.status.success() {
            return Ok(vec![]);
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Switch to a branch or commit.
    pub fn switch_branch(&self, branch: &str, create: bool) -> Result<()> {
        let mut args = vec!["switch".to_string()];
        if create {
            args.push("-c".into());
        }
        args.push(branch.into());

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git switch")?;

        if !output.status.success() {
            bail!("switch failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    /// Merge a branch, returning the output.
    pub fn merge(&self, branch: &str) -> Result<MergeResult> {
        let output = Command::new("git")
            .args(["merge", branch])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git merge")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(MergeResult {
                success: true,
                message: stdout,
                conflicts: vec![],
            })
        } else if stdout.contains("CONFLICT") || stderr.contains("CONFLICT") {
            // Parse conflicted files
            let conflicts = self.list_conflicts()?;
            Ok(MergeResult {
                success: false,
                message: format!("{stdout}{stderr}"),
                conflicts,
            })
        } else {
            bail!("merge failed: {stderr}");
        }
    }

    /// List files with merge conflicts.
    pub fn list_conflicts(&self) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to list conflicts")?;

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Mark conflicts as resolved by staging files.
    pub fn resolve_conflicts(&self, paths: &[&str]) -> Result<()> {
        let mut args = vec!["add"];
        args.extend(paths);

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git add")?;

        if !output.status.success() {
            bail!(
                "resolve failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Abort an in-progress merge.
    pub fn merge_abort(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git merge --abort")?;

        if !output.status.success() {
            bail!(
                "merge abort failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Get log entries.
    pub fn log(&self, max_count: Option<usize>, oneline: bool) -> Result<Vec<LogEntry>> {
        let mut args = vec![
            "log".to_string(),
            "--format=%H%n%h%n%an%n%ae%n%aI%n%s%n%b%n---END---".to_string(),
        ];
        if let Some(n) = max_count {
            args.push(format!("-{n}"));
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.workdir)
            .output()
            .context("failed to run git log")?;

        if !output.status.success() {
            // empty repo
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        let mut lines_iter = stdout.lines();

        loop {
            let hash = match lines_iter.next() {
                Some(h) if !h.is_empty() => h.to_string(),
                _ => break,
            };
            let short_hash = lines_iter.next().unwrap_or("").to_string();
            let author_name = lines_iter.next().unwrap_or("").to_string();
            let author_email = lines_iter.next().unwrap_or("").to_string();
            let date = lines_iter.next().unwrap_or("").to_string();
            let subject = lines_iter.next().unwrap_or("").to_string();

            // Read body until ---END---
            let mut body_lines = Vec::new();
            for line in lines_iter.by_ref() {
                if line == "---END---" {
                    break;
                }
                body_lines.push(line.to_string());
            }

            let _ = oneline; // used by caller for formatting
            entries.push(LogEntry {
                hash,
                short_hash,
                author_name,
                author_email,
                date,
                subject,
                body: body_lines.join("\n").trim().to_string(),
            });
        }

        Ok(entries)
    }
}

/// Result of a merge operation.
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub success: bool,
    pub message: String,
    pub conflicts: Vec<String>,
}

/// A log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub hash: String,
    pub short_hash: String,
    pub author_name: String,
    pub author_email: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

/// A file-level diff entry.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub status: DiffStatus,
    pub path: String,
}

/// Status of a changed file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
}

/// Information about a remote.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
}

/// Information about a branch.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_init_and_open() {
        let dir = tempdir();
        let _repo = Repository::init(dir.path()).unwrap();
        assert!(dir.path().join(".git").exists());

        let _repo2 = Repository::open(dir.path()).unwrap();
    }

    #[test]
    fn test_head_on_empty_repo() {
        let dir = tempdir();
        let repo = Repository::init(dir.path()).unwrap();
        let head = repo.head_commit().unwrap();
        assert!(head.is_none());
    }

    #[test]
    fn test_commit_and_branch() {
        let dir = tempdir();
        let repo = Repository::init(dir.path()).unwrap();

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        fs::write(dir.path().join("test.txt"), "hello").unwrap();
        let result = repo.commit("initial").unwrap();
        assert!(!result.contains("nothing to commit"));

        let head = repo.head_commit().unwrap();
        assert!(head.is_some());

        repo.create_branch("feature", "HEAD").unwrap();
        let branches = repo.branches().unwrap();
        assert!(branches.iter().any(|b| b.name == "feature"));

        repo.delete_branch("feature").unwrap();
        let branches = repo.branches().unwrap();
        assert!(!branches.iter().any(|b| b.name == "feature"));
    }

    fn tempdir() -> TempDir {
        TempDir::new()
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "geogit-test-{}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
