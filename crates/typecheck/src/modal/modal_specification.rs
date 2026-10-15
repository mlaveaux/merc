use std::collections::HashMap;
use std::collections::HashSet;

use merc_syntax::ActDecl;
use merc_syntax::SortExpression;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::StateFrm;
use merc_syntax::StateVarId;
use merc_syntax::UntypedStateFrmSpec;

use crate::DataSpecification;
use crate::NumberEncoding;
use crate::ResolvedSortId;
use crate::TypingInfo;
use crate::checking;
use crate::resolve_modal_variables;

use super::ModalError;
use super::check;
use super::modal_equation_system::ModalEquationSystem;

/// Whether a state formula's `val(...)` occurrences are `Real`- or `Bool`-sorted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormulaType {
    /// Quantitative: every state-level `val(...)` is `Real`-sorted, or `Bool`
    /// sorts mapped to `Real`.
    Real,
    /// Plain mu-calculus: every state-level `val(...)` is `Bool`-sorted.
    Bool,
}

/// A type-checked modal (mu-calculus) state formula: the data specification plus its `act`
/// declarations and the formula itself, all resolved and checked against it — every action
/// instance, fixpoint (`mu`/`nu`) variable, and `forall`/`exists`/`inf`/`sup`/`sum` binder
/// included.
pub struct ModalSpecification {
    /// The original specification, *minus* its data specification.
    spec: UntypedStateFrmSpec,
    data: DataSpecification,
    /// Every checked expression's `TypingInfo`.
    typing: TypingInfo,
    /// The type every state-level `val(...)` occurrence was checked against.
    formula_type: FormulaType,
    /// The formula's own checked, flattened fixpoint-equation view.
    equation_system: ModalEquationSystem,
}

impl ModalSpecification {
    /// Type checks `spec` against a fresh, throwaway [`SourceMap`], using the default number
    /// encoding.
    pub fn from_untyped(spec: UntypedStateFrmSpec, formula_type: FormulaType) -> Result<Self, ModalError> {
        Self::from_untyped_with(spec, formula_type, NumberEncoding::default(), &mut SourceMap::new())
    }

    /// Type checks `spec` against the given number encoding and source map, requiring val
    /// occurrences to conform to `formula_type`.
    ///
    /// `sources` also accumulates the system-defined ("Appendix B") content this generates.
    pub fn from_untyped_with(
        mut spec: UntypedStateFrmSpec,
        formula_type: FormulaType,
        encoding: NumberEncoding,
        sources: &mut SourceMap,
    ) -> Result<Self, ModalError> {
        // A pure syntactic pass, before anything else needs `spec`.
        let mut state_var_ids = resolve_modal_variables(&mut spec.formula);

        let data_spec = std::mem::take(&mut spec.data_specification);
        let mut data = DataSpecification::from_untyped_with(data_spec, encoding, sources)?;

        // Flattens the formula's nested `mu`/`nu`s into a ranked list of equations up front, the
        // same way a PBES already is one — see `DeclarationTables::build` and
        // `check::check_modal_specification`'s per-equation loop.
        let equation_system = ModalEquationSystem::new(&spec.formula, &mut state_var_ids);

        let tables = DeclarationTables::build(
            &mut data,
            &spec.action_declarations,
            equation_system,
            resolve_declared_sort,
        )?;
        let typing = check::check_modal_specification(&mut data, &tables, &spec, formula_type)?;

        Ok(ModalSpecification {
            spec,
            data,
            typing,
            formula_type,
            equation_system: tables.equation_system,
        })
    }

    /// The checked data specification.
    pub fn data_specification(&self) -> &DataSpecification {
        &self.data
    }

    /// Consumes `self`, returning the checked data specification.
    pub fn into_data_specification(self) -> DataSpecification {
        self.data
    }

    /// The `act` declarations, in scope in every modality's action/regular formula.
    pub fn action_declarations(&self) -> &[ActDecl] {
        &self.spec.action_declarations
    }

    /// The state formula itself.
    pub fn formula(&self) -> &StateFrm {
        &self.spec.formula
    }

    /// The formula's own checked, flattened fixpoint-equation view — the intended input for
    /// anything that wants to *use* the type-checked formula (e.g. `merc_vpg::translate`), rather
    /// than [`Self::formula`]'s raw nested tree.
    pub fn equation_system(&self) -> &ModalEquationSystem {
        &self.equation_system
    }

    /// Every checked expression's typing across the *whole* specification.
    pub fn typing_info(&self) -> TypingInfo {
        let mut info = self.data.typing_info();
        info.merge(self.typing.clone());
        info
    }

    /// The sort of this modal formula.
    pub fn val_sort(&self) -> FormulaType {
        self.formula_type
    }
}

/// How a state formula's action references are checked: decided once for the whole spec, from
/// whether it declares any `act` at all, rather than inferred per call site from an incidentally
/// empty [`checking::ActionTable`].
pub(super) enum ActionChecking {
    /// At least one `act` was declared: every reference must resolve against the table.
    Declared(checking::ActionTable),
    /// No `act` at all: every action is a "simple action" — a plain LTS label, matched
    /// structurally rather than typed.
    Simple,
}

/// The resolved `act` declaration table, plus the formula's own flattened fixpoint-equation view
/// and each equation's resolved parameter sorts — built once by [`Self::build`] and used by
/// [`super::check`]'s per-equation walk to resolve every action instance and fixpoint-variable
/// reference it reaches. Mirrors `crate::pbes::pbes_specification::DeclarationTables`, with a
/// [`StateVarId`]-keyed lookup in place of PBES's name-keyed one: a modal fixpoint variable's own
/// name can be shadowed (`mu X. (nu X. ...)`, legal mCRL2 syntax), unlike a PBES equation's.
pub(super) struct DeclarationTables {
    pub(super) actions: ActionChecking,
    /// The formula's own equations, in the shape [`super::check`]'s per-equation loop walks.
    pub(super) equation_system: ModalEquationSystem,
    /// Resolved `(name, sort)` parameters of each equation, parallel to `equation_system`'s own
    /// equations.
    pub(super) equation_params: Vec<Vec<(String, ResolvedSortId)>>,
    /// Each equation's own variable declaration span, parallel to `equation_params`.
    pub(super) equation_decl_spans: Vec<Span>,
    /// A fixpoint variable's own [`StateVarId`] -> index into `equation_params`/
    /// `equation_decl_spans`/`equation_system`'s own equations.
    pub(super) equations_by_id: HashMap<StateVarId, usize>,
}

impl DeclarationTables {
    fn build<F>(
        data: &mut DataSpecification,
        action_declarations: &[ActDecl],
        equation_system: ModalEquationSystem,
        mut resolve_declared_sort: F,
    ) -> Result<Self, ModalError>
    where
        F: FnMut(&mut DataSpecification, &SortExpression) -> Result<ResolvedSortId, ModalError>,
    {
        let actions = if action_declarations.is_empty() {
            ActionChecking::Simple
        } else {
            ActionChecking::Declared(checking::ActionTable::build(
                data,
                action_declarations,
                &mut resolve_declared_sort,
            )?)
        };

        let mut equation_params = Vec::with_capacity(equation_system.len());
        let mut equation_decl_spans = Vec::with_capacity(equation_system.len());
        let mut equations_by_id = HashMap::with_capacity(equation_system.len());
        for index in 0..equation_system.len() {
            let variable = equation_system.equation(index).variable();

            let mut params = Vec::with_capacity(variable.arguments.len());
            let mut seen = HashSet::new();
            for argument in &variable.arguments {
                if !seen.insert(argument.decl.identifier.as_str()) {
                    return Err(ModalError::DuplicateFixedPointParameter {
                        variable: variable.identifier.node.clone(),
                        name: argument.decl.identifier.node.clone(),
                        span: argument.decl.identifier.span.clone(),
                    });
                }
                let sort = resolve_declared_sort(data, &argument.decl.sort)?;
                params.push((argument.decl.identifier.node.clone(), sort));
            }

            // `ModalEquationSystem` itself already guarantees every equation's own `StateVarId`
            // is unique (it's built from a resolved formula), so this can't collide.
            let id = variable.id.expect("ModalEquationSystem requires a resolved formula");
            equations_by_id.insert(id, equation_params.len());
            equation_params.push(params);
            equation_decl_spans.push(variable.span.clone());
        }

        Ok(DeclarationTables {
            actions,
            equation_system,
            equation_params,
            equation_decl_spans,
            equations_by_id,
        })
    }
}

/// Resolves a sort expression occurring in an `act`/fixpoint-variable-parameter/binder declaration:
/// rejects an anonymous `struct` (never legal here), then defers to
/// [`DataSpecification::resolve_declared_sort`] for the rest.
pub(super) fn resolve_declared_sort(
    data: &mut DataSpecification,
    sort: &SortExpression,
) -> Result<ResolvedSortId, ModalError> {
    checking::resolve_declared_sort(data, sort, |span| ModalError::AnonymousStructInDeclaration { span })
}
