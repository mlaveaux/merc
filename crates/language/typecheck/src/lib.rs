#![doc = include_str!("../README.md")]
mod builtins;
#[cfg(feature = "internal")]
mod checking;
mod data_specification;
mod inference;
mod instantiate;
mod ir;
mod number_encoding;
mod resolution;
mod signature;
mod typing_info;

// The internal passes are flattened to the crate root for convenience; their
// exact module is not part of the interface. Only the items below marked `pub`
// are exposed outside the crate.
pub(crate) use builtins::*;
pub(crate) use data_specification::*;
pub(crate) use inference::*;
pub(crate) use instantiate::*;
pub(crate) use ir::*;
pub(crate) use resolution::*;
pub(crate) use signature::*;

pub use data_specification::DataSpecification;
pub use data_specification::TypedDataSpecification;
pub use data_specification::TypedExpression;
pub use inference::InferenceError;
pub use number_encoding::NumberEncoding;
pub use signature::WellTypedError;
pub use typing_info::ResolvedName;
pub use typing_info::TypedNode;
pub use typing_info::TypingInfo;

/// The building blocks for type checking a specification defined in another crate, such as the
/// process specifications of `merc_process`, the state formulas of `merc_formula`, and the PBES
/// and PRES specifications of `merc_pbes` and `merc_pres`: variable resolution, scoped expression
/// checking, and recording typing information. Also the inferred typing that `merc_lowering`
/// reads to lower a checked specification to the aterm form. These are implementation details of the type checker and not a
/// stable interface, hence only available with the `internal` feature.
#[cfg(feature = "internal")]
#[doc(hidden)]
pub mod internal {
    use merc_syntax::EqnSpecId;
    use merc_syntax::EquationId;
    use merc_syntax::Span;

    use crate::DataSpecification;
    use crate::ResolvedName;
    use crate::TypingInfo;

    pub use crate::builtins::is_system_generated_name;
    pub use crate::inference::EquationTyping;
    pub use crate::inference::ExprId;
    pub use crate::inference::ExprTag;
    pub use crate::inference::NameTarget;
    pub use crate::inference::ResolvedSort;
    pub use crate::inference::ResolvedSortId;
    pub use crate::inference::ResolvedSortTag;
    pub use crate::inference::SortInterner;
    pub use crate::inference::TypeCheckContext;
    pub use crate::inference::for_each_expr_child;
    pub use crate::inference::unreachable_not_a_value_sort;
    pub use crate::instantiate::instantiate_used_templates;
    pub use crate::resolution::NameStack;
    pub use crate::resolution::Scope as VariableScope;
    pub use crate::resolution::resolve_in_data_expr;
    pub use crate::resolution::resolve_in_prop_var_inst;
    pub use crate::typing_info::SortReference;
    pub use crate::typing_info::collect_sort_name_references;
    pub use crate::typing_info::declared_span;
    pub use crate::typing_info::push_sort_references;

    /// The resolved `Bool` sort of `data`.
    pub fn bool_sort(data: &DataSpecification) -> ResolvedSortId {
        data.context().sorts.bool_sort()
    }

    /// The resolved `Real` sort of `data`.
    pub fn real_sort(data: &DataSpecification) -> ResolvedSortId {
        data.context().sorts.real_sort()
    }

    /// The query context holding the sorts and equation typings inferred for `data`.
    pub fn context(data: &DataSpecification) -> &TypeCheckContext {
        data.context()
    }

    /// The cached typing of the user equation identified by `key` of `data`.
    pub fn equation_typing(data: &DataSpecification, key: (EqnSpecId, EquationId)) -> &EquationTyping {
        data.equation_typing(key)
    }

    /// The inferred typing of `expr`.
    pub fn expression_typing(expr: &crate::TypedExpression) -> &EquationTyping {
        expr.typing()
    }

    /// Records that the occurrence at `span` resolves to `name`.
    pub fn push_resolved_name(typing: &mut TypingInfo, span: Span, name: ResolvedName) {
        typing.push(span, name);
    }

    /// The least upper bound of `lhs` and `rhs` in the sort lattice of `data`, if there is one.
    pub fn join_sorts(
        data: &mut DataSpecification,
        lhs: ResolvedSortId,
        rhs: ResolvedSortId,
    ) -> Option<ResolvedSortId> {
        let (ctx, _) = data.context_and_specs_mut();
        ctx.sorts.join(lhs, rhs)
    }

    /// Whether a value of sort `from` can be coerced to sort `to`.
    pub fn is_materializable(data: &DataSpecification, from: ResolvedSortId, to: ResolvedSortId) -> bool {
        data.context().sorts.is_materializable(from, to)
    }

    /// Renders the resolved sort `id` of `data`, for use in an error message.
    pub fn display_sort(data: &mut DataSpecification, id: ResolvedSortId) -> String {
        let (ctx, spec) = data.context_and_specs_mut();
        crate::DisplaySortContext::new(ctx, spec, id).to_string()
    }

    /// Shared building blocks for checking expressions in the scope of a declaration.
    pub mod checking {
        pub use crate::checking::ActionTable;
        pub use crate::checking::Scope;
        pub use crate::checking::check_expression_against;
        pub use crate::checking::collect_binder_sorts;
        pub use crate::checking::declare_variables;
        pub use crate::checking::resolve_declared_sort;
        pub use crate::checking::resolve_single_candidate;
    }
}
