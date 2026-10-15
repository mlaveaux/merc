use std::collections::HashMap;
use std::collections::HashSet;

use merc_syntax::IdDecl;
use merc_syntax::PresEquation;
use merc_syntax::PropVarInst;
use merc_syntax::SortExpression;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::UntypedPres;

use crate::DataSpecification;
use crate::NumberEncoding;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking;

use super::PresError;
use super::check;

/// A type-checked mCRL2 PRES.
///
/// Does not check PRES well-formedness properties like monotonicity of
/// propositional variables.
pub struct PresSpecification {
    /// The original specification, *minus* its untyped data specification.
    spec: UntypedPres,
    /// The type checked data specification.
    data: DataSpecification,
    /// Every checked expression's `TypingInfo`, merged during the construction walk (see
    /// [`Self::typing_info`]).
    typing: TypingInfo,
}

impl PresSpecification {
    /// Type checks `spec`, using the default number encoding. See [`Self::from_untyped_with`].
    pub fn from_untyped(spec: UntypedPres) -> Result<Self, PresError> {
        Self::from_untyped_with(spec, NumberEncoding::default())
    }

    /// Type checks the given mCRL2 PRES specification.
    pub fn from_untyped_with(mut spec: UntypedPres, encoding: NumberEncoding) -> Result<Self, PresError> {
        // A pure syntactic pass, before anything else needs `spec` — see
        // `resolution::variable_resolution`.
        crate::resolve_pres_variables(&mut spec);

        let data_spec = std::mem::take(&mut spec.data_specification);
        // See `modal_specification.rs`'s equivalent call: a PRES is not (yet) part of
        // the shared, file-based `%import` pipeline, so its embedded data
        // specification gets its own, throwaway `SourceMap`.
        let mut data = DataSpecification::from_untyped_with(data_spec, encoding, &mut SourceMap::new())?;

        let tables = DeclarationTables::build(&mut data, &spec)?;
        let typing = check::check_pres_specification(&mut data, &tables, &spec)?;

        Ok(PresSpecification { spec, data, typing })
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
    pub fn equations(&self) -> &[PresEquation] {
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
    /// `crate::pbes::pbes_specification::DeclarationTables::equation_decl_spans`.
    pub(super) equation_decl_spans: Vec<Span>,
    /// Propositional-variable name -> index into `spec.equations`/`equation_params`.
    pub(super) equations_by_name: HashMap<String, usize>,
}

impl DeclarationTables {
    fn build(data: &mut DataSpecification, spec: &UntypedPres) -> Result<Self, PresError> {
        let mut global_sorts = Vec::with_capacity(spec.global_variables.len());
        let mut seen_globals = HashSet::new();
        for decl in &spec.global_variables {
            if !seen_globals.insert(decl.identifier.as_str()) {
                return Err(PresError::DuplicateGlobalVariable {
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
                    return Err(PresError::DuplicateEquationParameter {
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
                return Err(PresError::DuplicatePropositionalVariable {
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

/// Resolves a sort expression occurring in a `glob`/PRES-equation-parameter declaration, see
/// [`checking::resolve_declared_sort`].
pub(super) fn resolve_declared_sort(
    data: &mut DataSpecification,
    sort: &SortExpression,
) -> Result<ResolvedSortId, PresError> {
    checking::resolve_declared_sort(data, sort, |span| PresError::AnonymousStructInDeclaration { span })
}
