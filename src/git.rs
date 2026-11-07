use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Context,
    Added,
    Removed,
}

#[derive(PartialEq)]
pub struct Line {
    pub kind: Kind,
    pub old: u32,
    pub new: u32,
    pub text: String,
}

#[derive(PartialEq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub context: String,
    pub lines: Vec<Line>,
}

#[derive(PartialEq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: Status,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub removed: usize,
}

pub struct Repo {
    pub name: String,
    pub branch: String,
}

fn git(args: &[&str]) -> Result<(i32, String)> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .output()
        .context("failed to run git")?;
    let code = out.status.code().unwrap_or(-1);
    if code > 1 {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok((code, String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// For commands that change the repo: any failure becomes an error carrying git's first output line.
fn run(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("failed to run git")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let line = stderr.lines().chain(stdout.lines()).map(str::trim).find(|l| !l.is_empty());
        bail!("{}", line.unwrap_or("git failed"));
    }
    Ok(stdout)
}

/// Stages and commits exactly `paths`, leaving anything else staged untouched. Returns the short hash.
pub fn commit(message: &str, paths: &[&str]) -> Result<String> {
    // `git add` rejects a path that is gone from both disk and index (an already-staged delete).
    let (present, gone): (Vec<&str>, Vec<&str>) = paths.iter().partition(|p| std::path::Path::new(p).exists());
    if !gone.is_empty() {
        run(&[&["rm", "-q", "--cached", "--ignore-unmatch", "--"], &gone[..]].concat())?;
    }
    if !present.is_empty() {
        run(&[&["add", "-A", "--"], &present[..]].concat())?;
    }
    run(&[&["commit", "-m", message, "--"], paths].concat())?;
    Ok(run(&["rev-parse", "--short", "HEAD"])?.trim().to_string())
}

pub fn push(branch: &str) -> Result<()> {
    if run(&["rev-parse", "-q", "--verify", "HEAD"]).is_err() {
        bail!("Nothing to push yet, make a commit first");
    }
    if upstream().is_some() {
        run(&["push"])?;
    } else {
        run(&["push", "-u", "origin", branch])?;
    }
    Ok(())
}

/// Upstream branch name and how many local commits it is missing.
pub fn upstream() -> Option<(String, usize)> {
    let name = run(&["rev-parse", "--abbrev-ref", "@{u}"]).ok()?.trim().to_string();
    let ahead = run(&["rev-list", "--count", "@{u}..HEAD"]).ok()?.trim().parse().ok()?;
    Some((name, ahead))
}

/// Moves into the repo root so every path git prints is root-relative.
pub fn open() -> Result<Repo> {
    let (code, root) = git(&["rev-parse", "--show-toplevel"])?;
    if code != 0 {
        bail!("not a git repository");
    }
    let root = root.trim();
    std::env::set_current_dir(root)?;
    let name = root.rsplit('/').next().unwrap_or(root).to_string();
    let branch = git(&["branch", "--show-current"])?.1.trim().to_string();
    Ok(Repo { name, branch: if branch.is_empty() { "HEAD".into() } else { branch } })
}

/// With no args: everything changed since HEAD, untracked files included.
/// With args: passed straight through to `git diff`.
pub fn load(args: &[String]) -> Result<Vec<FileDiff>> {
    let mut cmd = vec!["diff", "--no-color", "--no-ext-diff", "-U3"];
    if !args.is_empty() {
        cmd.extend(args.iter().map(String::as_str));
        return Ok(parse(&git(&cmd)?.1));
    }
    let has_head = git(&["rev-parse", "-q", "--verify", "HEAD"])?.0 == 0;
    cmd.push(if has_head { "HEAD" } else { EMPTY_TREE });
    let mut files = parse(&git(&cmd)?.1);
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?.1;
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        let (_, out) = git(&["diff", "--no-color", "--no-index", "--", "/dev/null", path])?;
        files.extend(parse(&out));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_left, mut new_left) = (0u32, 0u32);
    let (mut old_no, mut new_no) = (0u32, 0u32);

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, p)| p);
            files.push(FileDiff {
                path: path.to_string(),
                old_path: None,
                status: Status::Modified,
                binary: false,
                hunks: Vec::new(),
                added: 0,
                removed: 0,
            });
            (old_left, new_left) = (0, 0);
            continue;
        }
        let Some(file) = files.last_mut() else { continue };

        if old_left > 0 || new_left > 0 {
            let hunk = file.hunks.last_mut().expect("counts imply a hunk");
            let (kind, body) = match line.as_bytes().first() {
                Some(b'+') => (Kind::Added, &line[1..]),
                Some(b'-') => (Kind::Removed, &line[1..]),
                Some(b' ') => (Kind::Context, &line[1..]),
                _ => (Kind::Context, line),
            };
            let text = body.replace('\t', "    ").replace('\r', "");
            match kind {
                Kind::Added => {
                    hunk.lines.push(Line { kind, old: 0, new: new_no, text });
                    new_no += 1;
                    new_left = new_left.saturating_sub(1);
                    file.added += 1;
                }
                Kind::Removed => {
                    hunk.lines.push(Line { kind, old: old_no, new: 0, text });
                    old_no += 1;
                    old_left = old_left.saturating_sub(1);
                    file.removed += 1;
                }
                Kind::Context => {
                    hunk.lines.push(Line { kind, old: old_no, new: new_no, text });
                    old_no += 1;
                    new_no += 1;
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                }
            }
        } else if let Some((hunk, new_start, new_len)) = parse_hunk_header(line) {
            (old_no, new_no) = (hunk.old_start, new_start);
            (old_left, new_left) = (hunk.old_len, new_len);
            file.hunks.push(hunk);
        } else if line.starts_with("new file mode") {
            file.status = Status::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = Status::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            file.old_path = Some(from.to_string());
            file.status = Status::Renamed;
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.path = to.to_string();
        } else if let Some(to) = line.strip_prefix("+++ b/") {
            file.path = to.to_string();
        } else if line.starts_with("Binary files") {
            file.binary = true;
        }
    }
    files
}

fn parse_hunk_header(line: &str) -> Option<(Hunk, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, context) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = range(old)?;
    let (new_start, new_len) = range(new)?;
    let hunk = Hunk { old_start, old_len, context: context.trim().to_string(), lines: Vec::new() };
    Some((hunk, new_start, new_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modified_file() {
        let diff = "diff --git a/src/a b.rs b/src/a b.rs\nindex 1..2 100644\n--- a/src/a b.rs\n+++ b/src/a b.rs\n@@ -10,3 +10,4 @@ fn main() {\n ctx\n--- removed\n+added\n+more\n";
        let files = parse(diff);
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "src/a b.rs");
        assert_eq!((f.added, f.removed), (2, 1));
        let h = &f.hunks[0];
        assert_eq!(h.context, "fn main() {");
        assert_eq!(h.lines[1].text, "-- removed");
        assert_eq!(h.lines[1].old, 11);
        assert_eq!(h.lines[3].new, 12);
    }

    #[test]
    fn parses_new_and_renamed() {
        let diff = "diff --git a/x b/x\nnew file mode 100644\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+hi\ndiff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";
        let files = parse(diff);
        assert!(files[0].status == Status::Added && files[0].hunks[0].lines.len() == 1);
        assert!(files[1].status == Status::Renamed && files[1].path == "new");
    }
}
