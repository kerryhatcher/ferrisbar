//! Resolves which git repository a working directory belongs to, for
//! analytics indexing: a normalized `origin` remote when one is
//! configured, otherwise the repository's own root folder name. See
//! docs/superpowers/specs/2026-08-11-repo-cost-analytics-design.md.
//!
//! This module itself is not feature-gated (it's plain, dependency-free
//! Rust), but its only real callers — `analytics::store::Sink::record` and
//! `main.rs`'s `run_report` — are both behind `#[cfg(feature = "analytics")]`.
//! In a default build `resolve` and everything it calls are genuinely
//! unreachable outside tests, hence the module-wide allow below rather than
//! one scattered across every function.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub key: String,
    pub display: String,
}

/// Resolves a worktree/submodule gitdir (e.g. `.git/worktrees/<name>`) to
/// the *common* git directory shared by every worktree of the same
/// repository, by following its `commondir` file. An ordinary repository's
/// `.git` has no `commondir` file and resolves to itself unchanged.
fn resolve_common_git_dir(git_dir: &Path) -> PathBuf {
    let Ok(contents) = std::fs::read_to_string(git_dir.join("commondir")) else {
        return git_dir.to_path_buf();
    };
    let relative = contents.trim();
    if relative.is_empty() {
        return git_dir.to_path_buf();
    }
    let joined = git_dir.join(relative);
    // Canonicalize to resolve .. and . components, but if that fails
    // (e.g., path doesn't exist), fall back to the joined path.
    joined.canonicalize().unwrap_or(joined)
}

/// Strips scheme, embedded `user[:token]@`, and a trailing `.git`, unifying
/// `git@host:org/repo.git`, `ssh://git@host/org/repo.git`, and
/// `https://host/org/repo(.git)?` down to the same `host/org/repo` string.
/// `None` for an empty or unparseable URL.
fn normalize_remote_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let has_scheme = trimmed.contains("://");
    let without_scheme = trimmed.split("://").last().unwrap_or(trimmed);
    // The last '@' strips both a URL's `user[:token]@host` and the
    // scp-like shorthand's `user@host:path` down to whatever follows it.
    let host_and_path = without_scheme
        .rfind('@')
        .map_or(without_scheme, |i| &without_scheme[i + 1..]);
    let normalized = if has_scheme {
        host_and_path.to_string()
    } else {
        // scp-like shorthand uses `:` where a URL uses `/` between host
        // and path — swap only the first one so both forms end up alike.
        host_and_path.replacen(':', "/", 1)
    };
    let normalized = normalized.strip_suffix(".git").unwrap_or(&normalized);
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_lowercase())
    }
}

/// Reads `.git/config`'s `[remote "origin"]` section for its `url`. `None`
/// for a missing/unreadable config, or one with no origin remote — both
/// are normal, not faults.
fn read_origin_url(git_dir: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_origin_section = trimmed.eq_ignore_ascii_case("[remote \"origin\"]");
            continue;
        }
        if !in_origin_section {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("url") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// The last path component of `.git`'s parent (the repository root),
/// falling back to `"unknown"` for the practically-impossible case of a
/// root path with no final component at all.
fn root_folder_name(git_dir: &Path) -> String {
    git_dir.parent().and_then(Path::file_name).map_or_else(
        || "unknown".to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `cwd`'s own last path component, falling back to `"unknown"` the same
/// way `root_folder_name` does. Used when `cwd` is not inside any git
/// repository at all — there is no repo root to name, so `cwd` itself
/// stands in for it.
fn cwd_folder_name(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or_else(
        || "unknown".to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Resolves `cwd`'s repository identity. Always returns a usable
/// `RepoIdentity` — a missing `.git`, an unreadable config, or an
/// unparseable remote URL all degrade to a `local:` identity rather than
/// failing.
pub fn resolve(cwd: &str) -> RepoIdentity {
    let Some(git_dir) = crate::git::find_git_dir(Path::new(cwd)) else {
        let name = cwd_folder_name(cwd);
        return RepoIdentity {
            key: format!("local:{name}"),
            display: name,
        };
    };
    // For worktrees/submodules, resolve to the common git directory
    // so we can read the actual config and get the correct repo name.
    let common_git_dir = resolve_common_git_dir(&git_dir);
    if let Some(origin) =
        read_origin_url(&common_git_dir).and_then(|raw| normalize_remote_url(&raw))
    {
        return RepoIdentity {
            key: format!("remote:{origin}"),
            display: origin,
        };
    }
    let name = root_folder_name(&common_git_dir);
    RepoIdentity {
        key: format!("local:{name}"),
        display: name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_git_dir(repo_root: &Path, origin_url: Option<&str>) {
        let git_dir = repo_root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let config = origin_url.map_or_else(String::new, |url| {
            format!("[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n")
        });
        std::fs::write(git_dir.join("config"), config).unwrap();
    }

    #[test]
    fn ssh_https_and_scp_forms_of_the_same_remote_resolve_to_the_same_key() {
        let cases = [
            "git@github.com:kerryhatcher/ferrisbar.git",
            "ssh://git@github.com/kerryhatcher/ferrisbar.git",
            "https://github.com/kerryhatcher/ferrisbar.git",
            "https://github.com/kerryhatcher/ferrisbar",
        ];
        let mut keys = Vec::new();
        for url in cases {
            let dir = tempfile::tempdir().unwrap();
            write_git_dir(dir.path(), Some(url));
            keys.push(resolve(dir.path().to_str().unwrap()).key);
        }
        for key in &keys[1..] {
            assert_eq!(
                key, &keys[0],
                "all four URL forms must normalize identically"
            );
        }
        assert_eq!(keys[0], "remote:github.com/kerryhatcher/ferrisbar");
    }

    #[test]
    fn embedded_credentials_are_stripped() {
        let dir = tempfile::tempdir().unwrap();
        write_git_dir(
            dir.path(),
            Some("https://user:token@github.com/org/repo.git"),
        );
        let identity = resolve(dir.path().to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/org/repo");
    }

    #[test]
    fn no_origin_remote_falls_back_to_the_repo_root_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path().join("my-local-project");
        std::fs::create_dir_all(&repo_root).unwrap();
        write_git_dir(&repo_root, None);
        let identity = resolve(repo_root.to_str().unwrap());
        assert_eq!(identity.key, "local:my-local-project");
        assert_eq!(identity.display, "my-local-project");
    }

    #[test]
    fn no_git_repo_at_all_falls_back_to_the_cwd_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let leaf = dir.path().join("scratch-dir");
        std::fs::create_dir_all(&leaf).unwrap();
        let identity = resolve(leaf.to_str().unwrap());
        assert_eq!(identity.key, "local:scratch-dir");
    }

    #[test]
    fn resolution_walks_up_from_a_nested_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        write_git_dir(dir.path(), Some("https://github.com/a/b.git"));
        let nested = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        let identity = resolve(nested.to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/a/b");
    }

    #[test]
    fn nonexistent_cwd_still_resolves_without_panicking() {
        let identity = resolve("/does/not/exist/at/all");
        assert_eq!(identity.key, "local:all");
        let _: PathBuf = PathBuf::new(); // silence unused-import warning if any
    }

    #[test]
    fn worktree_with_origin_resolves_to_remote_identity() {
        let dir = tempfile::tempdir().unwrap();
        let main_repo = dir.path().join("main-repo");
        std::fs::create_dir_all(&main_repo).unwrap();

        // Create main repo's .git with a config containing origin remote
        let main_git = main_repo.join(".git");
        std::fs::create_dir_all(&main_git).unwrap();
        let config = "[remote \"origin\"]\n\turl = https://github.com/test/repo.git\n";
        std::fs::write(main_git.join("config"), config).unwrap();
        std::fs::write(main_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // Create worktree checkout
        let worktree_dir = dir.path().join("worktree-1");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        // Create worktree's .git/worktrees/<name> directory
        let worktree_git = main_git.join("worktrees").join("wt-1");
        std::fs::create_dir_all(&worktree_git).unwrap();

        // Write commondir to point back to main repo's .git
        std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();

        // Write .git file (gitdir pointer) in worktree checkout
        std::fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        // The worktree should resolve to the same remote identity as the main repo
        let identity = resolve(worktree_dir.to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/test/repo");
    }

    #[test]
    fn worktree_without_origin_resolves_to_local_with_main_repo_name() {
        let dir = tempfile::tempdir().unwrap();
        let main_repo = dir.path().join("my-project");
        std::fs::create_dir_all(&main_repo).unwrap();

        // Create main repo's .git without origin
        let main_git = main_repo.join(".git");
        std::fs::create_dir_all(&main_git).unwrap();
        std::fs::write(main_git.join("config"), "").unwrap();
        std::fs::write(main_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // Create worktree checkout
        let worktree_dir = dir.path().join("worktree-2");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        // Create worktree's .git/worktrees/<name> directory
        let worktree_git = main_git.join("worktrees").join("wt-2");
        std::fs::create_dir_all(&worktree_git).unwrap();

        // Write commondir to point back to main repo's .git
        std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();

        // Write .git file (gitdir pointer) in worktree checkout
        std::fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        // The worktree should use the main repo's folder name, not "worktrees"
        let identity = resolve(worktree_dir.to_str().unwrap());
        assert_eq!(identity.key, "local:my-project");
        assert_eq!(identity.display, "my-project");
    }

    #[test]
    fn ordinary_repo_without_commondir_still_works() {
        let dir = tempfile::tempdir().unwrap();
        write_git_dir(dir.path(), Some("https://github.com/org/repo.git"));
        let identity = resolve(dir.path().to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/org/repo");
    }
}
