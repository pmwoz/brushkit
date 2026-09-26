//! The README usage block and examples/readme.rs are the same code, so the
//! README compiles whenever the example does.

use std::path::Path;

fn readme() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [manifest.join("README.md"), manifest.join("../../README.md")]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .expect("README.md at the manifest or the workspace root")
}

#[test]
fn readme_usage_block_is_the_example() {
    let readme = readme();
    let block = readme
        .split("```rust\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("a rust block in README.md");

    let example = include_str!("../examples/readme.rs");
    let body = example
        .lines()
        .skip_while(|line| line.starts_with("//!") || line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    assert_eq!(
        block, body,
        "README.md usage block differs from examples/readme.rs"
    );
}

/// A release commit bumps the version in Cargo.toml, and the README
/// dependency line has to move with it.
#[test]
fn readme_dependency_line_is_the_current_version() {
    let version = env!("CARGO_PKG_VERSION");
    let major_minor = version
        .rsplit_once('.')
        .map_or(version, |(major_minor, _patch)| major_minor);
    let expected = format!("brushkit = \"{major_minor}\"");
    assert!(
        readme().lines().any(|line| line == expected),
        "README.md should carry `{expected}`"
    );
}
