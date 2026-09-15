use std::collections::HashMap;
use std::ops::ControlFlow;

use merc_syntax::ActDecl;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::SourceMap;
use merc_syntax::Span;
use merc_syntax::StateFrm;
use merc_syntax::Traverse;
use merc_syntax::UntypedStateFrmSpec;

use crate::DataSpecification;
use crate::NumberEncoding;
use crate::ResolvedSortId;
use crate::TypingInfo;

use super::ModalError;
use super::check;

/// Whether a state formula's `val(...)` occurrences are `Real`- or `Bool`-sorted; see
/// `super::check`'s module doc comment for how this is decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValSort {
    /// Every `val(...)` in the formula is `Real`-sorted, combined via a `*`-multiplier into a
    /// PRES-style quantitative formula.
    Real,
    /// Every `val(...)` in the formula is `Bool`-sorted, a plain mu-calculus atom.
    Bool,
    /// The formula has no state-level `val(...)` at all, so nothing pins the choice down.
    Unknown,
}

/// A type-checked modal state formula: the data specification plus its `act` declarations and the
/// formula itself, all resolved and checked against it. See the module doc comment for what's in
/// and out of scope.
pub struct ModalSpecification {
    /// The original specification, *minus* its data specification.
    spec: UntypedStateFrmSpec,
    data: DataSpecification,
    /// Every checked expression's `TypingInfo`.
    typing: TypingInfo,
    /// Whether the formula's `val(...)` occurrences are `Real`- or `Bool`-sorted; see [`ValSort`].
    val_sort: ValSort,
}

impl ModalSpecification {
    /// Type checks `spec` against a fresh, throwaway [`SourceMap`], using the default number
    /// encoding. See [`Self::from_untyped_with`].
    ///
    /// Prefer [`Self::from_untyped_with`] with a real `sources` (e.g. the one
    /// `UntypedStateFrmSpec::parse_with_imports` built) when `spec` came from a file on disk that
    /// may itself `%import` a process specification — this entry point's own throwaway
    /// `SourceMap` is discarded on return, so any span into `spec`'s system-defined content would
    /// no longer render against anything afterward.
    pub fn from_untyped(spec: UntypedStateFrmSpec) -> Result<Self, ModalError> {
        Self::from_untyped_with(spec, NumberEncoding::default(), &mut SourceMap::new())
    }

    /// Type checks `spec`: its data specification first (exactly as
    /// [`DataSpecification::from_untyped_with`] does), then its `act` declarations' argument
    /// sorts, and finally the formula itself against them.
    ///
    /// `sources` accumulates the system-defined ("Appendix B") content this generates, the same
    /// way [`DataSpecification::from_untyped_with`]'s own `sources` parameter does — pass the
    /// `SourceMap` `spec` was parsed (and, if applicable, `%import`-resolved) against so every
    /// span, whether from `spec`'s own text, something it imports, or Appendix B, renders
    /// correctly against one shared offset space; pass a fresh one if nothing else needs to share
    /// it.
    pub fn from_untyped_with(
        mut spec: UntypedStateFrmSpec,
        encoding: NumberEncoding,
        sources: &mut SourceMap,
    ) -> Result<Self, ModalError> {
        // A pure syntactic pass, before anything else needs `spec` — see
        // `resolution::variable_resolution`.
        crate::resolve_modal_variables(&mut spec);

        let data_spec = std::mem::take(&mut spec.data_specification);
        let mut data = DataSpecification::from_untyped_with(data_spec, encoding, sources)?;

        let tables = DeclarationTables::build(&mut data, &spec)?;
        let (typing, val_sort) = check::check_modal_specification(&mut data, &tables, &spec)?;

        Ok(ModalSpecification {
            spec,
            data,
            typing,
            val_sort,
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

    /// Every checked expression's typing across the *whole* specification.
    pub fn typing_info(&mut self) -> TypingInfo {
        let mut info = self.data.typing_info();
        info.merge(self.typing.clone());
        info
    }

    /// Whether the formula's `val(...)` occurrences are `Real`- or `Bool`-sorted; see [`ValSort`].
    pub fn val_sort(&self) -> ValSort {
        self.val_sort
    }
}

/// The resolved `act` declaration table, built once by [`Self::build`] and used by
/// [`super::check`]'s scoped walk to resolve every action instance it reaches.
pub(super) struct DeclarationTables {
    /// Resolved argument-sort domain of each action declaration, parallel to
    /// `spec.action_declarations`.
    pub(super) action_domains: Vec<Vec<ResolvedSortId>>,
    /// `spec.action_declarations[i].identifier.span`, parallel to `action_domains`.
    pub(super) action_decl_spans: Vec<Span>,
    /// name -> indices into `spec.action_declarations`/`action_domains` declaring it.
    pub(super) actions_by_name: HashMap<String, Vec<usize>>,
}

impl DeclarationTables {
    fn build(data: &mut DataSpecification, spec: &UntypedStateFrmSpec) -> Result<Self, ModalError> {
        let mut action_domains = Vec::with_capacity(spec.action_declarations.len());
        let mut action_decl_spans = Vec::with_capacity(spec.action_declarations.len());
        let mut actions_by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, decl) in spec.action_declarations.iter().enumerate() {
            let domain = decl
                .args
                .iter()
                .map(|sort| resolve_declared_sort(data, sort))
                .collect::<Result<Vec<_>, _>>()?;
            actions_by_name
                .entry(decl.identifier.node.clone())
                .or_default()
                .push(index);
            action_domains.push(domain);
            action_decl_spans.push(decl.identifier.span.clone());
        }

        Ok(DeclarationTables {
            action_domains,
            action_decl_spans,
            actions_by_name,
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
    if let Some(span) = find_anonymous_struct(sort) {
        return Err(ModalError::AnonymousStructInDeclaration { span });
    }
    Ok(data.resolve_declared_sort(sort)?)
}

/// The span of the first anonymous `struct` anywhere within `sort`, if any.
fn find_anonymous_struct(sort: &SortExpression) -> Option<Span> {
    sort.visit(|expr| match &expr.node {
        SortExpressionKind::Struct { .. } => ControlFlow::Break(expr.span.clone()),
        _ => ControlFlow::Continue(()),
    })
}
