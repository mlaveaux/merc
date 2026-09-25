use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

#[test]
fn probe_lambda_only_function_update_sort() {
    let text = "map b: Bool; \
                 eqn b = (lambda n: Pos. true)[1 -> false] == (lambda n: Pos. false);";
    let spec = UntypedDataSpecification::parse(text).expect("parses");
    let checked = match DataSpecification::from_untyped(spec) {
        Ok(checked) => checked,
        Err(err) => {
            println!("REJECTED at type-check: {err}");
            return;
        }
    };
    let lowered = checked.lower_data_specification();
    let func_update_maps: Vec<String> = lowered
        .mappings()
        .iter()
        .filter(|m| m.name().value().contains("func_update"))
        .map(|m| m.to_string())
        .collect();
    let func_update_eqns: Vec<String> = lowered
        .equations()
        .iter()
        .filter(|e| e.lhs().to_string().contains("func_update"))
        .map(|e| e.to_string())
        .collect();
    println!("func_update mappings: {func_update_maps:#?}");
    println!("func_update equations: {func_update_eqns:#?}");
    for eqn in lowered.equations().iter() {
        if eqn.rhs().to_string().contains("func_update") || eqn.lhs().to_string().contains("func_update") {
            println!("USE SITE: {} = {}", eqn.lhs(), eqn.rhs());
        }
    }
    // Also dump every equation whose lhs mentions `b` (our declared map), to see the actual term.
    for eqn in lowered.equations().iter() {
        let s = eqn.lhs().to_string();
        if s == "b" {
            println!("b's rhs = {}", eqn.rhs());
        }
    }
    assert!(
        !func_update_eqns.is_empty(),
        "expected ground @func_update equations for the Pos -> Bool sort used by the update expression, found none"
    );
}
