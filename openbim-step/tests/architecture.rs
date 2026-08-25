#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn package_has_no_ifc_package_dependencies_or_imports() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest.join("Cargo.toml"))
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success());
    let metadata = String::from_utf8(output.stdout).unwrap();
    for dependency_fragment in metadata.match_indices("\"name\":\"") {
        let tail = &metadata[dependency_fragment.0 + dependency_fragment.1.len()..];
        let name = tail.split('"').next().unwrap();
        assert!(
            !name.starts_with("ifc-"),
            "forbidden package dependency: {name}"
        );
    }

    let mut sources = Vec::new();
    rust_sources(&manifest.join("src"), &mut sources);
    for source in sources {
        let text = std::fs::read_to_string(&source).unwrap();
        assert!(
            !text.contains("use ifc_"),
            "forbidden import in {}",
            source.display()
        );
        assert!(
            !text.contains("extern crate ifc_"),
            "forbidden import in {}",
            source.display()
        );
    }
}
