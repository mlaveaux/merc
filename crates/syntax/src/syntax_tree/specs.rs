use itertools::Itertools;
use merc_pest_consume::Error;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;
use pest::error::ErrorVariant;
use std::fmt;
use std::hash::Hash;

use crate::ActionName;
use crate::ConstructorId;
use crate::FixedPointOperator;
use crate::IdDecl;
use crate::MapId;
use crate::Mcrl2Parser;
use crate::ProcessExpr;
use crate::Rule;
use crate::StateFrm;
use crate::spanned::Spanned;

use super::DataExpr;
use super::EqnSpecId;
use super::EquationId;
use super::ParseNode;
use super::ParseResult;
use super::PbesExpr;
use super::PresExpr;
use super::ProcessName;
use super::PropVarName;
use super::SortExpression;
use super::SortId;
use super::TypeVarId;

/// A complete mCRL2 process specification.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedProcessSpecification {
    pub data_specification: UntypedDataSpecification,
    pub global_variables: Vec<IdDecl>,
    pub action_declarations: Vec<ActDecl>,
    pub process_declarations: Vec<ProcDecl>,
    pub init: Option<ProcessExpr>,
}

impl UntypedProcessSpecification {
    /// Merges another process specification's declarations into this one.
    ///
    /// `other.init` is discarded: the importing file's own `init` always wins.
    /// A file meant to be imported would not usually declare one anyway.
    pub fn merge(&mut self, other: &UntypedProcessSpecification) {
        self.data_specification.merge(&other.data_specification);
        self.global_variables.extend_from_slice(&other.global_variables);
        self.action_declarations.extend_from_slice(&other.action_declarations);
        self.process_declarations.extend_from_slice(&other.process_declarations);
    }
}

/// An mCRL2 data specification.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedDataSpecification {
    pub sort_declarations: Vec<SortDecl>,
    pub constructor_declarations: Vec<IdDecl<ConstructorId>>,
    pub map_declarations: Vec<IdDecl<MapId>>,
    pub equation_declarations: Vec<EqnSpec>,
    pub type_var_declarations: Vec<TypeVarDecl>,
}

impl UntypedDataSpecification {
    /// Returns true if the data specification is empty.
    pub fn is_empty(&self) -> bool {
        self.sort_declarations.is_empty()
            && self.constructor_declarations.is_empty()
            && self.map_declarations.is_empty()
            && self.equation_declarations.is_empty()
            && self.type_var_declarations.is_empty()
    }

    /// Merges another data specification into the current one.
    pub fn merge(&mut self, other_spec: &UntypedDataSpecification) {
        self.sort_declarations.extend_from_slice(&other_spec.sort_declarations);
        self.constructor_declarations
            .extend_from_slice(&other_spec.constructor_declarations);
        self.map_declarations.extend_from_slice(&other_spec.map_declarations);
        self.equation_declarations
            .extend_from_slice(&other_spec.equation_declarations);
        self.type_var_declarations
            .extend_from_slice(&other_spec.type_var_declarations);
    }
}

/// A bound sort (type) variable's own declaration, introduced by a `type_var` block.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TypeVarDecl {
    /// The type variable's own name (`S`).
    pub identifier: String,
    /// Where the type variable is declared.
    pub span: Span,
    /// Unique ID assigned to this declaration during name resolution.
    pub id: Option<TypeVarId>,
}

impl TypeVarDecl {
    /// Creates a new type variable declaration with the given identifier and span.
    pub fn new(identifier: String, span: Span) -> Self {
        TypeVarDecl {
            identifier,
            span,
            id: None,
        }
    }
}

/// An mCRL2 parameterised boolean equation system (PBES).
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedPbes {
    pub data_specification: UntypedDataSpecification,
    pub global_variables: Vec<IdDecl>,
    pub equations: Vec<PbesEquation>,
    pub init: PropVarInst,
}

/// An mCRL2 parameterised real equation system (PRES).
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedPres {
    pub data_specification: UntypedDataSpecification,
    pub global_variables: Vec<IdDecl>,
    pub equations: Vec<PresEquation>,
    pub init: PropVarInst,
}

/// A `pbes`/`pres` equation's own declaration.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PropVarDecl {
    pub identifier: PropVarName,
    pub parameters: Vec<IdDecl>,
    pub span: Span,
}

impl PropVarDecl {
    /// Creates a new propositional variable declaration with the given identifier and parameters.
    pub fn new(identifier: String, parameters: Vec<IdDecl>) -> Self {
        PropVarDecl {
            identifier: PropVarName {
                node: identifier,
                span: Span::default(),
            },
            parameters,
            span: Span::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct PropVarInstData {
    pub identifier: PropVarName,
    pub arguments: Vec<DataExpr>,
}

/// A propositional-variable instantiation, paired with the source [Span] it was parsed from.
/// Equality/ordering/hashing ignore the span, per [Spanned]'s documented convention.
pub type PropVarInst = Spanned<PropVarInstData>;

impl PropVarInstData {
    /// Wraps this data together with a source `span`.
    pub fn spanned(self, span: Span) -> PropVarInst {
        Spanned { node: self, span }
    }
}

impl PropVarInst {
    /// Creates a new instance of a propositional variable with the given identifier and
    /// arguments. Both the instantiation and the identifier itself get [Span::default], for a
    /// synthetic instantiation with no source location.
    pub fn new(identifier: String, arguments: Vec<DataExpr>) -> Self {
        PropVarInstData {
            identifier: PropVarName {
                node: identifier,
                span: Span::default(),
            },
            arguments,
        }
        .spanned(Span::default())
    }
}

/// Sort declaration
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SortDecl {
    /// Sort identifier
    pub identifier: String,
    /// Sort expression (if structured)
    pub expr: Option<SortExpression>,
    /// Where the sort is defined
    pub span: Span,
    /// Unique ID assigned to this declaration during name resolution.
    pub id: Option<SortId>,
}

impl SortDecl {
    /// Creates a new sort declaration with the given identifier, expression, and span.
    pub fn new(identifier: String, expr: Option<SortExpression>, span: Span) -> Self {
        SortDecl {
            identifier,
            expr,
            span,
            id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EqnSpecData {
    pub variables: Vec<IdDecl>,
    pub equations: Vec<EqnDecl>,
    /// Unique ID assigned to this block during declaration-id resolution.
    pub id: Option<EqnSpecId>,
}

/// An equation-specification block (`var ... eqn ...`), paired with the source [Span] of the
/// whole block, from `var`/`eqn` (whichever comes first) to at least the final `;`.
/// Equality/ordering/hashing ignore the span, per [Spanned]'s documented convention.
pub type EqnSpec = Spanned<EqnSpecData>;

impl EqnSpecData {
    /// Wraps this data together with a source `span`.
    pub fn spanned(self, span: Span) -> EqnSpec {
        Spanned { node: self, span }
    }
}

/// Equation declaration
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EqnDecl {
    pub condition: Option<DataExpr>,
    pub lhs: DataExpr,
    pub rhs: DataExpr,
    pub span: Span,
    /// Unique ID assigned to this equation during declaration-id resolution,
    /// local to its enclosing [EqnSpec].
    pub id: Option<EquationId>,
}

/// Action declaration.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ActDecl {
    pub identifier: ActionName,
    pub args: Vec<SortExpression>,
    pub span: Span,
}

/// Process declaration.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ProcDecl {
    pub identifier: ProcessName,
    pub params: Vec<IdDecl>,
    pub body: ProcessExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct UntypedStateFrmSpec {
    pub data_specification: UntypedDataSpecification,
    pub action_declarations: Vec<ActDecl>,
    pub formula: StateFrm,
}

/// Represents a multi action label `a | b | c ...`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct MultiActionLabel {
    pub actions: Vec<ActionName>,
}

impl MultiActionLabel {
    /// Creates a new multi-action label from a list of action identifiers.
    pub fn new(actions: Vec<ActionName>) -> Self {
        MultiActionLabel { actions }
    }

    /// Returns true if the multi-action label is empty (i.e., contains no actions).
    pub fn is_tau_label(&self) -> bool {
        self.actions.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Action {
    pub id: ActionName,
    pub args: Vec<DataExpr>,
}

impl Action {
    /// Creates a new action from an identifier and a list of arguments. `id` gets
    /// [Span::default], for a synthetic action with no source location.
    pub fn new(id: String, args: Vec<DataExpr>) -> Self {
        Action {
            id: ActionName {
                node: id,
                span: Span::default(),
            },
            args,
        }
    }
}

#[derive(Clone, Debug, Eq)]
pub struct MultiAction {
    pub actions: Vec<Action>,
}

impl MultiAction {
    /// Creates a new multi-action from a list of actions.
    pub fn new(actions: Vec<Action>) -> Self {
        MultiAction { actions }
    }

    /// Creates the empty multi-action, which represents the tau action.
    pub fn tau() -> Self {
        MultiAction { actions: Vec::new() }
    }
}

impl PartialEq for MultiAction {
    fn eq(&self, other: &Self) -> bool {
        // A multi-action is a multiset of actions, so equality must be
        // order-independent but multiplicity-sensitive. Sort clones (the same
        // canonicalization `Hash` below uses) and compare the results.
        if self.actions.len() != other.actions.len() {
            return false;
        }

        let mut self_actions = self.actions.clone();
        let mut other_actions = other.actions.clone();
        self_actions.sort();
        other_actions.sort();

        self_actions == other_actions
    }
}

impl Hash for MultiAction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut actions = self.actions.clone();
        // Sort the action ids to ensure that the hash is independent of the order.
        actions.sort();
        for action in actions {
            action.hash(state);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PbesEquation {
    pub operator: FixedPointOperator,
    pub variable: PropVarDecl,
    pub formula: PbesExpr,
    pub span: Span,
}

impl PbesEquation {
    /// Creates a new PBES equation with the given operator, variable and formula.
    pub fn new(operator: FixedPointOperator, variable: PropVarDecl, formula: PbesExpr) -> Self {
        PbesEquation {
            operator,
            variable,
            formula,
            span: Span::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PresEquation {
    pub operator: FixedPointOperator,
    pub variable: PropVarDecl,
    pub formula: PresExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Rename {
    pub from: ActionName,
    pub to: ActionName,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct CommExpr {
    pub from: MultiActionLabel,
    pub to: ActionName,
}

impl CommExpr {
    /// Creates a new communication expression from a multi-action label and a target action identifier.
    pub fn new(from: MultiActionLabel, to: ActionName) -> Self {
        CommExpr { from, to }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct UntypedActionRenameSpec {
    pub data_specification: UntypedDataSpecification,
    pub action_declarations: Vec<ActDecl>,
    pub rename_declarations: Vec<ActionRenameDecl>,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ActionRenameDecl {
    pub variables_specification: Vec<IdDecl>,
    pub rename_rule: ActionRenameRule,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ActionRenameRule {
    pub condition: Option<DataExpr>,
    pub action: Action,
    pub rhs: ActionRHS,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ActionRHS {
    Tau,
    Delta,
    Action(Action),
}

impl fmt::Display for UntypedProcessSpecification {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;

        if !self.action_declarations.is_empty() {
            writeln!(f, "act")?;
            for act_decl in &self.action_declarations {
                writeln!(f, "   {act_decl};")?;
            }

            writeln!(f)?;
        }

        if !self.process_declarations.is_empty() {
            writeln!(f, "proc")?;
            for proc_decl in &self.process_declarations {
                writeln!(f, "   {proc_decl};")?;
            }

            writeln!(f)?;
        }

        if !self.global_variables.is_empty() {
            writeln!(f, "glob")?;
            for var_decl in &self.global_variables {
                writeln!(f, "   {var_decl};")?;
            }

            writeln!(f)?;
        }

        if let Some(init) = &self.init {
            writeln!(f, "init {init};")?;
        }
        Ok(())
    }
}

impl fmt::Display for UntypedDataSpecification {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.type_var_declarations.is_empty() {
            writeln!(f, "type_var")?;
            for decl in &self.type_var_declarations {
                writeln!(f, "   {};", decl.identifier)?;
            }

            writeln!(f)?;
        }

        if !self.sort_declarations.is_empty() {
            writeln!(f, "sort")?;
            for decl in &self.sort_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        if !self.constructor_declarations.is_empty() {
            writeln!(f, "cons")?;
            for decl in &self.constructor_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        if !self.map_declarations.is_empty() {
            writeln!(f, "map")?;
            for decl in &self.map_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        for decl in &self.equation_declarations {
            writeln!(f, "{decl}")?;
        }
        Ok(())
    }
}

impl fmt::Display for UntypedPbes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;
        writeln!(f)?;
        if !self.global_variables.is_empty() {
            writeln!(f, "glob")?;
            for var_decl in &self.global_variables {
                writeln!(f, "   {var_decl};")?;
            }

            writeln!(f)?;
        }
        writeln!(f)?;

        if !self.equations.is_empty() {
            writeln!(f, "pbes")?;
            for equation in &self.equations {
                writeln!(f, "   {equation};")?;
            }
        }

        writeln!(f, "init {};", self.init)
    }
}

impl fmt::Display for PropVarInst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.arguments.iter().format(", "))
        }
    }
}

impl fmt::Display for PbesEquation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} = {}", self.operator, self.variable, self.formula)
    }
}

impl fmt::Display for PropVarDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.parameters.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.parameters.iter().format(", "))
        }
    }
}

impl fmt::Display for UntypedPres {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;
        writeln!(f)?;
        if !self.global_variables.is_empty() {
            writeln!(f, "glob")?;
            for var_decl in &self.global_variables {
                writeln!(f, "   {var_decl};")?;
            }

            writeln!(f)?;
        }
        writeln!(f)?;

        if !self.equations.is_empty() {
            writeln!(f, "pres")?;
            for equation in &self.equations {
                writeln!(f, "   {equation};")?;
            }
        }

        writeln!(f, "init {};", self.init)
    }
}

impl fmt::Display for PresEquation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} = {}", self.operator, self.variable, self.formula)
    }
}

impl fmt::Display for EqnSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // The grammar requires at least one declaration after `var`, so only
        // emit the section when there are variables to declare.
        if !self.variables.is_empty() {
            writeln!(f, "var")?;
            for decl in &self.variables {
                writeln!(f, "   {decl};")?;
            }
        }

        writeln!(f, "eqn")?;
        for decl in &self.equations {
            writeln!(f, "   {decl};")?;
        }
        Ok(())
    }
}

impl fmt::Display for SortDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.identifier)?;

        if let Some(expr) = &self.expr {
            write!(f, " = {expr}")?;
        }

        Ok(())
    }
}

impl fmt::Display for ActDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An action declaration is `id: sort # sort # ...`, matching the
        // `IdList ~ ":" ~ SortProduct` grammar rule.
        if self.args.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}: {}", self.identifier, self.args.iter().format(" # "))
        }
    }
}

impl fmt::Display for EqnDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.condition {
            Some(condition) => write!(f, "{} -> {} = {}", condition, self.lhs, self.rhs),
            None => write!(f, "{} = {}", self.lhs, self.rhs),
        }
    }
}

impl fmt::Display for UntypedStateFrmSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;

        // Wrap the formula in a `form ...;` section: the bare-formula grammar
        // alternative is only valid when no specification elements precede it.
        writeln!(f, "form {};", self.formula)
    }
}

impl fmt::Display for MultiAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "tau")
        } else {
            write!(f, "{}", self.actions.iter().format("|"))
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "{}", self.id)
        } else {
            write!(f, "{}({})", self.id, self.args.iter().format(", "))
        }
    }
}

impl fmt::Display for ProcDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.params.is_empty() {
            write!(f, "{} = {}", self.identifier, self.body)
        } else {
            write!(
                f,
                "{}({}) = {}",
                self.identifier,
                self.params.iter().format(", "),
                self.body
            )
        }
    }
}

impl fmt::Display for CommExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.from, self.to)
    }
}

impl fmt::Display for Rename {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.from, self.to)
    }
}

impl fmt::Display for ActionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.node)
    }
}

impl fmt::Display for MultiActionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "tau")
        } else {
            write!(f, "{}", self.actions.iter().format("|"))
        }
    }
}

/// Declarations shared by every `UntypedDataSpecification`-bearing top-level spec
/// (`MCRL2Spec`, `ActionRenameSpec`, `StateFrmSpec`), collected while walking that spec's own
/// top-level declaration rules.
#[derive(Default)]
struct DataSpecDeclarations {
    map_declarations: Vec<IdDecl<MapId>>,
    constructor_declarations: Vec<IdDecl<ConstructorId>>,
    equation_declarations: Vec<EqnSpec>,
    sort_declarations: Vec<SortDecl>,
    type_var_declarations: Vec<TypeVarDecl>,
}

impl DataSpecDeclarations {
    fn into_data_specification(self) -> UntypedDataSpecification {
        UntypedDataSpecification {
            map_declarations: self.map_declarations,
            constructor_declarations: self.constructor_declarations,
            equation_declarations: self.equation_declarations,
            sort_declarations: self.sort_declarations,
            type_var_declarations: self.type_var_declarations,
        }
    }
}

struct ProcessSpecDeclarations {
    data: DataSpecDeclarations,
    action_declarations: Vec<ActDecl>,
    global_variables: Vec<IdDecl>,
    process_declarations: Vec<ProcDecl>,
    init: Option<ProcessExpr>,
}

/// Walks `MCRL2Spec`'s own top-level declaration rules, collecting each into its matching field.
fn collect_process_spec(spec: ParseNode) -> ParseResult<ProcessSpecDeclarations> {
    let mut decls = ProcessSpecDeclarations {
        data: DataSpecDeclarations::default(),
        action_declarations: Vec::new(),
        global_variables: Vec::new(),
        process_declarations: Vec::new(),
        init: None,
    };

    for child in spec.into_children() {
        match child.as_rule() {
            Rule::ActSpec => {
                decls.action_declarations.extend(Mcrl2Parser::ActSpec(child)?);
            }
            Rule::ConsSpec => {
                decls
                    .data
                    .constructor_declarations
                    .append(&mut Mcrl2Parser::ConsSpec(child)?);
            }
            Rule::MapSpec => {
                decls.data.map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
            }
            Rule::GlobVarSpec => {
                decls.global_variables.append(&mut Mcrl2Parser::GlobVarSpec(child)?);
            }
            Rule::EqnSpec => {
                decls
                    .data
                    .equation_declarations
                    .append(&mut Mcrl2Parser::EqnSpec(child)?);
            }
            Rule::ProcSpec => {
                decls.process_declarations.append(&mut Mcrl2Parser::ProcSpec(child)?);
            }
            Rule::SortSpec => {
                decls.data.sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
            }
            Rule::TypeVarSpec => {
                decls
                    .data
                    .type_var_declarations
                    .append(&mut Mcrl2Parser::TypeVarSpec(child)?);
            }
            Rule::Init => {
                if decls.init.is_some() {
                    return Err(Error::new_from_span(
                        ErrorVariant::CustomError {
                            message: "Multiple init expressions are not allowed".to_string(),
                        },
                        child.as_span(),
                    ));
                }

                decls.init = Some(Mcrl2Parser::Init(child)?);
            }
            Rule::EOI => {
                // End of input
                break;
            }
            _ => {
                unimplemented!("Unexpected rule: {:?}", child.as_rule());
            }
        }
    }

    Ok(decls)
}

struct ActionRenameSpecDeclarations {
    data: DataSpecDeclarations,
    action_declarations: Vec<ActDecl>,
    rename_declarations: Vec<ActionRenameDecl>,
}

/// Walks `ActionRenameSpec`'s own top-level declaration rules, collecting each into its matching
/// field.
fn collect_action_rename_spec(spec: ParseNode) -> ParseResult<ActionRenameSpecDeclarations> {
    let mut decls = ActionRenameSpecDeclarations {
        data: DataSpecDeclarations::default(),
        action_declarations: Vec::new(),
        rename_declarations: Vec::new(),
    };

    for child in spec.into_children() {
        match child.as_rule() {
            Rule::ConsSpec => {
                decls
                    .data
                    .constructor_declarations
                    .append(&mut Mcrl2Parser::ConsSpec(child)?);
            }
            Rule::MapSpec => {
                decls.data.map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
            }
            Rule::EqnSpec => {
                decls
                    .data
                    .equation_declarations
                    .append(&mut Mcrl2Parser::EqnSpec(child)?);
            }
            Rule::SortSpec => {
                decls.data.sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
            }
            Rule::TypeVarSpec => {
                decls
                    .data
                    .type_var_declarations
                    .append(&mut Mcrl2Parser::TypeVarSpec(child)?);
            }
            Rule::ActSpec => {
                decls.action_declarations.append(&mut Mcrl2Parser::ActSpec(child)?);
            }
            Rule::ActionRenameRuleSpec => {
                decls
                    .rename_declarations
                    .append(&mut Mcrl2Parser::ActionRenameRuleSpec(child)?);
            }
            Rule::EOI => {
                // End of input
                break;
            }
            _ => {
                unimplemented!("Unexpected rule: {:?}", child.as_rule());
            }
        }
    }

    Ok(decls)
}

struct StateFrmSpecDeclarations {
    data: DataSpecDeclarations,
    action_declarations: Vec<ActDecl>,
    formula: Option<StateFrm>,
}

/// Consumes a single `StateFrmSpecElt` child (already unwrapped to its one inner rule),
/// collecting it into its matching field.
fn collect_state_frm_spec_elt(element: ParseNode, decls: &mut StateFrmSpecDeclarations) -> ParseResult<()> {
    match element.as_rule() {
        Rule::ConsSpec => {
            decls
                .data
                .constructor_declarations
                .append(&mut Mcrl2Parser::ConsSpec(element)?);
        }
        Rule::MapSpec => {
            decls.data.map_declarations.append(&mut Mcrl2Parser::MapSpec(element)?);
        }
        Rule::EqnSpec => {
            decls
                .data
                .equation_declarations
                .append(&mut Mcrl2Parser::EqnSpec(element)?);
        }
        Rule::SortSpec => {
            decls
                .data
                .sort_declarations
                .append(&mut Mcrl2Parser::SortSpec(element)?);
        }
        Rule::TypeVarSpec => {
            decls
                .data
                .type_var_declarations
                .append(&mut Mcrl2Parser::TypeVarSpec(element)?);
        }
        Rule::ActSpec => {
            decls.action_declarations.append(&mut Mcrl2Parser::ActSpec(element)?);
        }
        _ => {
            unimplemented!("Unexpected rule in StateFrmSpecElt: {:?}", element.as_rule());
        }
    }
    Ok(())
}

/// Walks `StateFrmSpec`'s own top-level declaration rules, collecting each into its matching
/// field.
fn collect_state_frm_spec(spec: ParseNode) -> ParseResult<StateFrmSpecDeclarations> {
    let mut decls = StateFrmSpecDeclarations {
        data: DataSpecDeclarations::default(),
        action_declarations: Vec::new(),
        formula: None,
    };

    for child in spec.into_children() {
        match child.as_rule() {
            Rule::StateFrmSpecElt => {
                let element = child
                    .into_children()
                    .next()
                    .expect("StateFrmSpecElt has exactly one child");
                collect_state_frm_spec_elt(element, &mut decls)?;
            }
            Rule::StateFrm => {
                if decls.formula.is_some() {
                    return Err(Error::new_from_span(
                        ErrorVariant::CustomError {
                            message: "Multiple state formula specifications are not allowed".to_string(),
                        },
                        child.as_span(),
                    ));
                }
                decls.formula = Some(Mcrl2Parser::StateFrm(child)?);
            }
            Rule::FormSpec => {
                if decls.formula.is_some() {
                    return Err(Error::new_from_span(
                        ErrorVariant::CustomError {
                            message: "Multiple state formula specifications are not allowed".to_string(),
                        },
                        child.as_span(),
                    ));
                }
                decls.formula = Some(Mcrl2Parser::FormSpec(child)?);
            }
            Rule::EOI => {
                // End of input
                break;
            }
            _ => {
                unimplemented!("Unexpected rule: {:?}", child.as_rule());
            }
        }
    }

    Ok(decls)
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    // Although these are not public, they are the main entry points for consuming the parse tree.
    pub(crate) fn MCRL2Spec(spec: ParseNode) -> ParseResult<UntypedProcessSpecification> {
        let decls = collect_process_spec(spec)?;
        Ok(UntypedProcessSpecification {
            data_specification: decls.data.into_data_specification(),
            global_variables: decls.global_variables,
            action_declarations: decls.action_declarations,
            process_declarations: decls.process_declarations,
            init: decls.init,
        })
    }

    pub fn PbesSpec(spec: ParseNode) -> ParseResult<UntypedPbes> {
        let mut data_specification = None;
        let mut global_variables = None;
        let mut equations = None;
        let mut init = None;

        let span = spec.as_span();
        for child in spec.into_children() {
            match child.as_rule() {
                Rule::DataSpecBody => {
                    data_specification = Some(Mcrl2Parser::DataSpecBody(child)?);
                }
                Rule::GlobVarSpec => {
                    global_variables = Some(Mcrl2Parser::GlobVarSpec(child)?);
                }
                Rule::PbesEqnSpec => {
                    equations = Some(Mcrl2Parser::PbesEqnSpec(child)?);
                }
                Rule::PbesInit => {
                    init = Some(Mcrl2Parser::PbesInit(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        Ok(UntypedPbes {
            data_specification: data_specification.unwrap_or_default(),
            global_variables: global_variables.unwrap_or_default(),
            equations: equations.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PBES requires a (possibly empty) pbes equation section".to_string(),
                    },
                    span,
                )
            })?,
            init: init.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PBES requires an init declaration".to_string(),
                    },
                    span,
                )
            })?,
        })
    }

    fn PbesInit(init: ParseNode) -> ParseResult<PropVarInst> {
        match_nodes!(init.into_children();
            [PropVarInst(inst)] => {
                Ok(inst)
            }
        )
    }

    fn PbesEqnSpec(spec: ParseNode) -> ParseResult<Vec<PbesEquation>> {
        match_nodes!(spec.into_children();
            [PbesEqnDecl(equations)..] => {
                Ok(equations.collect())
            },
        )
    }

    fn PbesEqnDecl(decl: ParseNode) -> ParseResult<PbesEquation> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [FixedPointOperator(operator), PropVarDecl(variable), PbesExpr(formula)] => {
                Ok(PbesEquation {
                    operator,
                    variable,
                    formula,
                    span: span.into(),
                })
            },
        )
    }

    fn FixedPointOperator(op: ParseNode) -> ParseResult<FixedPointOperator> {
        match op.into_children().next().unwrap().as_rule() {
            Rule::FixedPointMu => Ok(FixedPointOperator::Least),
            Rule::FixedPointNu => Ok(FixedPointOperator::Greatest),
            x => unimplemented!("This is not a fixed point operator: {:?}", x),
        }
    }

    fn PropVarDecl(decl: ParseNode) -> ParseResult<PropVarDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [Id(identifier), VarsDeclList(params)] => {
                Ok(PropVarDecl {
                    identifier,
                    parameters: params,
                    span: span.into(),
                })
            },
            [Id(identifier)] => {
                let span = identifier.span.clone();
                Ok(PropVarDecl {
                    identifier,
                    parameters: Vec::new(),
                    span,
                })
            }
        )
    }

    pub(crate) fn PropVarInst(inst: ParseNode) -> ParseResult<PropVarInst> {
        let span = inst.as_span();
        match_nodes!(inst.into_children();
            [Id(identifier)] => {
                Ok(PropVarInstData {
                    identifier,
                    arguments: Vec::new(),
                }.spanned(span.into()))
            },
            [Id(identifier), DataExprList(arguments)] => {
                Ok(PropVarInstData {
                    identifier,
                    arguments,
                }.spanned(span.into()))
            }
        )
    }

    pub fn PresSpec(spec: ParseNode) -> ParseResult<UntypedPres> {
        let mut data_specification = None;
        let mut global_variables = None;
        let mut equations = None;
        let mut init = None;

        let span = spec.as_span();
        for child in spec.into_children() {
            match child.as_rule() {
                Rule::DataSpecBody => {
                    data_specification = Some(Mcrl2Parser::DataSpecBody(child)?);
                }
                Rule::GlobVarSpec => {
                    global_variables = Some(Mcrl2Parser::GlobVarSpec(child)?);
                }
                Rule::PresEqnSpec => {
                    equations = Some(Mcrl2Parser::PresEqnSpec(child)?);
                }
                Rule::PbesInit => {
                    init = Some(Mcrl2Parser::PbesInit(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        Ok(UntypedPres {
            data_specification: data_specification.unwrap_or_default(),
            global_variables: global_variables.unwrap_or_default(),
            equations: equations.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PRES requires a (possibly empty) pres equation section".to_string(),
                    },
                    span,
                )
            })?,
            init: init.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PRES requires an init declaration".to_string(),
                    },
                    span,
                )
            })?,
        })
    }

    fn PresEqnSpec(spec: ParseNode) -> ParseResult<Vec<PresEquation>> {
        match_nodes!(spec.into_children();
            [PresEqnDecl(equations)..] => {
                Ok(equations.collect())
            },
        )
    }

    fn PresEqnDecl(decl: ParseNode) -> ParseResult<PresEquation> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [FixedPointOperator(operator), PropVarDecl(variable), PresExpr(formula)] => {
                Ok(PresEquation {
                    operator,
                    variable,
                    formula,
                    span: span.into(),
                })
            },
        )
    }

    fn ActSpec(spec: ParseNode) -> ParseResult<Vec<ActDecl>> {
        match_nodes!(spec.into_children();
            [ActDecl(decls)..] => {
                Ok(decls.flatten().collect())
            },
        )
    }

    fn ActDecl(decl: ParseNode) -> ParseResult<Vec<ActDecl>> {
        // Shared by every identifier in the `a, b: Nat` group below: there is no narrower
        // per-name "whole declaration" extent than the group itself.
        let span: Span = decl.as_span().into();
        match_nodes!(decl.into_children();
            [IdList(identifiers)] => {
                Ok(identifiers.into_iter().map(|(name, id_span)| ActDecl {
                    identifier: ActionName { node: name, span: id_span },
                    args: Vec::new(),
                    span: span.clone(),
                }).collect())
            },
            [IdList(identifiers), SortProduct(args)] => {
                Ok(identifiers.into_iter().map(|(name, id_span)| ActDecl {
                    identifier: ActionName { node: name, span: id_span },
                    args: args.clone(),
                    span: span.clone(),
                }).collect())
            },
        )
    }

    fn GlobVarSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(spec.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            }
        )
    }

    pub(crate) fn DataSpec(spec: ParseNode) -> ParseResult<UntypedDataSpecification> {
        // `DataSpecBody` always matches (its repetition allows zero declarations), so it is
        // always the first child, ahead of `EOI`.
        let Some(child) = spec.into_children().next() else {
            return Ok(UntypedDataSpecification::default());
        };

        match child.as_rule() {
            Rule::DataSpecBody => Mcrl2Parser::DataSpecBody(child),
            rule => unimplemented!("Unexpected rule: {:?}", rule),
        }
    }

    pub(crate) fn DataSpecBody(spec: ParseNode) -> ParseResult<UntypedDataSpecification> {
        let mut map_declarations = Vec::new();
        let mut equation_declarations = Vec::new();
        let mut constructor_declarations = Vec::new();
        let mut sort_declarations = Vec::new();
        let mut type_var_declarations = Vec::new();

        for child in spec.into_children() {
            match child.as_rule() {
                Rule::ConsSpec => {
                    constructor_declarations.append(&mut Mcrl2Parser::ConsSpec(child)?);
                }
                Rule::MapSpec => {
                    map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
                }
                Rule::EqnSpec => {
                    equation_declarations.append(&mut Mcrl2Parser::EqnSpec(child)?);
                }
                Rule::SortSpec => {
                    sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
                }
                Rule::TypeVarSpec => {
                    type_var_declarations.append(&mut Mcrl2Parser::TypeVarSpec(child)?);
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        let data_specification = UntypedDataSpecification {
            map_declarations,
            equation_declarations,
            constructor_declarations,
            sort_declarations,
            type_var_declarations,
        };

        Ok(data_specification)
    }

    pub fn ActionRenameSpec(spec: ParseNode) -> ParseResult<UntypedActionRenameSpec> {
        let decls = collect_action_rename_spec(spec)?;
        Ok(UntypedActionRenameSpec {
            data_specification: decls.data.into_data_specification(),
            action_declarations: decls.action_declarations,
            rename_declarations: decls.rename_declarations,
        })
    }

    fn MapSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl<MapId>>> {
        match_nodes!(spec.into_children();
            [IdsDecl(decls)..] => {
                Ok(decls.flatten().map(IdDecl::retag).collect())
            }
        )
    }

    fn SortSpec(spec: ParseNode) -> ParseResult<Vec<SortDecl>> {
        match_nodes!(spec.into_children();
            [SortDecl(decls)..] => {
                Ok(decls.flatten().collect())
            }
        )
    }

    fn SortDecl(decl: ParseNode) -> ParseResult<Vec<SortDecl>> {
        match_nodes!(decl.into_children();
            // The alias form (`sort A = Bool;`) always names exactly one sort per node.
            [IdAt(identifier), SortExpr(expr)] => {
                Ok(vec![SortDecl::new(identifier.node, Some(expr), identifier.span)])
            },
            // `sort A, B, C;`: each gets its own precise identifier span (see `IdList`).
            [IdList(ids)] => {
                Ok(ids.into_iter().map(|(identifier, span)| SortDecl::new(identifier, None, span)).collect())
            },
        )
    }

    fn TypeVarSpec(spec: ParseNode) -> ParseResult<Vec<TypeVarDecl>> {
        match_nodes!(spec.into_children();
            [IdList(ids)..] => {
                Ok(ids.flatten().map(|(identifier, span)| TypeVarDecl::new(identifier, span)).collect())
            }
        )
    }

    fn ConsSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl<ConstructorId>>> {
        match_nodes!(spec.into_children();
            [IdsDecl(decls)..] => {
                Ok(decls.flatten().map(IdDecl::retag).collect())
            }
        )
    }

    fn Init(init: ParseNode) -> ParseResult<ProcessExpr> {
        match_nodes!(init.into_children();
            [ProcExpr(expr)] => {
                Ok(expr)
            }
        )
    }

    fn ProcSpec(spec: ParseNode) -> ParseResult<Vec<ProcDecl>> {
        match_nodes!(spec.into_children();
            [ProcDecl(decls)..] => {
                Ok(decls.collect())
            },
        )
    }

    fn ProcDecl(decl: ParseNode) -> ParseResult<ProcDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [Id(identifier), VarsDeclList(params), ProcExpr(body)] => {
                Ok(ProcDecl {
                    identifier,
                    params,
                    body,
                    span: span.into(),
                })
            },
            [Id(identifier), ProcExpr(body)] => {
                Ok(ProcDecl {
                    identifier,
                    params: Vec::new(),
                    body,
                    span: span.into(),
                })
            }
        )
    }

    fn VarSpec(vars: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(vars.into_children();
            [VarsDeclList(ids)..] => {
                Ok(ids.flatten().collect())
            },
        )
    }

    fn IdInfix(identifier: ParseNode) -> ParseResult<String> {
        Ok(identifier.as_str().to_string())
    }

    fn IdInfixList(identifiers: ParseNode) -> ParseResult<Vec<(String, Span)>> {
        Ok(identifiers
            .into_children()
            .map(|node| (node.as_str().to_string(), node.as_span().into()))
            .collect())
    }

    fn IdsDecl(decl: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(decl.into_children();
            [IdInfixList(identifiers), SortExpr(sort)] => {
                let id_decls = identifiers.into_iter().map(|(identifier, span)| {
                    IdDecl::new(identifier, sort.clone(), span)
                }).collect();

                Ok(id_decls)
            },
        )
    }

    fn EqnSpec(spec: ParseNode) -> ParseResult<Vec<EqnSpec>> {
        let span = spec.as_span();
        let mut ids = Vec::new();

        match_nodes!(spec.into_children();
            [VarSpec(variables), EqnDecl(decls)..] => {
                ids.push(EqnSpecData {
                    variables,
                    equations: decls.collect(),
                    id: None,
                }.spanned(span.into()));
            },
            [EqnDecl(decls)..] => {
                ids.push(EqnSpecData { variables: Vec::new(), equations: decls.collect(), id: None }.spanned(span.into()));
            },
        );

        Ok(ids)
    }

    fn EqnDecl(decl: ParseNode) -> ParseResult<EqnDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [DataExpr(condition), DataExpr(lhs), DataExpr(rhs)] => {
                Ok(EqnDecl { condition: Some(condition), lhs, rhs, span: span.into(), id: None })
            },
            [DataExpr(lhs), DataExpr(rhs)] => {
                Ok(EqnDecl { condition: None, lhs, rhs, span: span.into(), id: None })
            },
        )
    }

    fn ActionRenameRuleSpec(spec: ParseNode) -> ParseResult<Vec<ActionRenameDecl>> {
        match_nodes!(spec.into_children();
            [VarSpec(variables_specification), ActionRenameRule(renames)..] => {
                Ok(renames.map(|rename_rule| {
                    ActionRenameDecl { variables_specification: variables_specification.clone(), rename_rule }
                }).collect())
            },
            [ActionRenameRule(renames)..] => {
                Ok(renames.map(|rename_rule| {
                    ActionRenameDecl { variables_specification: Vec::new(), rename_rule }
                }).collect())
            },
        )
    }

    fn ActionRenameRule(input: ParseNode) -> ParseResult<ActionRenameRule> {
        match_nodes!(input.into_children();
            [DataExpr(condition), Action(action), ActionRenameRuleRHS(rhs)] => {
                Ok(ActionRenameRule { condition: Some(condition), action, rhs })
            },
            [Action(action), ActionRenameRuleRHS(rhs)] => {
                Ok(ActionRenameRule { condition: None, action, rhs })
            },
        )
    }

    fn ActionRenameRuleRHS(input: ParseNode) -> ParseResult<ActionRHS> {
        match_nodes!(input.into_children();
            [Action(action)] => {
                Ok(ActionRHS::Action(action))
            },
            [MultiActTau(_)] => {
                Ok(ActionRHS::Tau)
            },
            [ProcExprDelta(_)] => {
                Ok(ActionRHS::Delta)
            },
        )
    }

    fn FormSpec(input: ParseNode) -> ParseResult<StateFrm> {
        match_nodes!(input.into_children();
            [StateFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmSpec(spec: ParseNode) -> ParseResult<UntypedStateFrmSpec> {
        let span = spec.as_span();
        let decls = collect_state_frm_spec(spec)?;
        Ok(UntypedStateFrmSpec {
            data_specification: decls.data.into_data_specification(),
            action_declarations: decls.action_declarations,
            formula: decls.formula.ok_or(Error::new_from_span(
                ErrorVariant::CustomError {
                    message: "No state formula found in the state formula specification".to_string(),
                },
                span,
            ))?,
        })
    }

    fn EOI(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }
}
