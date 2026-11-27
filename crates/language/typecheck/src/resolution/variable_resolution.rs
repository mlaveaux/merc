use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::IdDecl;
#[cfg(feature = "internal")]
use merc_syntax::PropVarInst;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::VarId;
use merc_syntax::VarIdAllocator;
use merc_syntax::respan;
use merc_utilities::Span;

/// Resolves every free variable reference in a standalone expression's
/// own local binders: every binder `expr` declares is local to `expr` itself,
/// so resolution starts from an empty [Scope], exactly as it would for a fresh
/// `var`-block-less equation.
pub(crate) fn resolve_data_expr_variables(expr: &mut DataExpr) {
    let mut ids = VarIdAllocator::default();
    let mut scope = Scope::default();
    resolve_in_data_expr(expr, &mut scope, &mut ids);
}

/// Resolves every free variable reference in `spec`'s own `var`-block equations.
pub(crate) fn resolve_data_specification_variables(spec: &mut UntypedDataSpecification) {
    let mut ids = VarIdAllocator::default();

    for eqn_spec in &mut spec.equation_declarations {
        let mut scope = Scope::from_declarations(&mut eqn_spec.variables, &mut ids);

        for equation in &mut eqn_spec.equations {
            if let Some(condition) = &mut equation.condition {
                resolve_in_data_expr(condition, &mut scope, &mut ids);
            }

            resolve_in_data_expr(&mut equation.lhs, &mut scope, &mut ids);
            resolve_in_data_expr(&mut equation.rhs, &mut scope, &mut ids);
        }
    }
}

/// A stack of `(name, id)` bindings supporting shadowing lookup: the innermost (most recently
/// pushed) binding for a name wins, and dropping back to an outer scope is a cheap truncate.
/// Shared by [Scope] (`VarId`-keyed data/action binders) and `merc_formula`'s scope of the
/// `StateVarId`-keyed fixpoint variables — the two id namespaces variable resolution tracks.
#[derive(Clone)]
pub struct NameStack<Id>(Vec<(String, Id)>);

impl<Id> Default for NameStack<Id> {
    fn default() -> Self {
        NameStack(Vec::new())
    }
}

impl<Id: Clone> NameStack<Id> {
    /// Pushes one `(name, id)` binding.
    pub fn push(&mut self, name: String, id: Id) {
        self.0.push((name, id));
    }

    /// Drops the `count` most recently pushed bindings, restoring the stack to what it was
    /// before they were pushed.
    pub fn pop(&mut self, count: usize) {
        self.0.truncate(self.0.len() - count);
    }

    /// The innermost binding named `name`, if one is in scope.
    pub fn resolve(&self, name: &str) -> Option<Id> {
        self.0
            .iter()
            .rev()
            .find(|(bound, _)| bound == name)
            .map(|(_, id)| id.clone())
    }
}

/// The binders currently in scope, each paired with its declaration's own [VarId] and span, so two
/// occurrences of the same binder keep comparing equal once rewritten to
/// [`DataExprKind::Resolved`], and each occurrence knows where its binder is declared.
pub type Scope = NameStack<(VarId, Span)>;

impl Scope {
    /// Builds a scope from a binder's own declarations, assigning each a fresh [VarId].
    pub fn from_declarations<'a, Id: 'a, I>(variables: I, ids: &mut VarIdAllocator) -> Self
    where
        I: IntoIterator<Item = &'a mut IdDecl<Id>>,
    {
        let mut scope = Scope::default();
        scope.push_declarations(variables, ids);
        scope
    }

    /// Pushes each declaration in `variables` onto the scope, assigning it a fresh [VarId], and
    /// returns how many were pushed so the caller can [`Scope::pop`] them back off once its
    /// subtree is done. Each binder is declared at its identifier's span.
    pub fn push_declarations<'a, Id: 'a, I>(&mut self, variables: I, ids: &mut VarIdAllocator) -> usize
    where
        I: IntoIterator<Item = &'a mut IdDecl<Id>>,
    {
        let mut pushed = 0;
        for variable in variables {
            variable.var_id =
                Some(self.declare(variable.identifier.node.clone(), variable.identifier.span.clone(), ids));
            pushed += 1;
        }
        pushed
    }

    /// Declares a single binder at `declaration`: allocates it a fresh [VarId] and pushes `name`
    /// onto the scope under that id.
    fn declare(&mut self, name: String, declaration: Span, ids: &mut VarIdAllocator) -> VarId {
        let id = ids.alloc();
        self.push(name, (id, declaration));
        id
    }
}

#[cfg(feature = "internal")]
pub fn resolve_in_prop_var_inst(inst: &mut PropVarInst, scope: &mut Scope, ids: &mut VarIdAllocator) {
    for argument in &mut inst.arguments {
        resolve_in_data_expr(argument, scope, ids);
    }
}

/// Rewrites every `Id(name)` in `expr` found in `scope` into `Resolved(name, binder)`, extending
/// `scope` (and allocating from `ids`) for the data-level binders it descends through (`lambda`, a
/// quantifier, a set/bag comprehension, `whr`).
pub fn resolve_in_data_expr(expr: &mut DataExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        DataExprKind::Id(name) => {
            if let Some((id, declaration)) = scope.resolve(name) {
                expr.node = DataExprKind::Resolved(name.clone(), respan(declaration, id));
            }
        }
        DataExprKind::Resolved(_, _)
        | DataExprKind::Number(_)
        | DataExprKind::Bool(_)
        | DataExprKind::EmptyList
        | DataExprKind::EmptySet
        | DataExprKind::EmptyBag => {}
        DataExprKind::Application { function, arguments } => {
            resolve_in_data_expr(function, scope, ids);
            for argument in arguments {
                resolve_in_data_expr(argument, scope, ids);
            }
        }
        DataExprKind::List(elements) | DataExprKind::Set(elements) => {
            for element in elements {
                resolve_in_data_expr(element, scope, ids);
            }
        }
        DataExprKind::Bag(elements) => {
            for element in elements {
                resolve_in_data_expr(&mut element.expr, scope, ids);
                resolve_in_data_expr(&mut element.multiplicity, scope, ids);
            }
        }
        DataExprKind::SetBagComp { variable, predicate } => {
            let pushed = scope.push_declarations(std::slice::from_mut(variable), ids);
            resolve_in_data_expr(predicate, scope, ids);
            scope.pop(pushed);
        }
        DataExprKind::Lambda { variables, body } | DataExprKind::Quantifier { variables, body, .. } => {
            let pushed = scope.push_declarations(variables, ids);
            resolve_in_data_expr(body, scope, ids);
            scope.pop(pushed);
        }
        DataExprKind::Unary { expr, .. } => resolve_in_data_expr(expr, scope, ids),
        DataExprKind::Binary { lhs, rhs, .. } => {
            resolve_in_data_expr(lhs, scope, ids);
            resolve_in_data_expr(rhs, scope, ids);
        }
        DataExprKind::FunctionUpdate { expr, update } => {
            resolve_in_data_expr(expr, scope, ids);
            resolve_in_data_expr(&mut update.expr, scope, ids);
            resolve_in_data_expr(&mut update.update, scope, ids);
        }
        DataExprKind::Whr { expr, assignments } => {
            // Each assignment's right-hand side is resolved in the *outer* scope — bindings
            // don't see each other, only the body does.
            for assignment in assignments.iter_mut() {
                resolve_in_data_expr(&mut assignment.expr, scope, ids);
            }
            let pushed = assignments.len();
            for assignment in assignments.iter_mut() {
                assignment.id = Some(scope.declare(assignment.identifier.clone(), assignment.span.clone(), ids));
            }
            resolve_in_data_expr(expr, scope, ids);
            scope.pop(pushed);
        }
    }
}
