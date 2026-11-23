use std::{env, path::Path, process::Command};

fn main() {
	println!("cargo:rerun-if-changed=src/");
	println!("cargo:rerun-if-env-changed=GIT_HASH");
	println!("cargo:rerun-if-env-changed=REDLIB_RELEASE");
	// The Dockerfile passes an empty hash when it got none.
	let hash = env::var("GIT_HASH").ok().filter(|hash| !hash.is_empty()).or_else(git_hash);
	let release = env::var("REDLIB_RELEASE").is_ok_and(|v| !v.is_empty());
	// Builds without any git information (distro builds from a release tarball) show the release version.
	let version = match &hash {
		Some(hash) if !release => format!("dev-{}", hash.get(..7).unwrap_or(hash)),
		_ => env::var("CARGO_PKG_VERSION").unwrap(),
	};
	println!("cargo:rustc-env=GIT_HASH={}", hash.as_deref().unwrap_or("dev"));
	println!("cargo:rustc-env=REDLIB_VERSION={version}");
}

fn git(arg: &str) -> Option<String> {
	let output = Command::new("git").args(["rev-parse", arg]).output().ok().filter(|o| o.status.success())?;
	Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

// Only a checkout of this repo counts, not a tarball unpacked inside another repo (e.g. a distro's package tree).
fn git_hash() -> Option<String> {
	let top = Path::new(&git("--show-toplevel")?).canonicalize().ok()?;
	if top != Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().ok()? {
		return None;
	}
	git("HEAD")
}
