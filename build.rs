use std::{env, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-env-changed=LIYU_GIT_HASH");
    println!("cargo:rerun-if-env-changed=LIYU_GIT_DIRTY");
    for path in [
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
        "src",
        "migrations",
        "web",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let root = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let root = Path::new(&root);
    // An extracted source archive inside another repository is not that repository's build.
    let local_git = git(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| Path::new(&path).canonicalize().ok())
        == root.canonicalize().ok();
    if local_git {
        for name in ["HEAD", "index", "packed-refs"] {
            if let Some(path) = git(root, &["rev-parse", "--git-path", name]) {
                println!("cargo:rerun-if-changed={}", root.join(path).display());
            }
        }
        if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
            if let Some(path) = git(root, &["rev-parse", "--git-path", &reference]) {
                println!("cargo:rerun-if-changed={}", root.join(path).display());
            }
        }
    }
    let supplied = env::var("LIYU_GIT_HASH").ok().filter(|v| !v.is_empty());
    let hash = supplied.clone().or_else(|| {
        local_git
            .then(|| git(root, &["rev-parse", "HEAD"]))
            .flatten()
    });
    let hash = hash
        .map(|hash| {
            assert!(
                (hash.len() == 40 || hash.len() == 64)
                    && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "LIYU_GIT_HASH must be a full Git commit hash"
            );
            hash.to_ascii_lowercase()
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let dirty = env::var("LIYU_GIT_DIRTY")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|value| {
            assert!(
                value == "true" || value == "false",
                "LIYU_GIT_DIRTY must be true or false"
            );
            value
        })
        .or_else(|| {
            if supplied.is_none() && local_git {
                git(root, &["status", "--porcelain", "--untracked-files=normal"])
                    .map(|status| (!status.is_empty()).to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=LIYU_BUILD_GIT_HASH={hash}");
    println!("cargo:rustc-env=LIYU_BUILD_GIT_DIRTY={dirty}");
}
