//! Bakes the app version into the binary (3.39, plan.md → *Show the
//! version subtly in the UI*): runs `git describe --tags --always --dirty`
//! and passes the output to the crate as `FINANCI_VERSION`. When git is
//! missing, this isn't a repository, or nothing came out, the env var is
//! empty and `version::version_from_git_describe` falls back to
//! `CARGO_PKG_VERSION` (that choice lives there so it is unit-tested).

use std::path::Path;
use std::process::Command;

/// Run git in the crate directory and return its trimmed stdout, or `None`
/// when git is unavailable, fails, or printed nothing.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when a commit or tag moves `git describe`. `.git` may be a
    // file, not a directory (a linked worktree), so ask git itself where
    // HEAD and the refs live and fall back to the in-tree paths. A path
    // that doesn't exist only means fewer rebuild triggers, never an error.
    for key in ["HEAD", "refs"] {
        let path = git(&["rev-parse", "--git-path", key]).unwrap_or_else(|| format!(".git/{key}"));
        let path = Path::new(&path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }
    // "v0.17.0", "v0.17.0-3-gabc1234-dirty"…, or empty when git failed.
    let version = git(&["describe", "--tags", "--always", "--dirty"]).unwrap_or_default();
    println!("cargo:rustc-env=FINANCI_VERSION={version}");
}
