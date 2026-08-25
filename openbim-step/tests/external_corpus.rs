#![allow(missing_docs)]

use openbim_step::{is_step_file, parse, write_to_string};
use std::path::{Path, PathBuf};

fn files_under(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("corpus directory is readable") {
        let path = entry.expect("corpus entry is readable").path();
        if path.is_dir() {
            files_under(&path, output);
        } else {
            output.push(path);
        }
    }
}

#[test]
fn configured_external_corpus_semantically_roundtrips() {
    let Some(root) = std::env::var_os("STEP_CORPUS_DIR") else {
        return;
    };
    let mut files = Vec::new();
    files_under(Path::new(&root), &mut files);
    assert!(!files.is_empty(), "configured corpus is empty");

    let mut checked = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("could not read corpus file {}: {error}", path.display())
        });
        if !is_step_file(&bytes) {
            continue;
        }
        checked += 1;
        let exchange = parse(&bytes)
            .unwrap_or_else(|error| panic!("could not parse {}: {error}", path.display()));
        let output = write_to_string(&exchange)
            .unwrap_or_else(|error| panic!("could not write {}: {error}", path.display()));
        let reparsed = parse(output.as_bytes())
            .unwrap_or_else(|error| panic!("could not reparse {}: {error}", path.display()));
        assert_eq!(exchange, reparsed, "semantic drift in {}", path.display());
    }
    assert!(
        checked > 0,
        "configured corpus contains no STEP physical files"
    );
}
