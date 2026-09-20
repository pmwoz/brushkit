use std::path::{Path, PathBuf};

fn abr_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            abr_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("abr"))
        {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_pack_parses() {
    let Some(root) = std::env::var_os("BRUSHKIT_CORPUS_DIR").map(PathBuf::from) else {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    };
    if !root.is_dir() {
        println!("skip: BRUSHKIT_CORPUS_DIR unset");
        return;
    }

    let mut files = Vec::new();
    abr_files(&root, &mut files);
    files.sort();
    for path in &files {
        let bytes = std::fs::read(path).expect("corpus file is readable");
        let name = path.strip_prefix(&root).unwrap_or(path).display();
        assert!(
            brushkit_abr::parse_abr(&bytes).is_ok(),
            "{name} failed to parse"
        );
    }
    println!("parsed {} corpus pack(s)", files.len());
}
