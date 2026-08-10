//! Current git branch, read straight off the filesystem.
//!
//! No `git` subprocess: spawning one on every render would be slow in a
//! large repo and simply unavailable if git isn't installed, and it's
//! unnecessary — `HEAD` is a couple of tiny, well-defined text files. Worth
//! noting for the never-panic invariant: every step here degrades to
//! `None` rather than erroring, since a missing/malformed `.git` (not a
//! repo at all, a submodule, a corrupt worktree pointer) is normal, not a
//! fault.

use std::path::{Path, PathBuf};

/// Walks up from `start` looking for a `.git` entry. A directory is a normal
/// repository; a file (worktrees, submodules) holds a `gitdir: <path>`
/// pointer to the real one, which may itself be relative to the file's own
/// directory.
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file() {
            let contents = std::fs::read_to_string(&candidate).ok()?;
            let pointer = contents.trim().strip_prefix("gitdir:")?.trim();
            return Some(d.join(pointer));
        }
        dir = d.parent();
    }
    None
}

/// The current branch for the repository containing `cwd`, or `None` — no
/// repository, an unreadable/malformed `HEAD`, or a `HEAD` pointing
/// somewhere other than `refs/heads/*` or a bare commit (a tag checkout,
/// say) all degrade to `None` rather than guessing.
///
/// A detached `HEAD` (a raw commit hash) renders as its first 7 hex
/// characters, matching the abbreviation most shell prompts and `git`
/// itself use.
pub fn branch_name(cwd: &str) -> Option<String> {
    let git_dir = find_git_dir(Path::new(cwd))?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return Some(branch).filter(|s| !s.is_empty()).map(str::to_string);
    }
    if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(head[..7].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_git_dir(repo_root: &Path, head_contents: &str) {
        let git_dir = repo_root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), head_contents).unwrap();
    }

    #[test]
    fn reads_the_branch_from_head() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "ref: refs/heads/main\n");
        assert_eq!(
            branch_name(dir.path().to_str().unwrap()),
            Some("main".to_string())
        );
    }

    #[test]
    fn branch_names_with_slashes_are_kept_whole() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "ref: refs/heads/feat/cost-chips\n");
        assert_eq!(
            branch_name(dir.path().to_str().unwrap()),
            Some("feat/cost-chips".to_string())
        );
    }

    #[test]
    fn walks_up_from_a_nested_subdirectory() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "ref: refs/heads/main\n");
        let nested = dir.path().join("src").join("deeply").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            branch_name(nested.to_str().unwrap()),
            Some("main".to_string())
        );
    }

    #[test]
    fn detached_head_renders_a_short_hash() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "1234567890abcdef1234567890abcdef12345678\n");
        assert_eq!(
            branch_name(dir.path().to_str().unwrap()),
            Some("1234567".to_string())
        );
    }

    #[test]
    fn no_git_dir_anywhere_is_none() {
        let dir = tempdir().unwrap();
        assert_eq!(branch_name(dir.path().to_str().unwrap()), None);
    }

    #[test]
    fn malformed_head_is_none() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "not a ref or a hash\n");
        assert_eq!(branch_name(dir.path().to_str().unwrap()), None);
    }

    #[test]
    fn empty_branch_name_is_none() {
        let dir = tempdir().unwrap();
        write_git_dir(dir.path(), "ref: refs/heads/\n");
        assert_eq!(branch_name(dir.path().to_str().unwrap()), None);
    }

    #[test]
    fn worktree_gitfile_pointer_is_followed() {
        let dir = tempdir().unwrap();
        let real_git_dir = dir.path().join("main-repo").join(".git");
        std::fs::create_dir_all(&real_git_dir).unwrap();
        std::fs::write(real_git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let worktree = dir.path().join("worktree-checkout");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real_git_dir.display()),
        )
        .unwrap();

        assert_eq!(
            branch_name(worktree.to_str().unwrap()),
            Some("main".to_string())
        );
    }

    #[test]
    fn nonexistent_cwd_is_none() {
        assert_eq!(branch_name("/does/not/exist/at/all"), None);
    }
}
