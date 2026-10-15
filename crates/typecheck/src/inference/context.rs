use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use merc_syntax::ConstructorId;
use merc_syntax::EqnSpecId;
use merc_syntax::EquationId;
use merc_syntax::MapId;
use merc_syntax::SortId;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;

use crate::EquationRole;
use crate::EquationTyping;
use crate::InferenceError;
use crate::ResolvedSortId;
use crate::Signature;
use crate::SortInterner;
use crate::TemplateCheck;
use crate::TemplateId;

/// The context shared by all type-checking queries.
///
/// It owns the [SortInterner] and one [HashMap] memoization table per query.
/// Each semantic fact is a memoized function on this context, so passes pull
/// their dependencies lazily and results are shared.
#[derive(Clone)]
pub(crate) struct TypeCheckContext {
    pub(crate) sorts: SortInterner,

    /// The memoized resolved sort of each sort declaration (an alias's resolved body), keyed by
    /// [SortId]. Populated lazily by `query_sort_of_def`.
    pub(crate) sort_of_def: HashMap<SortId, ResolvedSortId>,
    /// The memoized resolved sort of each constructor declaration, keyed by
    /// [ConstructorId]. Populated lazily by `query_sort_of_constructor`.
    pub(crate) sort_of_constructor: HashMap<ConstructorId, ResolvedSortId>,
    /// The memoized resolved sort of each map declaration, keyed by [MapId].
    /// Populated lazily by `query_sort_of_map`.
    pub(crate) sort_of_map: HashMap<MapId, ResolvedSortId>,
    /// The memoized resolved sort of each equation variable, keyed by its own [VarId] together
    /// with the [EquationRole] that numbered it — see `query_sort_of_equation_var`'s doc comment.
    pub(crate) sort_of_equation_var: HashMap<(EquationRole, VarId), ResolvedSortId>,

    /// The signature of the specification
    pub(crate) signature: Option<Arc<Signature>>,

    /// The memoized results of [`infer_equation_typing`](crate::infer_equation_typing), keyed by the id of the
    /// enclosing equation specification block and the equation's own id
    /// within it.
    pub(crate) equation_typing: HashMap<(EqnSpecId, EquationId), Result<Arc<EquationTyping>, InferenceError>>,
    /// The system-equation counterpart of `equation_typing`.
    pub(crate) system_equation_typing: HashMap<(EqnSpecId, EquationId), Result<Arc<EquationTyping>, InferenceError>>,
    /// The proven typing of each Appendix-B container/function-update
    /// template's own equations, checked once with its type variable(s) held
    /// rigid by `typecheck_template_equations`.
    pub(crate) template_typings: HashMap<TemplateId, TemplateCheck>,
}

impl TypeCheckContext {
    pub(crate) fn new() -> Self {
        TypeCheckContext {
            sorts: SortInterner::new(),
            sort_of_def: HashMap::new(),
            sort_of_constructor: HashMap::new(),
            sort_of_map: HashMap::new(),
            sort_of_equation_var: HashMap::new(),
            signature: None,
            equation_typing: HashMap::new(),
            system_equation_typing: HashMap::new(),
            template_typings: HashMap::new(),
        }
    }
}

impl TypeCheckContext {
    /// The declared name of the sort that [SortId] `def` resolves to — a user
    /// sort or a system-internal one such as `@NatPair` alike, both declared in
    /// `spec.sort_declarations` — or `None` when `def` is out of range.
    pub(crate) fn sort_name<'a>(&'a self, spec: &'a UntypedDataSpecification, def: SortId) -> Option<&'a str> {
        spec.sort_declarations.get(*def).map(|decl| decl.identifier.as_str())
    }

    /// As [`Self::sort_name`], but falls back to a synthesized `@sort_N` placeholder instead of
    /// `None` when `def` is out of range.
    pub(crate) fn sort_display_name<'a>(&'a self, spec: &'a UntypedDataSpecification, def: SortId) -> Cow<'a, str> {
        match self.sort_name(spec, def) {
            Some(name) => Cow::Borrowed(name),
            None => Cow::Owned(format!("@sort_{}", def.value())),
        }
    }
}

impl Default for TypeCheckContext {
    fn default() -> Self {
        TypeCheckContext::new()
    }
}
