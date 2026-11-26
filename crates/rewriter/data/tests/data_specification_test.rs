use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermReader;
use merc_aterm::BinaryATermWriter;
use merc_data::BasicSort;
use merc_data::DataEquation;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortAlias;
use merc_data::SortExpression;

/// Writes `spec` to a binary aterm stream and reads it back.
fn roundtrip(spec: &Mcrl2DataSpecification) -> Mcrl2DataSpecification {
    let mut stream: Vec<u8> = Vec::new();
    let mut writer = BinaryATermWriter::new(&mut stream).unwrap();
    spec.write(&mut writer).unwrap();
    ATermWrite::flush(&mut writer).unwrap();
    drop(writer); // Release the mutable borrow on the stream.

    let mut reader = BinaryATermReader::new(&stream[..]).unwrap();
    Mcrl2DataSpecification::read(&mut reader).unwrap()
}

fn assert_same(read: &Mcrl2DataSpecification, spec: &Mcrl2DataSpecification) {
    assert_eq!(read.sorts(), spec.sorts());
    assert_eq!(read.aliases(), spec.aliases());
    assert_eq!(read.constructors(), spec.constructors());
    assert_eq!(read.mappings(), spec.mappings());
    assert_eq!(read.equations(), spec.equations());
}

#[test]
fn test_mcrl2_data_specification_roundtrip() {
    let nat = SortExpression::nat();
    let x = DataVariable::with_sort("x", nat.copy());

    // Populate every section, including an empty one, to exercise the length-prefixed framing.
    // The sections have different lengths, so a reader that mixes them up fails.
    let spec = Mcrl2DataSpecification::new(
        vec![BasicSort::new("D")],
        Vec::new(),
        vec![
            DataFunctionSymbol::with_sort("c", nat.copy()),
            DataFunctionSymbol::with_sort("d", nat.copy()),
        ],
        vec![DataFunctionSymbol::with_sort("f", nat.copy())],
        vec![DataEquation::new(
            std::slice::from_ref(&x),
            None,
            x.clone().into(),
            x.clone().into(),
        )],
    );

    let read = roundtrip(&spec);
    assert_same(&read, &spec);
    assert_eq!(read.constructors().len(), 2);
}

#[test]
fn test_mcrl2_data_specification_alias_roundtrip() {
    let spec = Mcrl2DataSpecification::new(
        Vec::new(),
        vec![SortAlias::new(BasicSort::new("D"), SortExpression::nat())],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    let read = roundtrip(&spec);
    assert_same(&read, &spec);
    assert_eq!(read.aliases().len(), 1);
}

#[test]
fn test_mcrl2_data_specification_empty_roundtrip() {
    let read = roundtrip(&Mcrl2DataSpecification::default());
    assert_same(&read, &Mcrl2DataSpecification::default());
}
