use std::fmt;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::LazyLock;

use merc_syntax::ConstructorDecl;
use merc_syntax::OffsetSpans;
use merc_syntax::Sort;
use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;
use merc_utilities::MercError;

use crate::InferenceError;
use crate::NumberEncoding;
use crate::Signature;
use crate::TypeCheckContext;
use crate::assign_declaration_ids;
use crate::build_polymorphic_schemes;
use crate::lower_data_expressions;
use crate::merge_signatures;
use crate::resolve_data_specification_variables;
use crate::resolve_type_variables;
use crate::typecheck_template_equations;

/// Identifies one Appendix-B template, either bundled or generated to map to
/// the scheme typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TemplateId {
    List,
    Set,
    FSet,
    Bag,
    FBag,
    /// The generated, generic function-update template of the given arity.
    FunctionUpdateN(usize),
    /// The reflexive/derived comparison-operator template.
    Comparison,
}

/// Returns a standard data specification containing the standard sorts and their
/// associated constructors, mappings, and equations, in the given `encoding`,
/// registered into `sources`.
pub(crate) fn basic_sort_data_specification(
    sources: &mut SourceMap,
    encoding: NumberEncoding,
) -> UntypedDataSpecification {
    match encoding {
        NumberEncoding::Binary => basic_sorts_binary(sources),
        NumberEncoding::MachineWord => basic_sorts_machine_word(sources),
    }
}

/// The generic function-update template of arity `arity >= 1`.
pub(crate) fn function_update_template(arity: usize) -> UntypedDataSpecification {
    build_function_update_template(&function_update_template_text(arity), arity)
}

/// As [function_update_template], but registered into `sources` as a virtual
/// document first, so a genuine type error found while rigidly checking the
/// generic template (`typecheck_function_update_template`) renders against
/// real source text instead of a span that resolves to nothing.
pub(crate) fn register_function_update_template(sources: &mut SourceMap, arity: usize) -> UntypedDataSpecification {
    let text = function_update_template_text(arity);
    let id = sources.add_virtual(format!("<builtin>/function_update_{arity}.mcrl2"), text.clone());
    let base = sources.base_offset(id);

    let mut spec = build_function_update_template(&text, arity);
    spec.offset_spans(base);
    spec
}

/// The source text of the generic, arity-`arity` function-update template,
/// shared by [function_update_template] and [register_function_update_template].
fn function_update_template_text(arity: usize) -> String {
    debug_assert!(arity > 0, "a function sort always has at least one domain sort");

    let domain_names: Vec<String> = (0..arity).map(|i| format!("S{i}")).collect();
    let range_name = "T";
    format!(
        "type_var {}, {range_name};\n{}",
        domain_names.join(", "),
        function_update_text(&domain_names, range_name)
    )
}

/// Parses and fully lowers `text`, the arity-`arity` function-update
/// template's own source, shared by [function_update_template] and
/// [register_function_update_template].
fn build_function_update_template(text: &str, arity: usize) -> UntypedDataSpecification {
    let mut spec = UntypedDataSpecification::parse(text).unwrap_or_else(|err| {
        panic!("the generated arity-{arity} function-update template does not parse: {err}\n{text}")
    });

    resolve_type_variables(&mut spec).expect("the generated template's type_var block resolves");
    resolve_data_specification_variables(&mut spec);
    assign_declaration_ids(&mut spec);
    lower_data_expressions(&mut spec);
    spec
}

/// The body (`map`/`var`/`eqn` blocks) of the function-update operators for
/// arity `domain_names.len() >= 1`, over the given domain/range sort.
///
/// Uses the `@`-prefix reserved-name convention, so it can never collide with a
/// user-declared map, constructor or variable of the same name.
pub(crate) fn function_update_text(domain_names: &[String], range: &str) -> String {
    let arity = domain_names.len();
    debug_assert!(arity > 0, "a function sort always has at least one domain sort");

    // Parenthesized: this text is embedded as one operand alongside others in
    // `@func_update`'s own domain/range below, and a bare `A # B -> C` would
    // parse with the wrong grouping there.
    let function_sort = format!("({} -> {range})", domain_names.join(" # "));
    let domain_sorts = domain_names.join(" # ");

    let xs: Vec<String> = (0..arity).map(|i| format!("@x{i}")).collect();
    let ys: Vec<String> = (0..arity).map(|i| format!("@y{i}")).collect();
    let x_args = xs.join(", ");
    let y_args = ys.join(", ");

    let equal = |a: &[String], b: &[String]| -> String {
        a.iter()
            .zip(b)
            .map(|(l, r)| format!("{l} == {r}"))
            .collect::<Vec<_>>()
            .join(" && ")
    };

    // Lexicographic order over the index tuple, exactly as `structured_sort_equations`'s
    // `lexicographic` closure builds it for constructor arguments.
    let less = |a: &[String], b: &[String]| -> String {
        let last = a.len() - 1;
        let mut expr = format!("{} < {}", a[last], b[last]);
        for i in (0..last).rev() {
            expr = format!("{} < {} || ({} == {} && ({expr}))", a[i], b[i], a[i], b[i]);
        }
        expr
    };

    let x_eq_y = equal(&xs, &ys);
    let x_neq_y = format!("!({x_eq_y})");
    let y_lt_x = less(&ys, &xs);
    let x_lt_y = less(&xs, &ys);

    let mut spec = String::new();
    writeln!(
        spec,
        "map @func_update: {function_sort} # {domain_sorts} # {range} -> {function_sort};"
    )
    .unwrap();
    writeln!(
        spec,
        "    @func_update_stable: {function_sort} # {domain_sorts} # {range} -> {function_sort};"
    )
    .unwrap();
    writeln!(spec, "    @is_not_an_update: {function_sort} -> Bool;").unwrap();
    writeln!(
        spec,
        "    @if_always_else: Bool # {function_sort} # {function_sort} -> {function_sort};"
    )
    .unwrap();

    writeln!(spec, "var").unwrap();
    for (i, argument_sort) in domain_names.iter().enumerate() {
        writeln!(spec, "    @x{i}, @y{i}: {argument_sort};").unwrap();
    }
    writeln!(spec, "    @v, @w: {range};").unwrap();
    writeln!(spec, "    @f: {domain_sorts} -> {range};").unwrap();

    writeln!(
        spec,
        "eqn @is_not_an_update(@f) -> @func_update(@f,{x_args},@v) = \
         @if_always_else(@f({x_args}) == @v,@f,@func_update_stable(@f,{x_args},@v));"
    )
    .unwrap();
    writeln!(
        spec,
        "    @func_update(@func_update_stable(@f,{x_args},@w),{x_args},@v) = \
         @if_always_else(@f({x_args}) == @v,@f,@func_update_stable(@f,{x_args},@v));"
    )
    .unwrap();
    writeln!(
        spec,
        "    {y_lt_x} -> @func_update(@func_update_stable(@f,{y_args},@w), {x_args},@v) = \
         @func_update_stable(@func_update(@f,{x_args},@v),{y_args},@w);"
    )
    .unwrap();
    writeln!(
        spec,
        "    {x_lt_y} -> @func_update(@func_update_stable(@f,{y_args},@w), {x_args},@v) = \
         @if_always_else(@f({x_args}) == @v, \
         @func_update_stable(@f,{y_args},@w), \
         @func_update_stable(@func_update_stable(@f,{y_args},@w), {x_args},@v));"
    )
    .unwrap();
    writeln!(
        spec,
        "    {x_neq_y} -> @func_update_stable(@f,{x_args},@v)({y_args}) = @f({y_args});"
    )
    .unwrap();
    writeln!(spec, "    @func_update_stable(@f,{x_args},@v)({x_args}) = @v;").unwrap();
    writeln!(
        spec,
        "    @func_update(@f,{x_args},@v)({y_args}) = if({x_eq_y},@v,@f({y_args}));"
    )
    .unwrap();

    spec
}

/// Generates the defining equations of a structured sort, following Appendix
/// `B.10`.
///
/// # Details
///
/// Given the constructors `c_1, ..., c_n` of a structured sort, where every
/// constructor `c_i` has arguments of sorts `A_{i,1}, ..., A_{i,k_i}`, this
/// generates the equations defining the recognisers, the projections, and the
/// comparison operators `==`, `<`, `<=` and `less_total` over the constructors.
///
/// Only equations are generated; the abstract sort and the constructor,
/// recogniser and projection declarations are introduced by
/// `desugar_structured_sorts`, which also yields the `constructors` passed
/// here. The result joins the system-defined specification, like the other
/// Appendix-B content.
pub(crate) fn structured_sort_equations(
    sources: &mut SourceMap,
    constructors: &[ConstructorDecl],
) -> Result<UntypedDataSpecification, MercError> {
    // Builds the term `c_i(<prefix>i_0, ..., <prefix>i_{k_i - 1})`, using the
    // bare constructor name when `c_i` takes no arguments.
    let application = |i: usize, prefix: &str| -> String {
        let constructor = &constructors[i];
        if constructor.args.is_empty() {
            constructor.name.node.clone()
        } else {
            let arguments = (0..constructor.args.len())
                .map(|j| format!("{prefix}{i}_{j}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({arguments})", constructor.name.node)
        }
    };

    // Builds the right-hand side of the `<` or `<=` equation between two equal
    // constructors, i.e. the lexicographic comparison of their arguments where
    // the final argument is compared using `last_op` (`<` or `<=`):
    //   @x0 < @y0 || (@x0 == @y0 && (... || (@x_{k-2} == @y_{k-2} && (@x_{k-1} OP @y_{k-1}))...))
    let lexicographic = |i: usize, arity: usize, last_op: &str| -> String {
        let mut expr = format!("@x{i}_{last} {last_op} @y{i}_{last}", last = arity - 1);
        for j in (0..arity - 1).rev() {
            expr = format!("@x{i}_{j} < @y{i}_{j} || (@x{i}_{j} == @y{i}_{j} && ({expr}))");
        }
        expr
    };

    let mut spec = String::new();

    // var: one x/y pair per constructor argument.
    let mut vars = String::new();
    for (i, constructor) in constructors.iter().enumerate() {
        for (j, (_, sort)) in constructor.args.iter().enumerate() {
            writeln!(vars, "    @x{i}_{j}, @y{i}_{j}: {sort};").unwrap();
        }
    }

    // eqn: recogniser, projection and comparison equations.
    let mut eqns = String::new();

    // Recognisers: isC_i(c_i(..)) = true; isC_i(c_j(..)) = false for j != i.
    for (i, constructor) in constructors.iter().enumerate() {
        if let Some(recogniser) = &constructor.recogniser {
            let recogniser = &recogniser.node;
            writeln!(eqns, "    {recogniser}({}) = true;", application(i, "@x")).unwrap();
            for j in 0..constructors.len() {
                if j != i {
                    writeln!(eqns, "    {recogniser}({}) = false;", application(j, "@x")).unwrap();
                }
            }
        }
    }

    // Projections: pr_{i,j}(c_i(..)) = x_{i,j}.
    for (i, constructor) in constructors.iter().enumerate() {
        for (j, (projection, _)) in constructor.args.iter().enumerate() {
            if let Some(projection) = projection {
                let projection = &projection.node;
                writeln!(eqns, "    {projection}({}) = @x{i}_{j};", application(i, "@x")).unwrap();
            }
        }
    }

    // Equality: componentwise on equal constructors, false on distinct ones.
    for (i, constructor) in constructors.iter().enumerate() {
        let equal = if constructor.args.is_empty() {
            "true".to_string()
        } else {
            (0..constructor.args.len())
                .map(|j| format!("@x{i}_{j} == @y{i}_{j}"))
                .collect::<Vec<_>>()
                .join(" && ")
        };
        writeln!(
            eqns,
            "    {} == {} = {equal};",
            application(i, "@x"),
            application(i, "@y")
        )
        .unwrap();
        for j in 0..constructors.len() {
            if j != i {
                writeln!(
                    eqns,
                    "    {} == {} = false;",
                    application(i, "@x"),
                    application(j, "@y")
                )
                .unwrap();
            }
        }
    }

    // Less-than: lexicographic on equal constructors, by constructor index otherwise.
    // `<` is already a total order here, so `less_total` just reuses it.
    for (i, constructor) in constructors.iter().enumerate() {
        let less = if constructor.args.is_empty() {
            "false".to_string()
        } else {
            lexicographic(i, constructor.args.len(), "<")
        };
        writeln!(
            eqns,
            "    {} < {} = {less};",
            application(i, "@x"),
            application(i, "@y")
        )
        .unwrap();
        writeln!(
            eqns,
            "    less_total({}, {}) = {} < {};",
            application(i, "@x"),
            application(i, "@y"),
            application(i, "@x"),
            application(i, "@y")
        )
        .unwrap();
        for j in 0..constructors.len() {
            if i < j {
                writeln!(eqns, "    {} < {} = true;", application(i, "@x"), application(j, "@y")).unwrap();
                writeln!(
                    eqns,
                    "    less_total({}, {}) = true;",
                    application(i, "@x"),
                    application(j, "@y")
                )
                .unwrap();
            } else if i > j {
                writeln!(eqns, "    {} < {} = false;", application(i, "@x"), application(j, "@y")).unwrap();
                writeln!(
                    eqns,
                    "    less_total({}, {}) = false;",
                    application(i, "@x"),
                    application(j, "@y")
                )
                .unwrap();
            }
        }
    }

    // Less-than-or-equal: as `<`, but the last argument is compared with `<=`.
    for (i, constructor) in constructors.iter().enumerate() {
        let less_equal = if constructor.args.is_empty() {
            "true".to_string()
        } else {
            lexicographic(i, constructor.args.len(), "<=")
        };
        writeln!(
            eqns,
            "    {} <= {} = {less_equal};",
            application(i, "@x"),
            application(i, "@y")
        )
        .unwrap();
        for j in 0..constructors.len() {
            if i < j {
                writeln!(eqns, "    {} <= {} = true;", application(i, "@x"), application(j, "@y")).unwrap();
            } else if i > j {
                writeln!(
                    eqns,
                    "    {} <= {} = false;",
                    application(i, "@x"),
                    application(j, "@y")
                )
                .unwrap();
            }
        }
    }

    if vars.is_empty() {
        write!(spec, "eqn\n{eqns}").unwrap();
    } else {
        write!(spec, "var\n{vars}eqn\n{eqns}").unwrap();
    }

    // Named after the first constructor so a parse-error render reads as "the
    // struct with c1, ...", not an opaque, unnumbered "<generated>".
    let name = constructors
        .first()
        .map(|c| format!("<generated>/struct/{}.mcrl2", c.name.node))
        .unwrap_or_else(|| "<generated>/struct/empty.mcrl2".to_string());
    parse_generated(sources, &name, &spec)
}

impl fmt::Display for TemplateId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TemplateId::List => write!(f, "list"),
            TemplateId::Set => write!(f, "set"),
            TemplateId::FSet => write!(f, "fset"),
            TemplateId::Bag => write!(f, "bag"),
            TemplateId::FBag => write!(f, "fbag"),
            TemplateId::FunctionUpdateN(arity) => write!(f, "function_update_{arity}"),
            TemplateId::Comparison => write!(f, "comparison"),
        }
    }
}

/// Parses a bundled `spec/*.mcrl2` file, or an equally self-contained
/// hand-written template string (`BUILTIN_SCHEME_TEMPLATE`), with no
/// `SourceMap` involved.
fn parse_template_bare(text: &str) -> UntypedDataSpecification {
    let mut spec = UntypedDataSpecification::parse(text).expect("the bundled templates parse");
    resolve_type_variables(&mut spec).expect("the bundled template's type_var block resolves");
    spec
}

/// As [parse_template_bare], but also assings unique ids, and lowers the
/// specification.
pub fn parse_rigid_template(text: &str) -> UntypedDataSpecification {
    let mut spec = parse_template_bare(text);

    resolve_data_specification_variables(&mut spec);
    assign_declaration_ids(&mut spec);
    lower_data_expressions(&mut spec);
    spec
}

/// Registers `text` under `name` as a virtual source in `sources`, and shifts a clone of the
/// already-parsed `template`'s spans into that registration's base offset — reused every time a
/// bundled template's *content* is needed against a fresh `SourceMap` (once per type-checking
/// session), without re-parsing it.
pub fn register_bare_template(
    sources: &mut SourceMap,
    name: &str,
    text: &'static str,
    template: &UntypedDataSpecification,
) -> UntypedDataSpecification {
    let id = sources.add_virtual(name, text);
    let base = sources.base_offset(id);
    let mut spec = template.clone();
    spec.offset_spans(base);
    spec
}

/// Registers `text` under `name` as a virtual source and parses it, then shifts
/// every span it produced into that registration's base offset.
fn parse_generated(sources: &mut SourceMap, name: &str, text: &str) -> Result<UntypedDataSpecification, MercError> {
    let id = sources.add_virtual(name, text.to_string());
    let base = sources.base_offset(id);

    let mut spec = UntypedDataSpecification::parse(text)?;
    spec.offset_spans(base);

    resolve_type_variables(&mut spec)?;
    Ok(spec)
}

/// The raw, uninstantiated basic-sort templates, parsed once and span shifted
/// when necessary.
struct BasicSortTemplates {
    bool: UntypedDataSpecification,
    pos: UntypedDataSpecification,
    int: UntypedDataSpecification,
    nat: UntypedDataSpecification,
    real: UntypedDataSpecification,
    machine_word: UntypedDataSpecification,
    pos64: UntypedDataSpecification,
    int64: UntypedDataSpecification,
    nat64: UntypedDataSpecification,
    real64: UntypedDataSpecification,
}

static BASIC_SORT_TEMPLATES: LazyLock<BasicSortTemplates> = LazyLock::new(|| BasicSortTemplates {
    bool: parse_template_bare(include_str!("../../../syntax/spec/bool.mcrl2")),
    pos: parse_template_bare(include_str!("../../../syntax/spec/pos.mcrl2")),
    int: parse_template_bare(include_str!("../../../syntax/spec/int.mcrl2")),
    nat: parse_template_bare(include_str!("../../../syntax/spec/nat.mcrl2")),
    real: parse_template_bare(include_str!("../../../syntax/spec/real.mcrl2")),
    machine_word: parse_template_bare(include_str!("../../../syntax/spec/machine_word.mcrl2")),
    pos64: parse_template_bare(include_str!("../../../syntax/spec/pos64.mcrl2")),
    int64: parse_template_bare(include_str!("../../../syntax/spec/int64.mcrl2")),
    nat64: parse_template_bare(include_str!("../../../syntax/spec/nat64.mcrl2")),
    real64: parse_template_bare(include_str!("../../../syntax/spec/real64.mcrl2")),
});

/// Registers and merges each `(virtual path, source text, parsed template)`
/// triple in `entries` into one specification, in order — the shared body of
/// [basic_sorts_binary]/[basic_sorts_machine_word].
fn merge_bare_templates(
    sources: &mut SourceMap,
    entries: &[(&'static str, &'static str, &UntypedDataSpecification)],
) -> UntypedDataSpecification {
    let mut result = UntypedDataSpecification::default();
    for &(name, text, template) in entries {
        result.merge(&register_bare_template(sources, name, text, template));
    }
    result
}

/// The merged specifications of the five basic sorts (Appendix B) in the
/// recursive binary encoding.
fn basic_sorts_binary(sources: &mut SourceMap) -> UntypedDataSpecification {
    merge_bare_templates(
        sources,
        &[
            (
                "<builtin>/bool.mcrl2",
                include_str!("../../../syntax/spec/bool.mcrl2"),
                &BASIC_SORT_TEMPLATES.bool,
            ),
            (
                "<builtin>/pos.mcrl2",
                include_str!("../../../syntax/spec/pos.mcrl2"),
                &BASIC_SORT_TEMPLATES.pos,
            ),
            (
                "<builtin>/int.mcrl2",
                include_str!("../../../syntax/spec/int.mcrl2"),
                &BASIC_SORT_TEMPLATES.int,
            ),
            (
                "<builtin>/nat.mcrl2",
                include_str!("../../../syntax/spec/nat.mcrl2"),
                &BASIC_SORT_TEMPLATES.nat,
            ),
            (
                "<builtin>/real.mcrl2",
                include_str!("../../../syntax/spec/real.mcrl2"),
                &BASIC_SORT_TEMPLATES.real,
            ),
        ],
    )
}

/// The same five basic sorts in the 64-bit machine-word encoding.
fn basic_sorts_machine_word(sources: &mut SourceMap) -> UntypedDataSpecification {
    merge_bare_templates(
        sources,
        &[
            (
                "<builtin>/bool.mcrl2",
                include_str!("../../../syntax/spec/bool.mcrl2"),
                &BASIC_SORT_TEMPLATES.bool,
            ),
            (
                "<builtin>/machine_word.mcrl2",
                include_str!("../../../syntax/spec/machine_word.mcrl2"),
                &BASIC_SORT_TEMPLATES.machine_word,
            ),
            (
                "<builtin>/pos64.mcrl2",
                include_str!("../../../syntax/spec/pos64.mcrl2"),
                &BASIC_SORT_TEMPLATES.pos64,
            ),
            (
                "<builtin>/int64.mcrl2",
                include_str!("../../../syntax/spec/int64.mcrl2"),
                &BASIC_SORT_TEMPLATES.int64,
            ),
            (
                "<builtin>/nat64.mcrl2",
                include_str!("../../../syntax/spec/nat64.mcrl2"),
                &BASIC_SORT_TEMPLATES.nat64,
            ),
            (
                "<builtin>/real64.mcrl2",
                include_str!("../../../syntax/spec/real64.mcrl2"),
                &BASIC_SORT_TEMPLATES.real64,
            ),
        ],
    )
}

/// The raw, uninstantiated container templates, parsed once.
pub(crate) struct ContainerTemplates {
    pub(crate) list: UntypedDataSpecification,
    pub(crate) set: UntypedDataSpecification,
    pub(crate) fset: UntypedDataSpecification,
    pub(crate) bag: UntypedDataSpecification,
    pub(crate) fbag: UntypedDataSpecification,
}

/// The [TemplateId] of each [ContainerTemplates] field.
pub(crate) const CONTAINER_TEMPLATE_IDS: [TemplateId; 5] = [
    TemplateId::List,
    TemplateId::Set,
    TemplateId::FSet,
    TemplateId::Bag,
    TemplateId::FBag,
];

impl ContainerTemplates {
    /// All templates, for building the polymorphic signature.
    pub(crate) fn all(&self) -> [&UntypedDataSpecification; 5] {
        [&self.list, &self.set, &self.fset, &self.bag, &self.fbag]
    }

    /// As [Self::all], paired with each template's own [TemplateId] from
    /// [CONTAINER_TEMPLATE_IDS].
    pub(crate) fn all_named(&self) -> [(TemplateId, &UntypedDataSpecification); 5] {
        let templates = self.all();
        std::array::from_fn(|i| (CONTAINER_TEMPLATE_IDS[i], templates[i]))
    }
}

/// The container templates in the recursive binary encoding, only used for the
/// signatures.
pub(crate) static CONTAINER_TEMPLATES: LazyLock<ContainerTemplates> = LazyLock::new(|| ContainerTemplates {
    list: parse_rigid_template(include_str!("../../../syntax/spec/list.mcrl2")),
    set: parse_rigid_template(include_str!("../../../syntax/spec/set.mcrl2")),
    fset: parse_rigid_template(include_str!("../../../syntax/spec/fset.mcrl2")),
    bag: parse_rigid_template(include_str!("../../../syntax/spec/bag.mcrl2")),
    fbag: parse_rigid_template(include_str!("../../../syntax/spec/fbag.mcrl2")),
});

/// As [CONTAINER_TEMPLATES], for the container templates whose equations are expressed in terms of
/// the machine-word numeric sorts.
pub(crate) static CONTAINER_TEMPLATES_MACHINE_WORD: LazyLock<ContainerTemplates> =
    LazyLock::new(|| ContainerTemplates {
        list: parse_rigid_template(include_str!("../../../syntax/spec/list64.mcrl2")),
        set: parse_rigid_template(include_str!("../../../syntax/spec/set64.mcrl2")),
        fset: parse_rigid_template(include_str!("../../../syntax/spec/fset64.mcrl2")),
        bag: parse_rigid_template(include_str!("../../../syntax/spec/bag64.mcrl2")),
        fbag: parse_rigid_template(include_str!("../../../syntax/spec/fbag64.mcrl2")),
    });

/// [CONTAINER_TEMPLATES], registered into `sources` as virtual documents.
fn container_templates_binary(sources: &mut SourceMap) -> ContainerTemplates {
    ContainerTemplates {
        list: register_bare_template(
            sources,
            "<builtin>/list.mcrl2",
            include_str!("../../../syntax/spec/list.mcrl2"),
            &CONTAINER_TEMPLATES.list,
        ),
        set: register_bare_template(
            sources,
            "<builtin>/set.mcrl2",
            include_str!("../../../syntax/spec/set.mcrl2"),
            &CONTAINER_TEMPLATES.set,
        ),
        fset: register_bare_template(
            sources,
            "<builtin>/fset.mcrl2",
            include_str!("../../../syntax/spec/fset.mcrl2"),
            &CONTAINER_TEMPLATES.fset,
        ),
        bag: register_bare_template(
            sources,
            "<builtin>/bag.mcrl2",
            include_str!("../../../syntax/spec/bag.mcrl2"),
            &CONTAINER_TEMPLATES.bag,
        ),
        fbag: register_bare_template(
            sources,
            "<builtin>/fbag.mcrl2",
            include_str!("../../../syntax/spec/fbag.mcrl2"),
            &CONTAINER_TEMPLATES.fbag,
        ),
    }
}

/// [CONTAINER_TEMPLATES_MACHINE_WORD], registered into `sources` as virtual
/// documents.
fn container_templates_machine_word(sources: &mut SourceMap) -> ContainerTemplates {
    ContainerTemplates {
        list: register_bare_template(
            sources,
            "<builtin>/list64.mcrl2",
            include_str!("../../../syntax/spec/list64.mcrl2"),
            &CONTAINER_TEMPLATES_MACHINE_WORD.list,
        ),
        set: register_bare_template(
            sources,
            "<builtin>/set64.mcrl2",
            include_str!("../../../syntax/spec/set64.mcrl2"),
            &CONTAINER_TEMPLATES_MACHINE_WORD.set,
        ),
        fset: register_bare_template(
            sources,
            "<builtin>/fset64.mcrl2",
            include_str!("../../../syntax/spec/fset64.mcrl2"),
            &CONTAINER_TEMPLATES_MACHINE_WORD.fset,
        ),
        bag: register_bare_template(
            sources,
            "<builtin>/bag64.mcrl2",
            include_str!("../../../syntax/spec/bag64.mcrl2"),
            &CONTAINER_TEMPLATES_MACHINE_WORD.bag,
        ),
        fbag: register_bare_template(
            sources,
            "<builtin>/fbag64.mcrl2",
            include_str!("../../../syntax/spec/fbag64.mcrl2"),
            &CONTAINER_TEMPLATES_MACHINE_WORD.fbag,
        ),
    }
}

/// The container templates to instantiate for `encoding`, registered into
/// `sources` so their spans render correctly — shared by the rigid check
/// (`typecheck_templates`) and by concrete instantiation
/// (`crate::standard_sort`).
pub(crate) fn container_templates(sources: &mut SourceMap, encoding: NumberEncoding) -> ContainerTemplates {
    match encoding {
        NumberEncoding::Binary => container_templates_binary(sources),
        NumberEncoding::MachineWord => container_templates_machine_word(sources),
    }
}

/// Type checks every template's own equations once populating
/// `ctx.template_typings`. `sources` is where the container/comparison
/// templates' own bundled text is registered, so that a genuine type error
/// found here.
pub(crate) fn typecheck_templates(
    ctx: &mut TypeCheckContext,
    sources: &mut SourceMap,
    encoding: NumberEncoding,
) -> Result<(), InferenceError> {
    let templates = container_templates(sources, encoding);

    for (id, template) in templates.all_named() {
        if !ctx.template_typings.contains_key(&id) {
            let typings = typecheck_template_equations(ctx, id, template)?;
            ctx.template_typings.insert(id, typings);
        }
    }

    let comparison_template = register_bare_template(
        sources,
        "<builtin>/schemes/comparison.mcrl2",
        BUILTIN_SCHEME_TEMPLATE_TEXT,
        &BUILTIN_SCHEME_TEMPLATE,
    );
    let typings = typecheck_template_equations(ctx, TemplateId::Comparison, &comparison_template)?;
    ctx.template_typings.insert(TemplateId::Comparison, typings);
    Ok(())
}

/// Type checks the generic, n-`arity` (`>= 1`) function-update template's own
/// equations once populating `ctx.template_typings` under
/// [`TemplateId::FunctionUpdateN`], and merges the arity's own scheme into
/// `ctx.signature` permanently.
pub(crate) fn typecheck_function_update_template(
    ctx: &mut TypeCheckContext,
    sources: &mut SourceMap,
    arity: usize,
) -> Result<(), InferenceError> {
    let id = TemplateId::FunctionUpdateN(arity);
    if ctx.template_typings.contains_key(&id) {
        return Ok(());
    }
    let template = register_function_update_template(sources, arity);

    // This arity's own `@func_update`/`@func_update_stable`/`@is_not_an_update`/
    // `@if_always_else` are declared only inside `template` itself — the
    // pooled `ctx.signature` carries no version of these names at all, since
    // function-update is never bundled, only generated.
    let current_signature = Arc::clone(
        ctx.signature
            .as_ref()
            .expect("build_signature ran before check_function_update_template"),
    );
    let own_schemes = build_polymorphic_schemes(ctx, std::iter::once(&template));
    let own_signature = Signature {
        schemes: own_schemes,
        ..Signature::default()
    };
    ctx.signature = Some(Arc::new(merge_signatures(&own_signature, &current_signature)));

    let typings = typecheck_template_equations(ctx, id, &template)?;
    ctx.template_typings.insert(id, typings);
    Ok(())
}

/// [BUILTIN_SCHEME_TEMPLATE]'s source text.
pub(crate) const BUILTIN_SCHEME_TEMPLATE_TEXT: &str = include_str!("../../../syntax/spec/comparison.mcrl2");

/// The polymorphic built-in operators that exist for *every* sort.
pub(crate) static BUILTIN_SCHEME_TEMPLATE: LazyLock<UntypedDataSpecification> =
    LazyLock::new(|| parse_rigid_template(BUILTIN_SCHEME_TEMPLATE_TEXT));

/// The names of the polymorphic built-in schemes, derived from
/// [`BUILTIN_SCHEME_TEMPLATE`] so the list has a single definition. These names
/// are usable without a declaration, so the well-formedness and reserved-name
/// checks admit them.
pub(crate) fn builtin_scheme_names() -> impl Iterator<Item = &'static str> {
    BUILTIN_SCHEME_TEMPLATE
        .map_declarations
        .iter()
        .map(|decl| decl.identifier.as_str())
}

/// The raw, uninstantiated template of `sort`'s own Appendix-B declarations.
pub(crate) fn basic_sort_own_template(sort: Sort, encoding: NumberEncoding) -> &'static UntypedDataSpecification {
    match (sort, encoding) {
        (Sort::Bool, _) => &BASIC_SORT_TEMPLATES.bool,
        (Sort::Pos, NumberEncoding::Binary) => &BASIC_SORT_TEMPLATES.pos,
        (Sort::Pos, NumberEncoding::MachineWord) => &BASIC_SORT_TEMPLATES.pos64,
        (Sort::Nat, NumberEncoding::Binary) => &BASIC_SORT_TEMPLATES.nat,
        (Sort::Nat, NumberEncoding::MachineWord) => &BASIC_SORT_TEMPLATES.nat64,
        (Sort::Int, NumberEncoding::Binary) => &BASIC_SORT_TEMPLATES.int,
        (Sort::Int, NumberEncoding::MachineWord) => &BASIC_SORT_TEMPLATES.int64,
        (Sort::Real, NumberEncoding::Binary) => &BASIC_SORT_TEMPLATES.real,
        (Sort::Real, NumberEncoding::MachineWord) => &BASIC_SORT_TEMPLATES.real64,
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::ConstructorDecl;
    use merc_syntax::SortExpressionKind;
    use merc_syntax::SourceMap;
    use merc_syntax::UntypedDataSpecification;

    use super::CONTAINER_TEMPLATES;
    use super::structured_sort_equations;

    #[test]
    fn test_container_templates_declare_type_var() {
        // `list.mcrl2`/`bag.mcrl2`/... each declare one `type_var S;`, resolved
        // once by `parse_template_bare` — no bare `Reference("S")` should remain.
        for template in [
            &CONTAINER_TEMPLATES.list,
            &CONTAINER_TEMPLATES.set,
            &CONTAINER_TEMPLATES.fset,
            &CONTAINER_TEMPLATES.bag,
            &CONTAINER_TEMPLATES.fbag,
        ] {
            assert_eq!(template.type_var_declarations.len(), 1, "{template}");
            assert_eq!(template.type_var_declarations[0].identifier, "S");
            assert!(
                template.type_var_declarations[0].id.is_some(),
                "resolve_type_var_ids should have assigned an id"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_function_update_template_declares_two_type_vars() {
        // The arity-1 generic template plays the role the bundled
        // `function_update.mcrl2` used to: `type_var S0, T;`.
        let template = super::function_update_template(1);
        let names: Vec<&str> = template
            .type_var_declarations
            .iter()
            .map(|decl| decl.identifier.as_str())
            .collect();
        assert_eq!(names, ["S0", "T"]);
    }

    /// Extracts the constructors of the structured sort in `sort <ident> = <struct>;`.
    fn struct_constructors(spec: &str) -> Vec<ConstructorDecl> {
        let spec = UntypedDataSpecification::parse(spec).unwrap();
        let expr = spec
            .sort_declarations
            .into_iter()
            .find_map(|decl| decl.expr)
            .expect("expected a sort alias with a structured sort");
        let SortExpressionKind::Struct { inner } = expr.into_node() else {
            panic!("expected a structured sort");
        };
        inner
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn structured_sort_equations_generates_a_parseable_specification() {
        let constructors = struct_constructors("sort D = struct c1(pr1: Nat, pr2: Bool)?is_c1 | c2?is_c2 | c3(Nat);");

        // The generated specification should be well-formed and parseable, and
        // contain only equations; the declarations come from desugaring.
        let generated = structured_sort_equations(&mut SourceMap::new(), &constructors).unwrap();
        assert!(generated.sort_declarations.is_empty());
        assert!(generated.constructor_declarations.is_empty());
        assert!(generated.map_declarations.is_empty());

        let equations = generated
            .equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .map(|eqn| format!("{} = {}", eqn.lhs, eqn.rhs))
            .collect::<Vec<_>>();

        // Recogniser and projection equations for the declared names.
        assert!(
            equations
                .iter()
                .any(|eqn| eqn.contains("is_c1") && eqn.contains("true"))
        );
        assert!(
            equations
                .iter()
                .any(|eqn| eqn.contains("is_c1") && eqn.contains("false"))
        );
        assert!(equations.iter().any(|eqn| eqn.contains("pr1")));

        // c3 has no recogniser, so no equation defines one for it.
        assert!(!equations.iter().any(|eqn| eqn.contains("is_c3")));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn structured_sort_equations_supports_only_constant_constructors() {
        // A structured sort where no constructor has arguments generates no
        // variables, so the `eqn` block must be emitted without a `var` block.
        let constructors = struct_constructors("sort E = struct red | green | blue;");
        let generated = structured_sort_equations(&mut SourceMap::new(), &constructors).unwrap();

        assert!(!generated.equation_declarations.is_empty());
    }

    /// A system-defined declaration's span must render against its true
    /// origin — the bundled template file it came from — not the caller's
    /// own specification text, once it is registered into the same
    /// `SourceMap` the caller renders against.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_basic_sort_declaration_renders_against_its_builtin_source() {
        let mut sources = SourceMap::new();
        let basics = super::basic_sort_data_specification(&mut sources, crate::NumberEncoding::Binary);
        let bool_decl = basics
            .sort_declarations
            .iter()
            .find(|decl| decl.identifier == "Bool")
            .expect("Bool is always declared");

        let rendered = bool_decl.span.render(&sources);
        assert!(
            rendered.contains("bool.mcrl2"),
            "expected the Bool sort declaration to render against bool.mcrl2, got: {rendered}"
        );
        let id = sources.lookup(bool_decl.span.start);
        assert!(sources.is_virtual(id), "a builtin template's source must be virtual");
    }

    /// As [test_basic_sort_declaration_renders_against_its_builtin_source], but
    /// for a generic container template: [typecheck_templates] must rigidly
    /// check a clone registered into the caller's own `SourceMap`, not the raw
    /// static [CONTAINER_TEMPLATES], whose spans resolve against nothing.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_container_template_declaration_renders_against_its_builtin_source() {
        let mut sources = SourceMap::new();
        let templates = super::container_templates(&mut sources, crate::NumberEncoding::Binary);
        let map_decl = templates
            .list
            .map_declarations
            .first()
            .expect("list.mcrl2 declares at least one map");

        let id = sources.lookup(map_decl.identifier.span.start);
        assert!(
            sources.path(id).contains("list.mcrl2"),
            "expected the List map declaration to be registered against list.mcrl2, got: {}",
            sources.path(id)
        );
        assert!(sources.is_virtual(id), "a builtin template's source must be virtual");
    }

    /// As the container-template test above, for the generic function-update
    /// template that [typecheck_function_update_template] rigidly checks.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_function_update_template_declaration_renders_against_a_registered_source() {
        let mut sources = SourceMap::new();
        let template = super::register_function_update_template(&mut sources, 1);
        let map_decl = template
            .map_declarations
            .first()
            .expect("the function-update template declares at least one map");

        let id = sources.lookup(map_decl.identifier.span.start);
        assert!(
            sources.path(id).contains("function_update_1.mcrl2"),
            "expected the function-update map declaration to be registered against function_update_1.mcrl2, got: {}",
            sources.path(id)
        );
        assert!(sources.is_virtual(id), "a builtin template's source must be virtual");
    }
}
