use merc_utilities::SourceMap;

#[test]
fn test_single_file_lookup_and_accessors() {
    let mut sources = SourceMap::new();
    let id = sources.add_text("spec.mcrl2", "sort D;");
    assert_eq!(sources.file_count(), 1);
    assert_eq!(sources.text(id), "sort D;");
    assert_eq!(sources.path(id), "spec.mcrl2");
    assert!(!sources.is_virtual(id));
    assert_eq!(sources.lookup(0), id);
    assert_eq!(sources.lookup(3), id);
    // Past the end of the only file still resolves to it.
    assert_eq!(sources.lookup(1000), id);
}

#[test]
fn test_multiple_files_get_disjoint_offsets() {
    let mut sources = SourceMap::new();
    let first = sources.add_text("a.mcrl2", "sort D;");
    let second = sources.add_virtual("<builtin>/list.mcrl2", "sort List;");
    assert_eq!(sources.file_count(), 2);
    assert!(!sources.is_virtual(first));
    assert!(sources.is_virtual(second));
    assert_eq!(sources.text(second), "sort List;");
    assert_eq!(sources.path(second), "<builtin>/list.mcrl2");

    assert_eq!(sources.base_offset(first), 0);
    // One byte further than the naive "first.len()" — the gap between files.
    assert_eq!(sources.base_offset(second), "sort D;".len() + 1);

    assert_eq!(sources.lookup(0), first);
    assert_eq!(sources.lookup("sort D;".len() - 1), first);
    assert_eq!(sources.lookup(sources.base_offset(second)), second);
    assert_eq!(sources.lookup(sources.base_offset(second) + 3), second);
    assert_eq!(sources.lookup(1000), second);
}

#[test]
fn test_offset_one_past_a_file_end_resolves_to_that_file_not_the_next() {
    let mut sources = SourceMap::new();
    let first = sources.add_text("a.mcrl2", "sort D;");
    let second = sources.add_text("b.mcrl2", "sort E;");

    // The end of an end-exclusive span over the whole first file.
    assert_eq!(sources.lookup("sort D;".len()), first);
    assert_eq!(sources.lookup(sources.base_offset(second)), second);
}

#[test]
fn test_empty_file_gets_its_own_unambiguous_offset() {
    let mut sources = SourceMap::new();
    let first = sources.add_text("a.mcrl2", "sort D;");
    let empty = sources.add_text("empty.mcrl2", "");
    let third = sources.add_text("c.mcrl2", "sort F;");

    assert_ne!(sources.base_offset(empty), sources.base_offset(first));
    assert_ne!(sources.base_offset(empty), sources.base_offset(third));
    assert_eq!(sources.lookup(sources.base_offset(empty)), empty);
    // Every offset into the following file must resolve to it, not to the empty file.
    assert_eq!(sources.lookup(sources.base_offset(third)), third);
    assert_eq!(sources.lookup(sources.base_offset(third) + 3), third);
}

#[test]
fn test_load_file_reads_contents_and_path() {
    let path = std::env::temp_dir().join(format!("merc_source_map_test_{}.mcrl2", std::process::id()));
    std::fs::write(&path, "sort D;\n").unwrap();

    let mut sources = SourceMap::new();
    let result = sources.load_file(&path);
    std::fs::remove_file(&path).unwrap();

    let id = result.unwrap();
    assert_eq!(sources.text(id), "sort D;\n");
    assert_eq!(sources.path(id), path.display().to_string());
    assert!(!sources.is_virtual(id));

    assert!(sources.load_file(&path).is_err());
    assert_eq!(sources.file_count(), 1);
}

#[test]
#[should_panic(expected = "empty SourceMap")]
fn test_lookup_on_empty_source_map_panics() {
    SourceMap::new().lookup(0);
}
