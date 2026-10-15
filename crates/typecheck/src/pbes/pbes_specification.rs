//! [`PbesSpecification`]: type checking for a whole `UntypedPbes` — the data specification
//! (delegated to [`DataSpecification`]) plus its global variables, propositional-variable
//! equations, and `init`.
//!
//! **Scope.** This checks *sorts and names*: parameter/argument sort-checking, propositional-
//! variable name/arity resolution, quantifier-binder scoping. It does **not** check PBES
//! well-formedness properties like monotonicity or alternation depth of propositional variables —
//! those are a separate semantic analysis, not type checking, out of scope here (mirrors the
//! process crate's own scoping note in its README).

use std::collections::HashMap;
use std::collections::HashSet;

use merc_syntax::IdDecl;
use merc_syntax::PbesEquation;
use merc_syntax::PropVarInst;
use merc_syntax::SortExpression;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::UntypedPbes;

use crate::DataSpecification;
use crate::NumberEncoding;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking;

use super::PbesError;
use super::check;

/// A type-checked mCRL2 PBES: the data specification plus its `glob`, propositional-variable
/// equations, and `init`, all resolved and checked against it. See the module doc comment for
/// what's in and out of scope.
pub struct PbesSpecification {
    /// The original specification, *minus* its data specification.
    spec: UntypedPbes,
    data: DataSpecification,
    /// Every checked expression's `TypingInfo`, merged during the construction walk (see
    /// [`Self::typing_info`]).
    typing: TypingInfo,
}

impl PbesSpecification {
    /// Type checks `spec`, using the default number encoding. See [`Self::from_untyped_with`].
    pub fn from_untyped(spec: UntypedPbes) -> Result<Self, PbesError> {
        Self::from_untyped_with(spec, NumberEncoding::default())
    }

    /// Type checks `spec`: its data specification first (exactly as
    /// [`DataSpecification::from_untyped_with`] does), then its global variables, its
    /// propositional-variable equations' parameters, and every equation's formula and `init`
    /// against them.
    pub fn from_untyped_with(mut spec: UntypedPbes, encoding: NumberEncoding) -> Result<Self, PbesError> {
        // A pure syntactic pass, before anything else needs `spec` — see
        // `resolution::variable_resolution`.
        crate::resolve_pbes_variables(&mut spec);

        let data_spec = std::mem::take(&mut spec.data_specification);
        let mut data = DataSpecification::from_untyped_with(data_spec, encoding, &mut SourceMap::new())?;

        let tables = DeclarationTables::build(&mut data, &spec)?;
        let typing = check::check_pbes_specification(&mut data, &tables, &spec)?;

        Ok(PbesSpecification { spec, data, typing })
    }

    /// The checked data specification.
    pub fn data_specification(&self) -> &DataSpecification {
        &self.data
    }

    /// Consumes `self`, returning the checked data specification.
    pub fn into_data_specification(self) -> DataSpecification {
        self.data
    }

    /// The `glob` declarations, in scope in every equation's formula and in `init`.
    pub fn global_variables(&self) -> &[IdDecl] {
        &self.spec.global_variables
    }

    /// The propositional-variable equations.
    pub fn equations(&self) -> &[PbesEquation] {
        &self.spec.equations
    }

    /// The `init` propositional-variable instantiation.
    pub fn init(&self) -> &PropVarInst {
        &self.spec.init
    }

    /// Every checked expression's typing across the *whole* specification: every data equation
    /// (see [`DataSpecification::typing_info`]) plus every expression checked outside the data
    /// specification, span-keyed so hover/go-to-definition can look up a sub-expression by source
    /// position anywhere in the document (see [`TypingInfo::at_offset`]).
    pub fn typing_info(&self) -> TypingInfo {
        let mut info = self.data.typing_info();
        info.merge(self.typing.clone());
        info
    }
}

/// The resolved global-variable and per-equation-parameter tables, built once by [`Self::build`]
/// and used by [`super::check`]'s scoped walk to resolve every `PropVarInst` it reaches.
pub(super) struct DeclarationTables {
    /// Resolved sort of each `glob` declaration, parallel to `spec.global_variables`.
    pub(super) global_sorts: Vec<ResolvedSortId>,
    /// Resolved `(name, sort)` parameters of each equation, parallel to `spec.equations`.
    pub(super) equation_params: Vec<Vec<(String, ResolvedSortId)>>,
    /// `spec.equations[i].variable.identifier.span`, parallel to `equation_params` — mirrors
    /// `crate::process::process_specification::DeclarationTables::action_decl_spans`.
    pub(super) equation_decl_spans: Vec<Span>,
    /// Propositional-variable name -> index into `spec.equations`/`equation_params`. A single
    /// slot per name, not a `Vec<usize>`: unlike the process crate's actions/processes, PBES
    /// equations are declared once each — no overloading.
    pub(super) equations_by_name: HashMap<String, usize>,
}

impl DeclarationTables {
    fn build(data: &mut DataSpecification, spec: &UntypedPbes) -> Result<Self, PbesError> {
        let mut global_sorts = Vec::with_capacity(spec.global_variables.len());
        let mut seen_globals = HashSet::new();
        for decl in &spec.global_variables {
            if !seen_globals.insert(decl.identifier.as_str()) {
                return Err(PbesError::DuplicateGlobalVariable {
                    name: decl.identifier.node.clone(),
                    span: decl.identifier.span.clone(),
                });
            }
            global_sorts.push(resolve_declared_sort(data, &decl.sort)?);
        }

        let mut equation_params = Vec::with_capacity(spec.equations.len());
        let mut equation_decl_spans = Vec::with_capacity(spec.equations.len());
        let mut equations_by_name: HashMap<String, usize> = HashMap::new();
        for (index, eqn) in spec.equations.iter().enumerate() {
            let mut params = Vec::with_capacity(eqn.variable.parameters.len());
            let mut seen = HashSet::new();
            for param in &eqn.variable.parameters {
                if !seen.insert(param.identifier.as_str()) {
                    return Err(PbesError::DuplicateEquationParameter {
                        equation: eqn.variable.identifier.node.clone(),
                        name: param.identifier.node.clone(),
                        span: param.identifier.span.clone(),
                    });
                }
                let sort = resolve_declared_sort(data, &param.sort)?;
                params.push((param.identifier.node.clone(), sort));
            }

            if equations_by_name
                .insert(eqn.variable.identifier.node.clone(), index)
                .is_some()
            {
                return Err(PbesError::DuplicatePropositionalVariable {
                    name: eqn.variable.identifier.node.clone(),
                    span: eqn.variable.identifier.span.clone(),
                });
            }
            equation_params.push(params);
            equation_decl_spans.push(eqn.variable.identifier.span.clone());
        }

        Ok(DeclarationTables {
            global_sorts,
            equation_params,
            equation_decl_spans,
            equations_by_name,
        })
    }
}

/// Resolves a sort expression occurring in a `glob`/PBES-equation-parameter declaration, see
/// [`checking::resolve_declared_sort`].
pub(super) fn resolve_declared_sort(
    data: &mut DataSpecification,
    sort: &SortExpression,
) -> Result<ResolvedSortId, PbesError> {
    checking::resolve_declared_sort(data, sort, |span| PbesError::AnonymousStructInDeclaration { span })
}
