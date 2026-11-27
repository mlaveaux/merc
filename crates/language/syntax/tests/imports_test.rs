//! `%import` directives: scanning them and resolving them from the filesystem.

use std::fs;
use std::path::Path;

use merc_syntax::ImportError;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::scan_imports;
use merc_utilities::MercError;
use merc_utilities::SourceMap;
use merc_utilities::Span;

#[test]
fn test_scan_imports_finds_a_directive_line() {
    let text = "sort D;\n%import \"other.mcrl2\"\nmap f: D;\n";

    let directives = scan_imports(text);

    assert_eq!(directives.len(), 1);
    assert_eq!(directives[0].node.path, "other.mcrl2");
    assert_eq!(
        &text[directives[0].span.start..directives[0].span.end],
        "%import \"other.mcrl2\""
    );
}

#[test]
fn test_scan_imports_finds_every_directive_in_order() {
    let directives = scan_imports("%import \"a.mcrl2\"\nsort D;\n%import \"b.mcrl2\"\n");

    let paths: Vec<&str> = directives
        .iter()
        .map(|directive| directive.node.path.as_str())
        .collect();
    assert_eq!(paths, ["a.mcrl2", "b.mcrl2"]);
}

#[test]
fn test_scan_imports_ignores_ordinary_comments() {
    let text = "% just a comment\n%importance is not a directive\nsort D;\n";
    assert!(scan_imports(text).is_empty());
}

#[test]
fn test_scan_imports_ignores_a_directive_with_trailing_garbage() {
    assert!(scan_imports("%import \"a.mcrl2\" extra\n").is_empty());
}

#[test]
fn test_scan_imports_allows_leading_whitespace() {
    let directives = scan_imports("   %import \"a.mcrl2\"\n");
    assert_eq!(directives.len(), 1);
    assert_eq!(directives[0].node.path, "a.mcrl2");
}

#[test]
fn test_scan_imports_path_span_covers_just_the_quoted_path() {
    let text = "%import \"a.mcrl2\"\n";
    let directives = scan_imports(text);

    assert_eq!(directives.len(), 1);
    let path_span = &directives[0].node.path_span;
    assert_eq!(&text[path_span.start..path_span.end], "a.mcrl2");
}

/// Writes `files` (relative path, contents) into a fresh temporary directory.
fn temp_project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("should create a temp directory");
    for (name, contents) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("should create parent directories");
        }
        fs::write(path, contents).expect("should write the fixture file");
    }
    dir
}

/// The file name and the text that `span` refers to in `sources`.
fn locate<'a>(sources: &'a SourceMap, span: &Span) -> (&'a str, &'a str) {
    let id = sources.lookup(span.start);
    let base = sources.base_offset(id);
    let name = Path::new(sources.path(id))
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the source has a file name");
    (name, &sources.text(id)[span.start - base..span.end - base])
}

/// Downcasts `error` into the structured [ImportError].
fn as_import_error(error: &MercError) -> &ImportError {
    error
        .downcast_ref::<ImportError>()
        .expect("expected a structured ImportError")
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_merges_the_imported_declarations() {
    let dir = temp_project(&[
        ("main.mcrl2", "%import \"common.mcrl2\"\nmap g: D;\n"),
        ("common.mcrl2", "sort D;\n"),
    ]);

    let mut sources = SourceMap::new();
    let (spec, _graph) = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut sources)
        .expect("should resolve the import");

    assert_eq!(spec.to_string(), "sort\n   D;\n\nmap\n   g: D;\n\n");
    // Every declaration's span refers to the file it was declared in.
    assert_eq!(locate(&sources, &spec.sort_declarations[0].span), ("common.mcrl2", "D"));
    assert_eq!(
        locate(&sources, &spec.map_declarations[0].identifier.span),
        ("main.mcrl2", "g")
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_resolves_paths_relative_to_the_importing_file() {
    let dir = temp_project(&[
        ("main.mcrl2", "%import \"lib/a.mcrl2\"\n"),
        ("lib/a.mcrl2", "%import \"b.mcrl2\"\nsort A;\n"),
        ("lib/b.mcrl2", "sort B;\n"),
    ]);

    let mut sources = SourceMap::new();
    let (spec, graph) = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut sources)
        .expect("should resolve the nested import");

    let mut sorts: Vec<String> = spec.sort_declarations.iter().map(|decl| decl.to_string()).collect();
    sorts.sort();
    assert_eq!(sorts, ["A", "B"]);
    assert_eq!(graph.edges.len(), 2);
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_merges_a_diamond_import_once() {
    let dir = temp_project(&[
        ("main.mcrl2", "%import \"a.mcrl2\"\n%import \"b.mcrl2\"\n"),
        ("a.mcrl2", "%import \"common.mcrl2\"\n"),
        ("b.mcrl2", "%import \"common.mcrl2\"\n"),
        ("common.mcrl2", "sort D;\n"),
    ]);

    let mut sources = SourceMap::new();
    let (spec, graph) = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut sources)
        .expect("should resolve the diamond import");

    assert_eq!(spec.sort_declarations.len(), 1);

    // One edge per `%import` directive, so `common.mcrl2` has an edge from both `a` and `b`.
    assert!(sources.path(graph.root).ends_with("main.mcrl2"));
    let mut edges: Vec<(String, String)> = graph
        .edges
        .iter()
        .map(|(importer, _, imported)| {
            let name = |id| {
                Path::new(sources.path(id))
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("the source has a file name")
                    .to_string()
            };
            (name(*importer), name(*imported))
        })
        .collect();
    edges.sort();
    assert_eq!(
        edges,
        [
            ("a.mcrl2".to_string(), "common.mcrl2".to_string()),
            ("b.mcrl2".to_string(), "common.mcrl2".to_string()),
            ("main.mcrl2".to_string(), "a.mcrl2".to_string()),
            ("main.mcrl2".to_string(), "b.mcrl2".to_string()),
        ]
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_import_graph_edge_span_covers_the_importing_directive() {
    let main_text = "%import \"common.mcrl2\"\nmap g: D;\n";
    let dir = temp_project(&[("main.mcrl2", main_text), ("common.mcrl2", "sort D;\n")]);

    let mut sources = SourceMap::new();
    let (_spec, graph) = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut sources)
        .expect("should resolve the import");

    assert_eq!(graph.edges.len(), 1);
    let (importer, span, imported) = &graph.edges[0];
    assert_eq!(*importer, graph.root);
    assert_eq!(
        sources.path(*imported),
        dir.path().join("common.mcrl2").display().to_string()
    );
    assert_eq!(&main_text[span.start..span.end], "%import \"common.mcrl2\"");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_rejects_a_cycle() {
    let dir = temp_project(&[
        ("a.mcrl2", "%import \"b.mcrl2\"\n"),
        ("b.mcrl2", "%import \"a.mcrl2\"\n"),
    ]);

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("a.mcrl2"), &mut SourceMap::new())
        .expect_err("a cyclic import must be rejected");

    assert!(error.to_string().contains("import cycle detected"), "got: {error}");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_rejects_a_self_import() {
    let dir = temp_project(&[("a.mcrl2", "%import \"a.mcrl2\"\n")]);

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("a.mcrl2"), &mut SourceMap::new())
        .expect_err("a file importing itself must be rejected");

    assert!(error.to_string().contains("import cycle detected"), "got: {error}");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_reports_a_missing_import_as_a_structured_error() {
    let text = "%import \"missing.mcrl2\"\ninit delta;\n";
    let dir = temp_project(&[("main.mcrl2", text)]);

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut SourceMap::new())
        .expect_err("importing a nonexistent file must fail");

    let import_error = as_import_error(&error);
    let ImportError::Unresolved { path, .. } = import_error else {
        panic!("expected ImportError::Unresolved, got: {import_error:?}");
    };
    assert_eq!(path, "missing.mcrl2");
    assert_eq!(import_error.span(), Some(&Span::new(0, text.find('\n').unwrap())));
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_reports_a_transitively_missing_import_against_the_root_files_own_directive() {
    // `main.mcrl2` imports `common.mcrl2`, which itself imports something missing.
    let main_text = "%import \"common.mcrl2\"\ninit delta;\n";
    let dir = temp_project(&[
        ("main.mcrl2", main_text),
        ("common.mcrl2", "%import \"missing.mcrl2\"\n"),
    ]);

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut SourceMap::new())
        .expect_err("a transitively missing import must fail");

    let import_error = as_import_error(&error);
    let ImportError::Unresolved { path, cause, .. } = import_error else {
        panic!("expected ImportError::Unresolved, got: {import_error:?}");
    };
    assert_eq!(path, "common.mcrl2");
    assert_eq!(import_error.span(), Some(&Span::new(0, main_text.find('\n').unwrap())));
    assert!(
        import_error.to_string().contains("missing.mcrl2"),
        "expected the nested failure to be mentioned in the message, got: {import_error}"
    );

    // The nested failure is preserved as an `ImportError` in the `cause`.
    let nested = as_import_error(cause);
    let ImportError::Unresolved { path, .. } = nested else {
        panic!("expected ImportError::Unresolved, got: {nested:?}");
    };
    assert_eq!(path, "missing.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_reports_a_syntax_error_as_a_structured_syntax_error() {
    let dir = temp_project(&[("main.mcrl2", "sort D\n")]); // Missing the trailing `;`.

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut SourceMap::new())
        .expect_err("a syntax error must fail parsing");

    let import_error = as_import_error(&error);
    assert!(
        matches!(import_error, ImportError::Parse { .. }),
        "expected ImportError::Parse, got: {import_error:?}"
    );
    assert!(
        import_error.syntax_error().is_some(),
        "expected the underlying syntax error to be recoverable, got: {import_error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_parse_with_imports_recovers_a_transitively_imported_files_syntax_error() {
    let dir = temp_project(&[
        ("main.mcrl2", "%import \"common.mcrl2\"\nmap g: D;\n"),
        ("common.mcrl2", "sort D\n"), // Missing the trailing `;`.
    ]);

    let error = UntypedDataSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), &mut SourceMap::new())
        .expect_err("a transitively broken import must fail");

    let import_error = as_import_error(&error);
    assert!(
        matches!(import_error, ImportError::Unresolved { .. }),
        "expected ImportError::Unresolved, got: {import_error:?}"
    );
    assert!(
        import_error.syntax_error().is_some(),
        "expected the syntax error of the imported file to be recoverable, got: {import_error:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_process_spec_parse_with_imports_merges_the_imported_declarations() {
    let main_text = "%import \"common.mcrl2\"\nact b: D;\ninit a(c) . b(c);\n";
    let dir = temp_project(&[
        ("main.mcrl2", main_text),
        ("common.mcrl2", "sort D;\ncons c: D;\nact a: D;\n"),
    ]);

    let mut sources = SourceMap::new();
    let (spec, _graph) =
        UntypedProcessSpecification::parse_with_imports(&dir.path().join("main.mcrl2"), main_text, &mut sources)
            .expect("should resolve the import");

    assert_eq!(spec.data_specification.sort_declarations.len(), 1);
    assert_eq!(spec.data_specification.constructor_declarations.len(), 1);
    let actions: Vec<(&str, &str)> = spec
        .action_declarations
        .iter()
        .map(|decl| locate(&sources, &decl.identifier.span))
        .collect();
    assert_eq!(actions, [("common.mcrl2", "a"), ("main.mcrl2", "b")]);
    assert_eq!(spec.init.expect("main.mcrl2 has an init").to_string(), "(a(c) . b(c))");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_process_spec_parse_with_imports_keeps_the_importing_files_own_init() {
    // An imported file may have an `init` of its own, which must not override the importing one.
    let main_text = "%import \"common.mcrl2\"\nact b;\ninit b;\n";
    let dir = temp_project(&[("main.mcrl2", main_text), ("common.mcrl2", "act a;\ninit a;\n")]);

    let (spec, _graph) = UntypedProcessSpecification::parse_with_imports(
        &dir.path().join("main.mcrl2"),
        main_text,
        &mut SourceMap::new(),
    )
    .expect("should resolve the import");

    assert_eq!(spec.action_declarations.len(), 2);
    assert_eq!(spec.init.expect("main.mcrl2's own init must survive").to_string(), "b");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_modal_spec_parse_with_imports_pulls_in_action_declarations() {
    let formula_text = "%import \"common.mcrl2\"\nform nu X . [a]X;\n";
    let dir = temp_project(&[("formula.mcf", formula_text), ("common.mcrl2", "act a;\n")]);

    let mut sources = SourceMap::new();
    let (spec, _graph) =
        UntypedStateFrmSpec::parse_with_imports(&dir.path().join("formula.mcf"), formula_text, &mut sources)
            .expect("should resolve the import");

    assert_eq!(spec.action_declarations.len(), 1);
    assert_eq!(
        locate(&sources, &spec.action_declarations[0].identifier.span),
        ("common.mcrl2", "a")
    );
    assert_eq!(spec.formula.to_string(), "(nu X . [a]X)");
}

#[test]
#[cfg_attr(miri, ignore)] // Uses the filesystem.
fn test_modal_spec_parse_with_imports_reports_a_missing_import() {
    let formula_text = "%import \"missing.mcrl2\"\nform true;\n";
    let dir = temp_project(&[("formula.mcf", formula_text)]);

    let error =
        UntypedStateFrmSpec::parse_with_imports(&dir.path().join("formula.mcf"), formula_text, &mut SourceMap::new())
            .expect_err("importing a nonexistent file must fail");

    let import_error = as_import_error(&error);
    let ImportError::Unresolved { path, .. } = import_error else {
        panic!("expected ImportError::Unresolved, got: {import_error:?}");
    };
    assert_eq!(path, "missing.mcrl2");
}
