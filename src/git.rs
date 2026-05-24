// git.rs — data layer: loads commits, diffs, blame via libgit2 (git2 crate).

use git2::{DiffOptions, Repository, Sort};
use std::collections::HashMap;

// ──────────────────────────────────────────────
// Public data types
// ──────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub body: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub date_str: String,
    pub parents: Vec<String>,
    pub refs: Vec<RefLabel>,
}

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RefKind {
    Head,       // the current HEAD branch
    Branch,
    Tag,
    Remote,
}

#[derive(Clone, Debug)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>, // for renames
    pub status: char,             // A / M / D / R / ?
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Default)]
pub struct CommitDiff {
    pub files: Vec<DiffFile>,
    pub patch: String,
}

#[derive(Clone, Debug)]
pub struct BlameLine {
    pub commit_id: String,
    pub short_id: String,
    pub author: String,
    pub date: String,
    pub lineno: usize,
    pub content: String,
}

// ──────────────────────────────────────────────
// Repository
// ──────────────────────────────────────────────

pub struct GitRepo {
    pub repo: Repository,
    pub path: String,
}

impl GitRepo {
    pub fn open(path: &str) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        Ok(GitRepo { repo, path: path.to_string() })
    }

    pub fn name(&self) -> String {
        self.repo
            .workdir()
            .or_else(|| Some(self.repo.path()))
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.clone())
    }

    pub fn head_branch(&self) -> String {
        self.repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .unwrap_or_else(|| "HEAD".to_string())
    }

    // ── Ref map ───────────────────────────────
    fn build_ref_map(&self) -> HashMap<String, Vec<RefLabel>> {
        let mut map: HashMap<String, Vec<RefLabel>> = HashMap::new();

        let head_oid = self.repo.head().ok()
            .and_then(|h| h.resolve().ok())
            .and_then(|h| h.target())
            .map(|o| o.to_string());

        if let Ok(refs) = self.repo.references() {
            for r in refs.flatten() {
                let full = r.name().unwrap_or("").to_string();
                let short = r.shorthand().unwrap_or("?").to_string();
                let target = r.resolve().ok()
                    .and_then(|r| r.target())
                    .map(|o| o.to_string());

                let kind = if full.starts_with("refs/tags/") {
                    RefKind::Tag
                } else if full.starts_with("refs/remotes/") {
                    RefKind::Remote
                } else if head_oid.as_deref() == target.as_deref()
                    && self.repo.head().ok()
                        .and_then(|h| h.shorthand().map(|s| s.to_string()))
                        .as_deref() == Some(&short)
                {
                    RefKind::Head
                } else {
                    RefKind::Branch
                };

                if let Some(oid) = target {
                    map.entry(oid).or_default().push(RefLabel { name: short, kind });
                }
            }
        }
        map
    }

    // ── Commit list ───────────────────────────
    pub fn load_commits(&self, max: usize) -> Vec<Commit> {
        let mut revwalk = match self.repo.revwalk() {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).ok();
        revwalk.push_glob("refs/heads/*").ok();
        revwalk.push_glob("refs/tags/*").ok();
        revwalk.push_glob("refs/remotes/*").ok();
        if let Ok(head) = self.repo.head() {
            if let Some(oid) = head.target() {
                revwalk.push(oid).ok();
            }
        }

        let ref_map = self.build_ref_map();
        let mut commits = Vec::new();

        for oid in revwalk.flatten().take(max) {
            if let Ok(c) = self.repo.find_commit(oid) {
                let id = oid.to_string();
                let short_id = id[..8].to_string();
                let summary = c.summary().unwrap_or("").to_string();
                let body = c.body().unwrap_or("").to_string();
                let sig = c.author();
                let author = sig.name().unwrap_or("?").to_string();
                let email = sig.email().unwrap_or("").to_string();
                let timestamp = sig.when().seconds();
                let date_str = format_timestamp(timestamp);
                let parents = c.parent_ids().map(|p| p.to_string()).collect();
                let refs = ref_map.get(&id).cloned().unwrap_or_default();

                commits.push(Commit {
                    id, short_id, summary, body,
                    author, email, timestamp, date_str,
                    parents, refs,
                });
            }
        }
        commits
    }

    // ── Diff ──────────────────────────────────
    pub fn load_diff(&self, commit_id: &str) -> CommitDiff {
        let oid = match git2::Oid::from_str(commit_id) {
            Ok(o) => o,
            Err(_) => return CommitDiff::default(),
        };
        let commit = match self.repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => return CommitDiff::default(),
        };
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => return CommitDiff::default(),
        };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let mut opts = DiffOptions::new();
        opts.context_lines(3);

        let diff = match parent_tree {
            Some(pt) => self.repo.diff_tree_to_tree(Some(&pt), Some(&tree), Some(&mut opts)),
            None     => self.repo.diff_tree_to_tree(None, Some(&tree), Some(&mut opts)),
        };
        let diff = match diff {
            Ok(d) => d,
            Err(_) => return CommitDiff::default(),
        };

        let mut patch_buf = String::new();
        diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
            let o = line.origin();
            if matches!(o, '+' | '-' | ' ') { patch_buf.push(o); }
            if let Ok(s) = std::str::from_utf8(line.content()) {
                patch_buf.push_str(s);
            }
            true
        }).ok();

        let mut files: Vec<DiffFile> = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                let status = match delta.status() {
                    git2::Delta::Added    => 'A',
                    git2::Delta::Deleted  => 'D',
                    git2::Delta::Modified => 'M',
                    git2::Delta::Renamed  => 'R',
                    git2::Delta::Copied   => 'C',
                    _                     => '?',
                };
                let path = delta.new_file().path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let old_path = if status == 'R' {
                    delta.old_file().path().map(|p| p.display().to_string())
                } else { None };
                files.push(DiffFile { path, old_path, status, additions: 0, deletions: 0 });
                true
            },
            None,
            None,
            Some(&mut |delta, _hunk, line| {
                let np = delta.new_file().path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if let Some(f) = files.iter_mut().find(|f| f.path == np) {
                    match line.origin() {
                        '+' => f.additions += 1,
                        '-' => f.deletions += 1,
                        _   => {}
                    }
                }
                true
            }),
        ).ok();

        CommitDiff { files, patch: patch_buf }
    }

    // ── Blame ─────────────────────────────────
    pub fn blame_file(&self, file_path: &str, commit_id: Option<&str>) -> Vec<BlameLine> {
        let mut opts = git2::BlameOptions::new();
        if let Some(id) = commit_id {
            if let Ok(oid) = git2::Oid::from_str(id) {
                opts.newest_commit(oid);
            }
        }
        let blame = match self.repo.blame_file(std::path::Path::new(file_path), Some(&mut opts)) {
            Ok(b) => b,
            Err(_) => return vec![],
        };

        // Read file content at that commit (or HEAD)
        let content = self.read_file_at(file_path, commit_id).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();

        let mut result = Vec::new();
        for (lineno, text) in lines.iter().enumerate() {
            let hunk = match blame.get_line(lineno + 1) {
                Some(h) => h,
                None    => continue,
            };
            let sig = hunk.final_signature();
            let cid = hunk.final_commit_id().to_string();
            let short_id = cid[..8.min(cid.len())].to_string();
            let author = sig.name().unwrap_or("?").to_string();
            let date = format_timestamp(sig.when().seconds());

            result.push(BlameLine {
                commit_id: cid,
                short_id,
                author,
                date,
                lineno: lineno + 1,
                content: text.to_string(),
            });
        }
        result
    }

    fn read_file_at(&self, file_path: &str, commit_id: Option<&str>) -> Option<String> {
        let commit = if let Some(id) = commit_id {
            let oid = git2::Oid::from_str(id).ok()?;
            self.repo.find_commit(oid).ok()?
        } else {
            let head = self.repo.head().ok()?;
            self.repo.find_commit(head.target()?).ok()?
        };
        let tree = commit.tree().ok()?;
        let entry = tree.get_path(std::path::Path::new(file_path)).ok()?;
        let blob = self.repo.find_blob(entry.id()).ok()?;
        std::str::from_utf8(blob.content()).ok().map(|s| s.to_string())
    }

    // ── Branches / tags ───────────────────────
    pub fn all_branches(&self) -> Vec<String> {
        let mut out = vec!["(all)".to_string()];
        if let Ok(refs) = self.repo.references() {
            let mut names: Vec<String> = refs.flatten()
                .filter_map(|r| {
                    let n = r.shorthand()?.to_string();
                    if r.is_branch() || r.is_remote() { Some(n) } else { None }
                })
                .collect();
            names.sort();
            names.dedup();
            out.extend(names);
        }
        out
    }
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

pub fn format_timestamp(ts: i64) -> String {
    use chrono::{DateTime, Local, TimeZone};
    let dt: DateTime<Local> = Local.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Local::now());
    dt.format("%Y-%m-%d %H:%M").to_string()
}
