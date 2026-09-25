use merc_syntax::UntypedStateFrmSpec;
use merc_typecheck::FormulaType;
use merc_typecheck::ModalSpecification;
use std::fs;

#[test]
fn probe_parse_only() {
    let text = fs::read_to_string("/tmp/deep.mcrl2").unwrap();
    let spec = UntypedStateFrmSpec::parse(&text);
    println!("parse ok: {}", spec.is_ok());
}
