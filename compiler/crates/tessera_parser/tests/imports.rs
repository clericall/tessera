//! Import expansion: recursive splicing, include-once, cycles, diagnostics.

use std::fs;
use std::path::PathBuf;

use tessera_ast::ItemKind;
use tessera_parser::parse_file;

/// A throwaway directory per test so tests never see each other's files.
fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tesserac_import_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (rel, body) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }
    dir
}

fn fn_names(path: &PathBuf) -> Vec<String> {
    let (module, _root_errs, _imported) = parse_file(path);
    module
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn imports_splice_items_in_order() {
    let dir = fixture(
        "order",
        &[
            ("a.ter", "fn a() -> i64 { return 1; }\n"),
            ("main.ter", "import \"a.ter\";\nfn main() -> i64 { return a(); }\n"),
        ],
    );
    assert_eq!(fn_names(&dir.join("main.ter")), vec!["a", "main"]);
}

#[test]
fn nested_imports_resolve_against_the_importing_file() {
    let dir = fixture(
        "nested",
        &[
            ("sub/math.ter", "fn addup(a: i64, b: i64) -> i64 { return a + b; }\n"),
            ("sub/more.ter", "import \"math.ter\";\nfn twice(a: i64) -> i64 { return addup(a, a); }\n"),
            ("main.ter", "import \"sub/more.ter\";\nfn main() -> i64 { return twice(3); }\n"),
        ],
    );
    assert_eq!(fn_names(&dir.join("main.ter")), vec!["addup", "twice", "main"]);
}

#[test]
fn a_module_is_included_only_once() {
    let dir = fixture(
        "diamond",
        &[
            ("util.ter", "fn util() -> i64 { return 1; }\n"),
            ("left.ter", "import \"util.ter\";\nfn left() -> i64 { return util(); }\n"),
            ("right.ter", "import \"util.ter\";\nfn right() -> i64 { return util(); }\n"),
            (
                "main.ter",
                "import \"left.ter\";\nimport \"right.ter\";\nfn main() -> i64 { return left() + right(); }\n",
            ),
        ],
    );
    let names = fn_names(&dir.join("main.ter"));
    assert_eq!(names.iter().filter(|n| *n == "util").count(), 1, "duplicated: {names:?}");
    assert_eq!(names, vec!["util", "left", "right", "main"]);
}

#[test]
fn import_cycles_terminate() {
    let dir = fixture(
        "cycle",
        &[
            ("a.ter", "import \"b.ter\";\nfn a() -> i64 { return 1; }\n"),
            ("b.ter", "import \"a.ter\";\nfn b() -> i64 { return 2; }\n"),
        ],
    );
    let names = fn_names(&dir.join("a.ter"));
    assert_eq!(names.iter().filter(|n| *n == "a").count(), 1);
    assert_eq!(names.iter().filter(|n| *n == "b").count(), 1);
}

#[test]
fn self_import_terminates() {
    let dir = fixture("self", &[("s.ter", "import \"s.ter\";\nfn s() -> i64 { return 1; }\n")]);
    assert_eq!(fn_names(&dir.join("s.ter")), vec!["s"]);
}

#[test]
fn missing_module_is_reported_against_the_import_line() {
    let dir = fixture("missing", &[("main.ter", "import \"nope.ter\";\nfn main() -> i64 { return 0; }\n")]);
    let (_module, _root, imported) = parse_file(&dir.join("main.ter"));
    assert_eq!(imported.len(), 1);
    let d = &imported[0];
    assert!(d.err.message.contains("nope.ter"), "{}", d.err.message);
    assert_eq!(d.path, dir.join("main.ter").display().to_string());
}

#[test]
fn errors_inside_a_module_keep_their_own_location() {
    let dir = fixture(
        "inner_error",
        &[
            ("broken.ter", "fn broken( -> i64 { return 1; }\n"),
            ("main.ter", "import \"broken.ter\";\nfn main() -> i64 { return 0; }\n"),
        ],
    );
    let (_module, _root, imported) = parse_file(&dir.join("main.ter"));
    assert!(!imported.is_empty());
    let d = &imported[0];
    assert_eq!(d.path, dir.join("broken.ter").display().to_string());
    assert!(d.err.span.hi > d.err.span.lo);
    assert!(d.src.contains("fn broken("), "module source must travel with the diagnostic");
}

#[test]
fn imported_typedefs_and_structs_are_visible() {
    let dir = fixture(
        "types",
        &[
            ("types.ter", "type Sq = i64;\nstruct Vec2 { x: i64, y: i64 }\n"),
            ("main.ter", "import \"types.ter\";\nfn main() -> i64 { return 0; }\n"),
        ],
    );
    let (module, _root, _imported) = parse_file(&dir.join("main.ter"));
    assert!(module.items.iter().any(|i| matches!(&i.kind, ItemKind::TypeAlias { name, .. } if name == "Sq")));
    assert!(module.items.iter().any(|i| matches!(&i.kind, ItemKind::Struct(def) if def.name == "Vec2")));
}

#[test]
fn import_path_is_stored_without_quotes() {
    let dir = fixture("quotes", &[("main.ter", "import \"other.ter\";\n")]);
    let (module, _root, _imported) = parse_file(&dir.join("main.ter"));
    assert!(module.items.iter().all(|i| match &i.kind {
        ItemKind::Import { path } => !path.contains('"'),
        _ => true,
    }));
}