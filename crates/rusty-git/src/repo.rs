use git2::Repository;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub added:    u32,
    pub modified: u32,
    pub deleted:  u32,
    pub untracked: u32,
}

/// Everything the UI needs to render Git context for a pane's CWD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub branch:     String,
    pub ahead:      u32,
    pub behind:     u32,
    pub status:     GitStatus,
    pub is_dirty:   bool,
}

impl GitInfo {
    /// Cheaply gather info from the repo containing `path`.
    /// Returns None if `path` is not inside a git repo.
    pub fn from_path(path: &Path) -> Option<Self> {
        let repo = Repository::discover(path).ok()?;
        let branch = current_branch(&repo).unwrap_or_else(|| "HEAD".into());

        let mut status = GitStatus::default();
        if let Ok(statuses) = repo.statuses(None) {
            for entry in statuses.iter() {
                let s = entry.status();
                if s.contains(git2::Status::INDEX_NEW) || s.contains(git2::Status::WT_NEW) {
                    if s.contains(git2::Status::WT_NEW) { status.untracked += 1; }
                    else { status.added += 1; }
                }
                if s.contains(git2::Status::INDEX_MODIFIED) || s.contains(git2::Status::WT_MODIFIED) {
                    status.modified += 1;
                }
                if s.contains(git2::Status::INDEX_DELETED) || s.contains(git2::Status::WT_DELETED) {
                    status.deleted += 1;
                }
            }
        }

        let (ahead, behind) = upstream_counts(&repo).unwrap_or((0, 0));
        let is_dirty = status.added + status.modified + status.deleted + status.untracked > 0;

        Some(GitInfo { branch, ahead, behind, status, is_dirty })
    }
}

fn current_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    Some(head.shorthand()?.to_owned())
}

fn upstream_counts(repo: &Repository) -> Option<(u32, u32)> {
    let head       = repo.head().ok()?;
    let local_oid  = head.target()?;
    let branch_name = head.shorthand()?;
    let upstream   = repo.find_branch(
        &format!("origin/{}", branch_name),
        git2::BranchType::Remote,
    ).ok()?;
    let upstream_oid = upstream.get().target()?;
    let (a, b) = repo.graph_ahead_behind(local_oid, upstream_oid).ok()?;
    Some((a as u32, b as u32))
}
