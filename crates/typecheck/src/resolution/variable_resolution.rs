use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_syntax::ActFrm;
use merc_syntax::ActFrmKind;
use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::IdDecl;
use merc_syntax::PbesExpr;
use merc_syntax::PbesExprKind;
use merc_syntax::PresExpr;
use merc_syntax::PresExprKind;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::PropVarInst;
use merc_syntax::RegFrm;
use merc_syntax::RegFrmKind;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmKind;
use merc_syntax::StateVarId;
use merc_syntax::StateVarIdAllocator;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::UntypedPbes;
use merc_syntax::UntypedPres;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::VarId;
use merc_syntax::VarIdAllocator;
use merc_utilities::Step;

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

/// Resolves every free variable reference in `spec`'s `proc` bodies and `init`.
pub(crate) fn resolve_process_variables(spec: &mut UntypedProcessSpecification) {
    let mut ids = VarIdAllocator::default();
    let globals = Scope::from_declarations(&mut spec.global_variables, &mut ids);

    for proc_decl in &mut spec.process_declarations {
        // A process's own parameters shadow a global variable of the same name.
        let mut scope = globals.clone();
        scope.push_declarations(&mut proc_decl.params, &mut ids);
        resolve_in_process_expr(&mut proc_decl.body, &mut scope, &mut ids);
    }

    if let Some(init) = &mut spec.init {
        // `init` sits outside every process's own parameter scope — only globals apply.
        let mut scope = globals.clone();
        resolve_in_process_expr(init, &mut scope, &mut ids);
    }
}

/// Resolves every free variable reference in `pbes`'s equation bodies and `init`.
pub(crate) fn resolve_pbes_variables(pbes: &mut UntypedPbes) {
    let mut ids = VarIdAllocator::default();
    let globals = Scope::from_declarations(&mut pbes.global_variables, &mut ids);

    for equation in &mut pbes.equations {
        let mut scope = globals.clone();
        scope.push_declarations(&mut equation.variable.parameters, &mut ids);
        resolve_in_pbes_expr(&mut equation.formula, &mut scope, &mut ids);
    }

    // `init` sits outside every equation's own parameter scope — only globals apply.
    let mut scope = globals.clone();
    resolve_in_prop_var_inst(&mut pbes.init, &mut scope, &mut ids);
}

/// Resolves every free variable reference in `pres`'s equation bodies and `init`.
pub(crate) fn resolve_pres_variables(pres: &mut UntypedPres) {
    let mut ids = VarIdAllocator::default();
    let globals = Scope::from_declarations(&mut pres.global_variables, &mut ids);

    for equation in &mut pres.equations {
        let mut scope = globals.clone();
        scope.push_declarations(&mut equation.variable.parameters, &mut ids);
        resolve_in_pres_expr(&mut equation.formula, &mut scope, &mut ids);
    }

    // `init` sits outside every equation's own parameter scope — only globals apply.
    let mut scope = globals.clone();
    resolve_in_prop_var_inst(&mut pres.init, &mut scope, &mut ids);
}

/// Resolves every free variable reference in `spec`'s state formula.
///
/// This pass only decides *which* enclosing binder a name refers to; a fixpoint variable's own
/// *parameter sorts* still aren't known here.
pub(crate) fn resolve_modal_variables(spec: &mut UntypedStateFrmSpec) {
    let mut ids = VarIdAllocator::default();
    let mut state_var_ids = StateVarIdAllocator::default();
    resolve_in_state_frm(&mut spec.formula, &mut ids, &mut state_var_ids);
}

/// What a matching `enter` call pushed onto `StateFrmScope::scope`/`state_vars` for one node,
/// popped by `exit` once that node's whole subtree is resolved. `exit` never sees the node itself
/// again (see [`Traverse::apply_subtree_scoped`]'s own doc comment for why a mutating scoped walk's
/// `exit` cannot be handed one), so `enter` records here, once per node, what -- if anything --
/// needs undoing; every arm below pushes exactly one, keeping `enter`/`exit` calls paired 1:1.
enum Undo {
    /// This node pushed nothing.
    None,
    /// A `Quantifier`/`Bound`'s own bound variables: pop this many entries off `scope`.
    Scope(usize),
    /// A `FixedPoint`'s own parameters and its own variable name: pop this many entries off
    /// `scope`, then one off `state_vars` (in that order, mirroring the push order below).
    ScopeAndStateVar(usize),
}

/// The scoped mutable state threaded through [`resolve_in_state_frm`]'s [`Traverse::apply_scoped`]
/// walk: `enter`/`exit` both need `scope`/`state_vars` (so, per that method's contract, both must
/// go through this shared `state`, not plain closure capture), while `ids`/`state_var_ids` are used
/// by `enter` alone and stay ordinary closure captures.
#[derive(Default)]
struct StateFrmScope {
    scope: Scope,
    state_vars: FixpointScope,
    undo: Vec<Undo>,
}

/// Resolves every free variable reference in `formula`, rewriting each `Id` naming an enclosing
/// `mu`/`nu` into `Resolved` in place.
///
/// A single [`Traverse::apply_scoped`] walk replaces the previous hand-written recursive-descent
/// version: every arm resolves whatever isn't itself a same-type `StateFrm` child (a `DataExpr`
/// time/constant/argument, or -- for `FixedPoint` -- a parameter's own initial value) and pushes
/// exactly one [`Undo`], leaving the traversal's own descent to reach every operand in the same
/// left-to-right, pre-order sequence the original recursive calls did. `scope`/`state_vars` are
/// local to this walk, not threaded in from [`resolve_modal_variables`], the same way
/// `modal::check::check_state_formula`'s `state_vars` already is.
///
/// One order does shift, harmlessly: `DataValExprLeftMult`'s constant is resolved before its
/// `StateFrm` operand in both the original code and here, but `DataValExprRightMult`'s constant
/// used to be resolved *after* its operand (`resolve_in_state_frm(expr, ...); resolve_in_data_expr
/// (constant, ...)`) -- a pre-order walk's `enter` always runs before its own descent, so both are
/// now resolved constant-first. Neither order can change *which* binder a name resolves to (only a
/// nested binder inside `constant`/`expr` could allocate its own id in a different order), and
/// nothing here depends on the specific numeric value assigned to any id, only on occurrences of
/// the same binder continuing to share one -- unaffected either way.
fn resolve_in_state_frm(formula: &mut StateFrm, ids: &mut VarIdAllocator, state_var_ids: &mut StateVarIdAllocator) {
    let mut state = StateFrmScope::default();
    let _: Result<Option<Infallible>, Infallible> = formula.apply_scoped(
        (),
        &mut state,
        |formula, (), state| {
            match &mut formula.node {
                StateFrmKind::True | StateFrmKind::False => state.undo.push(Undo::None),

                StateFrmKind::Delay(time) | StateFrmKind::Yaled(time) => {
                    if let Some(time) = time {
                        resolve_in_data_expr(time, &mut state.scope, ids);
                    }
                    state.undo.push(Undo::None);
                }

                StateFrmKind::Id(name, arguments) => {
                    for argument in arguments.iter_mut() {
                        resolve_in_data_expr(argument, &mut state.scope, ids);
                    }
                    if let Some(declaration) = state.state_vars.resolve(name) {
                        formula.node = StateFrmKind::Resolved(name.clone(), std::mem::take(arguments), declaration);
                    }
                    state.undo.push(Undo::None);
                }

                // Already resolved: this pass never runs twice on the same tree.
                StateFrmKind::Resolved(_, arguments, _) => {
                    for argument in arguments.iter_mut() {
                        resolve_in_data_expr(argument, &mut state.scope, ids);
                    }
                    state.undo.push(Undo::None);
                }

                StateFrmKind::DataValExpr(data_expr) => {
                    resolve_in_data_expr(data_expr, &mut state.scope, ids);
                    state.undo.push(Undo::None);
                }

                StateFrmKind::DataValExprLeftMult(constant, _) | StateFrmKind::DataValExprRightMult(_, constant) => {
                    resolve_in_data_expr(constant, &mut state.scope, ids);
                    state.undo.push(Undo::None);
                }

                StateFrmKind::Modality { formula: reg, .. } => {
                    resolve_in_reg_frm(reg, &mut state.scope, ids);
                    state.undo.push(Undo::None);
                }

                StateFrmKind::Unary { .. } | StateFrmKind::Binary { .. } => state.undo.push(Undo::None),

                StateFrmKind::Quantifier { variables, .. } | StateFrmKind::Bound { variables, .. } => {
                    let pushed = state.scope.push_declarations(variables, ids);
                    state.undo.push(Undo::Scope(pushed));
                }

                StateFrmKind::FixedPoint { variable, .. } => {
                    // Each parameter's own initial value is a free read of the *outer* scope —
                    // the parameter it initializes (and any sibling parameter) isn't bound yet,
                    // mirroring `resolve_in_process_expr`'s treatment of an instantiation's
                    // assignment value.
                    for argument in &mut variable.arguments {
                        resolve_in_data_expr(&mut argument.expr, &mut state.scope, ids);
                    }
                    let pushed = variable.arguments.len();
                    for argument in &mut variable.arguments {
                        argument.id = Some(state.scope.declare(argument.identifier.node.clone(), ids));
                    }
                    // The fixpoint variable's own name is in scope for its body only (it may
                    // itself shadow an outer variable of the same name, `mu X. nu X. ...`).
                    let state_var_id = state_var_ids.alloc();
                    variable.id = Some(state_var_id);
                    state.state_vars.push(variable.identifier.clone(), state_var_id);
                    state.undo.push(Undo::ScopeAndStateVar(pushed));
                }
            }
            Ok(ControlFlow::Continue(Step::Into(())))
        },
        |(), state| match state.undo.pop().expect("enter pushes exactly one Undo per exit") {
            Undo::None => {}
            Undo::Scope(pushed) => state.scope.pop(pushed),
            Undo::ScopeAndStateVar(pushed) => {
                state.state_vars.pop(1);
                state.scope.pop(pushed);
            }
        },
    );
}

/// `RegFrm` itself carries no binder — only crossing through one to reach an `ActFrm` does — and
/// its own nesting (`.`/`+`/`*` chains within a single modality) is not the pathologically deep,
/// syntactically-driven recursion `resolve_in_state_frm`'s own migration closes off; left as a
/// plain recursive dispatch, the same way `modal::check::check_reg_formula` keeps `RegFrm`'s own
/// dispatch a thin nested call rather than a `Traverse` walk in its own right.
fn resolve_in_reg_frm(formula: &mut RegFrm, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut formula.node {
        RegFrmKind::Action(action) => resolve_in_act_frm(action, scope, ids),
        RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => resolve_in_reg_frm(inner, scope, ids),
        RegFrmKind::Sequence { lhs, rhs } | RegFrmKind::Choice { lhs, rhs } => {
            resolve_in_reg_frm(lhs, scope, ids);
            resolve_in_reg_frm(rhs, scope, ids);
        }
    }
}

/// The scoped state for [`resolve_in_act_frm`]'s own [`Traverse::apply_scoped`] walk: `scope` is
/// borrowed from the enclosing [`resolve_in_state_frm`] walk (an `ActFrm`'s own `Quantifier` binder
/// shares the same namespace an enclosing `StateFrm` binder does), while `undo` is local to this
/// walk, the only one of the two `exit` needs to unwind.
struct ActFrmScope<'a> {
    scope: &'a mut Scope,
    /// One entry per node entered, matching [`Undo`]'s two relevant cases here: `None` (pushed
    /// nothing) or `Some(pushed)` (a `Quantifier`'s own bound variables to pop).
    undo: Vec<Option<usize>>,
}

/// Resolves every free variable reference in `formula`, sharing `scope`/`ids` with whichever
/// `resolve_in_state_frm`/`resolve_in_reg_frm` call reached this `ActFrm` through a `Modality`.
///
/// A single [`Traverse::apply_scoped`] walk replaces the previous hand-written recursive-descent
/// version, the same way [`resolve_in_state_frm`] does above; only `Quantifier` pushes anything, so
/// `ActFrmScope::undo` only ever needs `Option<usize>`, not the fuller [`Undo`] enum.
fn resolve_in_act_frm(formula: &mut ActFrm, scope: &mut Scope, ids: &mut VarIdAllocator) {
    let mut state = ActFrmScope {
        scope,
        undo: Vec::new(),
    };
    let _: Result<Option<Infallible>, Infallible> = formula.apply_scoped(
        (),
        &mut state,
        |formula, (), state| {
            match &mut formula.node {
                ActFrmKind::True | ActFrmKind::False | ActFrmKind::Negation(_) | ActFrmKind::Binary { .. } => {
                    state.undo.push(None);
                }

                ActFrmKind::MultAct(multi_action) => {
                    for action in &mut multi_action.actions {
                        for argument in &mut action.args {
                            resolve_in_data_expr(argument, state.scope, ids);
                        }
                    }
                    state.undo.push(None);
                }

                ActFrmKind::DataExprVal(data_expr) => {
                    resolve_in_data_expr(data_expr, state.scope, ids);
                    state.undo.push(None);
                }

                ActFrmKind::Quantifier { variables, .. } => {
                    let pushed = state.scope.push_declarations(variables, ids);
                    state.undo.push(Some(pushed));
                }

                ActFrmKind::At { operand, .. } => {
                    resolve_in_data_expr(operand, state.scope, ids);
                    state.undo.push(None);
                }
            }
            Ok(ControlFlow::Continue(Step::Into(())))
        },
        |(), state| {
            if let Some(pushed) = state.undo.pop().expect("enter pushes exactly one Undo per exit") {
                state.scope.pop(pushed);
            }
        },
    );
}

/// A stack of `(name, id)` bindings supporting shadowing lookup: the innermost (most recently
/// pushed) binding for a name wins, and dropping back to an outer scope is a cheap truncate.
/// Shared by [Scope] (`VarId`-keyed data/action binders) and [FixpointScope] (`StateVarId`-keyed
/// fixpoint variables) — the two id namespaces variable resolution tracks.
#[derive(Clone, Default)]
struct NameStack<Id>(Vec<(String, Id)>);

impl<Id: Copy> NameStack<Id> {
    /// Pushes one `(name, id)` binding.
    fn push(&mut self, name: String, id: Id) {
        self.0.push((name, id));
    }

    /// Drops the `count` most recently pushed bindings, restoring the stack to what it was
    /// before they were pushed.
    fn pop(&mut self, count: usize) {
        self.0.truncate(self.0.len() - count);
    }

    /// The innermost binding named `name`, if one is in scope.
    fn resolve(&self, name: &str) -> Option<Id> {
        self.0.iter().rev().find(|(bound, _)| bound == name).map(|&(_, id)| id)
    }
}

/// The binders currently in scope, each paired with its declaration's own [VarId] so two
/// occurrences of the same binder keep comparing equal once rewritten to
/// [`DataExprKind::Resolved`].
type Scope = NameStack<VarId>;

impl Scope {
    /// Builds a scope from a binder's own declarations, assigning each a fresh [VarId].
    fn from_declarations<Id>(variables: &mut [IdDecl<Id>], ids: &mut VarIdAllocator) -> Self {
        let mut scope = Scope::default();
        scope.push_declarations(variables, ids);
        scope
    }

    /// Pushes each declaration in `variables` onto the scope, assigning it a fresh [VarId], and
    /// returns how many were pushed so the caller can [`Scope::pop`] them back off once its
    /// subtree is done.
    ///
    /// Each declaration's own identifier span stays on the AST node itself (`variable.identifier`)
    /// rather than being recorded here: a later, on-demand walk over the resolved tree (see
    /// `crate::typing_info::VariableSpans`) recovers it straight from the declaration when a
    /// `TypingInfo` query actually needs it, so resolution itself doesn't need to track it.
    fn push_declarations<Id>(&mut self, variables: &mut [IdDecl<Id>], ids: &mut VarIdAllocator) -> usize {
        for variable in variables.iter_mut() {
            variable.var_id = Some(self.declare(variable.identifier.node.clone(), ids));
        }
        variables.len()
    }

    /// Declares a single binder: allocates it a fresh [VarId] and pushes `name` onto the scope
    /// under that id.
    fn declare(&mut self, name: String, ids: &mut VarIdAllocator) -> VarId {
        let var_id = ids.alloc();
        self.push(name, var_id);
        var_id
    }
}

/// The fixpoint-variable names currently in scope, in the second, [`StateVarId`]-keyed namespace.
type FixpointScope = NameStack<StateVarId>;

fn resolve_in_process_expr(expr: &mut ProcessExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        ProcessExprKind::Delta | ProcessExprKind::Tau => {}
        ProcessExprKind::Action(_, args) => {
            for arg in args {
                resolve_in_data_expr(arg, scope, ids);
            }
        }
        ProcessExprKind::Id(_, assignments) => {
            // Only the assignment's *value* is a free variable read.
            for assignment in assignments {
                resolve_in_data_expr(&mut assignment.expr, scope, ids);
            }
        }
        ProcessExprKind::Sum { variables, operand } => {
            let pushed = scope.push_declarations(variables, ids);
            resolve_in_process_expr(operand, scope, ids);
            scope.pop(pushed);
        }
        ProcessExprKind::Dist {
            variables,
            expr: weight,
            operand,
        } => {
            let pushed = scope.push_declarations(variables, ids);
            // `dist`'s weight is resolved with its own bound variables already in scope.
            resolve_in_data_expr(weight, scope, ids);
            resolve_in_process_expr(operand, scope, ids);
            scope.pop(pushed);
        }
        ProcessExprKind::Binary { lhs, rhs, .. } => {
            resolve_in_process_expr(lhs, scope, ids);
            resolve_in_process_expr(rhs, scope, ids);
        }
        ProcessExprKind::Hide { operand, .. }
        | ProcessExprKind::Rename { operand, .. }
        | ProcessExprKind::Allow { operand, .. }
        | ProcessExprKind::Block { operand, .. }
        | ProcessExprKind::Comm { operand, .. } => resolve_in_process_expr(operand, scope, ids),
        ProcessExprKind::Condition { condition, then, else_ } => {
            resolve_in_data_expr(condition, scope, ids);
            resolve_in_process_expr(then, scope, ids);
            if let Some(else_) = else_ {
                resolve_in_process_expr(else_, scope, ids);
            }
        }
        ProcessExprKind::At { expr, operand } => {
            resolve_in_process_expr(expr, scope, ids);
            resolve_in_data_expr(operand, scope, ids);
        }
    }
}

fn resolve_in_pbes_expr(expr: &mut PbesExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        PbesExprKind::True | PbesExprKind::False => {}
        PbesExprKind::DataValExpr(data_expr) => resolve_in_data_expr(data_expr, scope, ids),
        PbesExprKind::PropVarInst(inst) => resolve_in_prop_var_inst(inst, scope, ids),
        PbesExprKind::Negation(inner) => resolve_in_pbes_expr(inner, scope, ids),
        PbesExprKind::Binary { lhs, rhs, .. } => {
            resolve_in_pbes_expr(lhs, scope, ids);
            resolve_in_pbes_expr(rhs, scope, ids);
        }
        PbesExprKind::Quantifier { variables, body, .. } => {
            let pushed = scope.push_declarations(variables, ids);
            resolve_in_pbes_expr(body, scope, ids);
            scope.pop(pushed);
        }
    }
}

fn resolve_in_pres_expr(expr: &mut PresExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        PresExprKind::True | PresExprKind::False => {}
        PresExprKind::DataValExpr(data_expr) => resolve_in_data_expr(data_expr, scope, ids),
        PresExprKind::PropVarInst(inst) => resolve_in_prop_var_inst(inst, scope, ids),
        PresExprKind::Negation(inner) => resolve_in_pres_expr(inner, scope, ids),
        PresExprKind::Binary { lhs, rhs, .. } => {
            resolve_in_pres_expr(lhs, scope, ids);
            resolve_in_pres_expr(rhs, scope, ids);
        }
        PresExprKind::Equal { body, .. } => resolve_in_pres_expr(body, scope, ids),
        PresExprKind::Condition { lhs, then, else_, .. } => {
            resolve_in_pres_expr(lhs, scope, ids);
            resolve_in_pres_expr(then, scope, ids);
            resolve_in_pres_expr(else_, scope, ids);
        }
        PresExprKind::RightConstantMultiply { expr, constant }
        | PresExprKind::LeftConstantMultiply { expr, constant } => {
            resolve_in_data_expr(constant, scope, ids);
            resolve_in_pres_expr(expr, scope, ids);
        }
        PresExprKind::Bound { variables, expr, .. } => {
            let pushed = scope.push_declarations(variables, ids);
            resolve_in_pres_expr(expr, scope, ids);
            scope.pop(pushed);
        }
    }
}

fn resolve_in_prop_var_inst(inst: &mut PropVarInst, scope: &mut Scope, ids: &mut VarIdAllocator) {
    for argument in &mut inst.arguments {
        resolve_in_data_expr(argument, scope, ids);
    }
}

/// Rewrites every `Id(name)` in `expr` found in `scope` into `Resolved(name, VarId)`, extending
/// `scope` (and allocating from `ids`) for the data-level binders it descends through (`lambda`, a
/// quantifier, a set/bag comprehension, `whr`).
fn resolve_in_data_expr(expr: &mut DataExpr, scope: &mut Scope, ids: &mut VarIdAllocator) {
    match &mut expr.node {
        DataExprKind::Id(name) => {
            if let Some(declaration) = scope.resolve(name) {
                expr.node = DataExprKind::Resolved(name.clone(), declaration);
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
                assignment.id = Some(scope.declare(assignment.identifier.clone(), ids));
            }
            resolve_in_data_expr(expr, scope, ids);
            scope.pop(pushed);
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::ActFrmKind;
    use merc_syntax::DataExprKind;
    use merc_syntax::PbesExprKind;
    use merc_syntax::PresExprKind;
    use merc_syntax::ProcessExprKind;
    use merc_syntax::RegFrmKind;
    use merc_syntax::StateFrmKind;
    use merc_syntax::UntypedDataSpecification;
    use merc_syntax::UntypedPbes;
    use merc_syntax::UntypedPres;
    use merc_syntax::UntypedProcessSpecification;
    use merc_syntax::UntypedStateFrmSpec;

    use super::resolve_data_specification_variables;
    use super::resolve_modal_variables;
    use super::resolve_pbes_variables;
    use super::resolve_pres_variables;
    use super::resolve_process_variables;

    #[test]
    fn test_action_argument_resolves_to_process_parameter() {
        let text = "act a: Nat; proc P(n: Nat) = a(n); init P(1);";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let declared = spec.process_declarations[0].params[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let ProcessExprKind::Action(_, args) = &spec.process_declarations[0].body.node else {
            panic!("expected an Action body");
        };
        assert!(matches!(
            &args[0].node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_sum_bound_variable_resolves_to_its_own_binder() {
        let text = "act a: Nat; proc P = sum x: Nat . a(x); init P;";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let ProcessExprKind::Sum { variables, operand } = &spec.process_declarations[0].body.node else {
            panic!("expected a Sum body");
        };
        let declared = variables[0].var_id.expect("the binder was assigned a VarId");
        let ProcessExprKind::Action(_, args) = &operand.node else {
            panic!("expected an Action operand");
        };
        assert!(matches!(
            &args[0].node,
            DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == declared
        ));
    }

    #[test]
    fn test_dist_weight_sees_its_own_bound_variable() {
        let text = "act a; proc P = dist x: Pos[1/x] . a; init P;";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let ProcessExprKind::Dist {
            variables,
            expr: weight,
            ..
        } = &spec.process_declarations[0].body.node
        else {
            panic!("expected a Dist body");
        };
        let declared = variables[0].var_id.expect("the binder was assigned a VarId");
        let DataExprKind::Binary { rhs, .. } = &weight.node else {
            panic!("expected a Binary (division) weight, got {:?}", weight.node);
        };
        assert!(matches!(
            &rhs.node,
            DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == declared
        ));
    }

    #[test]
    fn test_assignment_value_resolves_but_key_does_not() {
        let text = "proc P(n: Nat) = P(n = n); init P(0);";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let declared = spec.process_declarations[0].params[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let ProcessExprKind::Id(_, assignments) = &spec.process_declarations[0].body.node else {
            panic!("expected an Id (instantiation) body");
        };
        // The key `n` is a plain `String` field (`AssignmentData::identifier`), never touched by
        // this pass; only the value `n` (the expression) is a `DataExpr` and gets resolved.
        assert_eq!(assignments[0].identifier, "n");
        assert!(matches!(
            &assignments[0].expr.node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_nested_data_binder_inside_process_action_argument_resolves() {
        let text = "act a: Bool; proc P(n: Nat) = a(exists x: Nat . x == n); init P(0);";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let n_declared = spec.process_declarations[0].params[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let ProcessExprKind::Action(_, args) = &spec.process_declarations[0].body.node else {
            panic!("expected an Action body");
        };
        let DataExprKind::Quantifier { variables, body, .. } = &args[0].node else {
            panic!("expected a Quantifier argument");
        };
        let x_declared = variables[0].var_id.expect("the binder was assigned a VarId");
        let DataExprKind::Binary { lhs, rhs, .. } = &body.node else {
            panic!("expected a Binary (==) body");
        };
        assert!(matches!(
            &lhs.node,
            DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == x_declared
        ));
        assert!(matches!(
            &rhs.node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == n_declared
        ));
        // The process parameter and the nested quantifier binder are distinct binders and must
        // never share an id, even though each is the "first" binder of its own construct.
        assert_ne!(n_declared, x_declared);
    }

    #[test]
    fn test_undeclared_name_is_left_unresolved() {
        let text = "act a: Nat; proc P = a(m); init P;";
        let mut spec = UntypedProcessSpecification::parse(text).unwrap();
        resolve_process_variables(&mut spec);

        let ProcessExprKind::Action(_, args) = &spec.process_declarations[0].body.node else {
            panic!("expected an Action body");
        };
        // `m` isn't declared anywhere; this pass leaves it as a plain `Id` for the existing
        // `UndeclaredName` inference error to reject later.
        assert!(matches!(&args[0].node, DataExprKind::Id(name) if name == "m"));
    }

    #[test]
    fn test_prop_var_inst_argument_resolves_to_equation_parameter() {
        let text = "pbes nu X(n: Nat) = val(n == 0) || X(n); init X(0);";
        let mut pbes = UntypedPbes::parse(text).unwrap();
        resolve_pbes_variables(&mut pbes);

        let declared = pbes.equations[0].variable.parameters[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let PbesExprKind::Binary { rhs, .. } = &pbes.equations[0].formula.node else {
            panic!("expected a Binary (||) formula");
        };
        let PbesExprKind::PropVarInst(inst) = &rhs.node else {
            panic!("expected a PropVarInst");
        };
        assert!(matches!(
            &inst.arguments[0].node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_quantifier_bound_variable_resolves_to_its_own_binder() {
        let text = "pbes mu X = forall n: Nat . val(n == n); init X;";
        let mut pbes = UntypedPbes::parse(text).unwrap();
        resolve_pbes_variables(&mut pbes);

        let PbesExprKind::Quantifier { variables, body, .. } = &pbes.equations[0].formula.node else {
            panic!("expected a Quantifier formula");
        };
        let declared = variables[0].var_id.expect("the binder was assigned a VarId");
        let PbesExprKind::DataValExpr(data_expr) = &body.node else {
            panic!("expected a DataValExpr body");
        };
        let DataExprKind::Binary { lhs, .. } = &data_expr.node else {
            panic!("expected a Binary (==) expression");
        };
        assert!(matches!(
            &lhs.node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_pbes_init_only_sees_globals_not_any_equations_own_parameters() {
        let text = "glob g: Nat; pbes nu X(n: Nat) = val(n == n); init X(g);";
        let mut pbes = UntypedPbes::parse(text).unwrap();
        resolve_pbes_variables(&mut pbes);

        let declared = pbes.global_variables[0]
            .var_id
            .expect("the global was assigned a VarId");
        assert!(matches!(
            &pbes.init.arguments[0].node,
            DataExprKind::Resolved(name, var_id) if name == "g" && *var_id == declared
        ));
    }

    #[test]
    fn test_pres_prop_var_inst_argument_resolves_to_equation_parameter() {
        let text = "pres mu X(n: Nat) = val(n) || X(n); init X(0);";
        let mut pres = UntypedPres::parse(text).unwrap();
        resolve_pres_variables(&mut pres);

        let declared = pres.equations[0].variable.parameters[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let PresExprKind::Binary { rhs, .. } = &pres.equations[0].formula.node else {
            panic!("expected a Binary (||) formula");
        };
        let PresExprKind::PropVarInst(inst) = &rhs.node else {
            panic!("expected a PropVarInst");
        };
        assert!(matches!(
            &inst.arguments[0].node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_pres_bound_variable_resolves_to_its_own_binder() {
        let text = "pres mu X = sum n: Nat . val(n); init X;";
        let mut pres = UntypedPres::parse(text).unwrap();
        resolve_pres_variables(&mut pres);

        let PresExprKind::Bound { variables, expr, .. } = &pres.equations[0].formula.node else {
            panic!("expected a Bound formula");
        };
        let declared = variables[0].var_id.expect("the binder was assigned a VarId");
        let PresExprKind::DataValExpr(data_expr) = &expr.node else {
            panic!("expected a DataValExpr body");
        };
        assert!(matches!(
            &data_expr.node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_pres_constant_multiply_resolves_its_constant() {
        let text = "pres mu X(n: Nat) = val(n) * X; init X;";
        let mut pres = UntypedPres::parse(text).unwrap();
        resolve_pres_variables(&mut pres);

        let declared = pres.equations[0].variable.parameters[0]
            .var_id
            .expect("the parameter was assigned a VarId");
        let PresExprKind::LeftConstantMultiply { constant, .. } = &pres.equations[0].formula.node else {
            panic!("expected a LeftConstantMultiply formula");
        };
        assert!(matches!(
            &constant.node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == declared
        ));
    }

    #[test]
    fn test_pres_init_only_sees_globals_not_any_equations_own_parameters() {
        let text = "glob g: Nat; pres nu X(n: Nat) = val(n); init X(g);";
        let mut pres = UntypedPres::parse(text).unwrap();
        resolve_pres_variables(&mut pres);

        let declared = pres.global_variables[0]
            .var_id
            .expect("the global was assigned a VarId");
        assert!(matches!(
            &pres.init.arguments[0].node,
            DataExprKind::Resolved(name, var_id) if name == "g" && *var_id == declared
        ));
    }

    #[test]
    fn test_data_specification_equation_variable_resolves_to_its_var_block_declaration() {
        let text = "map f: Nat -> Nat; var x: Nat; eqn f(x) = x;";
        let mut spec = UntypedDataSpecification::parse(text).unwrap();
        resolve_data_specification_variables(&mut spec);

        let declared = spec.equation_declarations[0].variables[0]
            .var_id
            .expect("the var-block variable was assigned a VarId");
        let equation = &spec.equation_declarations[0].equations[0];
        assert!(matches!(
            &equation.lhs.node,
            DataExprKind::Application { arguments, .. }
                if matches!(&arguments[0].node, DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == declared)
        ));
        assert!(matches!(
            &equation.rhs.node,
            DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == declared
        ));
    }

    #[test]
    fn test_data_specification_equation_condition_resolves_too() {
        let text = "map f: Bool -> Nat; var x: Bool; eqn x -> f(x) = 0;";
        let mut spec = UntypedDataSpecification::parse(text).unwrap();
        resolve_data_specification_variables(&mut spec);

        let declared = spec.equation_declarations[0].variables[0]
            .var_id
            .expect("the var-block variable was assigned a VarId");
        let equation = &spec.equation_declarations[0].equations[0];
        let condition = equation.condition.as_ref().expect("expected a condition");
        assert!(matches!(
            &condition.node,
            DataExprKind::Resolved(name, var_id) if name == "x" && *var_id == declared
        ));
    }

    #[test]
    fn test_data_specification_variable_scope_does_not_leak_across_var_blocks() {
        let text = "map f: Nat -> Nat; g: Bool -> Nat; var x: Nat; eqn f(x) = x; var y: Bool; eqn g(y) = y;";
        let mut spec = UntypedDataSpecification::parse(text).unwrap();
        resolve_data_specification_variables(&mut spec);

        // `y` isn't declared in the first block; it's a separate `var` block's own variable, so
        // this pass leaves it as-is when scoped to the first block. Only checking the second
        // block resolves correctly, and that the two blocks' variables never share an id, are the
        // interesting assertions here.
        let x_declared = spec.equation_declarations[0].variables[0].var_id.unwrap();
        let y_declared = spec.equation_declarations[1].variables[0]
            .var_id
            .expect("the second block's variable was assigned a VarId");
        assert_ne!(x_declared, y_declared);
        let second = &spec.equation_declarations[1].equations[0];
        assert!(matches!(
            &second.rhs.node,
            DataExprKind::Resolved(name, var_id) if name == "y" && *var_id == y_declared
        ));
    }

    #[test]
    fn test_fixed_point_variable_resolves_to_its_own_binder() {
        let text = "mu X(n: Nat = 0) . val(n) || X(n)";
        let mut spec = UntypedStateFrmSpec::parse(text).unwrap();
        resolve_modal_variables(&mut spec);

        let StateFrmKind::FixedPoint { variable, body, .. } = &spec.formula.node else {
            panic!("expected a FixedPoint formula");
        };
        let declared = variable.id.expect("the fixpoint variable was assigned a StateVarId");
        let StateFrmKind::Binary { rhs, .. } = &body.node else {
            panic!("expected a Binary (||) body");
        };
        assert!(matches!(
            &rhs.node,
            StateFrmKind::Resolved(name, _, id) if name == "X" && *id == declared
        ));
    }

    #[test]
    fn test_nested_fixed_point_variables_of_the_same_name_do_not_share_an_id() {
        // The inner, parameter-less `X` refers to the *inner* `nu X`, shadowing the outer `mu X`
        // of the same name — the two binders must never share a StateVarId.
        let text = "mu X(n: Nat = 0) . [true](nu X. X)";
        let mut spec = UntypedStateFrmSpec::parse(text).unwrap();
        resolve_modal_variables(&mut spec);

        let StateFrmKind::FixedPoint {
            variable: outer, body, ..
        } = &spec.formula.node
        else {
            panic!("expected an outer FixedPoint formula");
        };
        let outer_declared = outer.id.expect("the outer fixpoint variable was assigned a StateVarId");
        let StateFrmKind::Modality { expr, .. } = &body.node else {
            panic!("expected a Modality body");
        };
        let StateFrmKind::FixedPoint {
            variable: inner,
            body: inner_body,
            ..
        } = &expr.node
        else {
            panic!("expected a nested FixedPoint formula");
        };
        let inner_declared = inner.id.expect("the inner fixpoint variable was assigned a StateVarId");
        assert_ne!(outer_declared, inner_declared);
        assert!(matches!(
            &inner_body.node,
            StateFrmKind::Resolved(name, _, id) if name == "X" && *id == inner_declared
        ));
    }

    #[test]
    fn test_action_formula_quantifier_resolves_to_its_own_binder_and_can_see_an_outer_state_formula_binder() {
        // Exercises `resolve_in_act_frm`'s own scoping specifically: `m` is bound by the action
        // formula's own `exists`, `n` by the *enclosing* state formula's `forall`, reached by
        // crossing through the modality's `RegFrm` -- both must resolve, to two different ids.
        let text = "act a: Nat # Nat; form forall n: Nat . [exists m: Nat . a(m, n)] true;";
        let mut spec = UntypedStateFrmSpec::parse(text).unwrap();
        resolve_modal_variables(&mut spec);

        let StateFrmKind::Quantifier { variables, body, .. } = &spec.formula.node else {
            panic!("expected a Quantifier formula");
        };
        let n_declared = variables[0].var_id.expect("the outer binder was assigned a VarId");

        let StateFrmKind::Modality { formula: reg, .. } = &body.node else {
            panic!("expected a Modality body");
        };
        let RegFrmKind::Action(act_frm) = &reg.node else {
            panic!("expected an Action regular formula");
        };
        let ActFrmKind::Quantifier {
            variables: act_vars,
            body: act_body,
            ..
        } = &act_frm.node
        else {
            panic!("expected a Quantifier action formula");
        };
        let m_declared = act_vars[0]
            .var_id
            .expect("the action formula binder was assigned a VarId");
        assert_ne!(n_declared, m_declared);

        let ActFrmKind::MultAct(multi_action) = &act_body.node else {
            panic!("expected a MultAct body");
        };
        assert!(matches!(
            &multi_action.actions[0].args[0].node,
            DataExprKind::Resolved(name, var_id) if name == "m" && *var_id == m_declared
        ));
        assert!(matches!(
            &multi_action.actions[0].args[1].node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == n_declared
        ));
    }

    #[test]
    fn test_action_formula_quantifier_shadows_an_outer_state_formula_binder_of_the_same_name() {
        // `n` is bound twice: the outer `forall` (`Bool`) and the action formula's own `exists`
        // (`Nat`). Inside the action formula, `n` must resolve to the *inner* binder -- the two
        // must never share a VarId.
        let text = "act a: Nat; form forall n: Bool . [exists n: Nat . a(n)] true;";
        let mut spec = UntypedStateFrmSpec::parse(text).unwrap();
        resolve_modal_variables(&mut spec);

        let StateFrmKind::Quantifier { variables, body, .. } = &spec.formula.node else {
            panic!("expected a Quantifier formula");
        };
        let outer_declared = variables[0].var_id.expect("the outer binder was assigned a VarId");

        let StateFrmKind::Modality { formula: reg, .. } = &body.node else {
            panic!("expected a Modality body");
        };
        let RegFrmKind::Action(act_frm) = &reg.node else {
            panic!("expected an Action regular formula");
        };
        let ActFrmKind::Quantifier {
            variables: act_vars,
            body: act_body,
            ..
        } = &act_frm.node
        else {
            panic!("expected a Quantifier action formula");
        };
        let inner_declared = act_vars[0].var_id.expect("the inner binder was assigned a VarId");
        assert_ne!(outer_declared, inner_declared);

        let ActFrmKind::MultAct(multi_action) = &act_body.node else {
            panic!("expected a MultAct body");
        };
        assert!(matches!(
            &multi_action.actions[0].args[0].node,
            DataExprKind::Resolved(name, var_id) if name == "n" && *var_id == inner_declared
        ));
    }
}

#[cfg(test)]
mod stack_depth_probe {
    //! Isolates `resolve_in_state_frm`'s own recursion from the parser's: the tree here is built
    //! directly, so a stack overflow can only come from this module's own walk. This was the last
    //! of the four SIGABRT instances `review/stack-overflow-recursion.md` originally found still
    //! using hand-written recursive descent, now migrated onto `Traverse::apply_scoped`
    //! (`crates/syntax/src/traverse.rs`'s mutating counterpart of `visit_scoped`, added for this
    //! migration). `formula` is let drop normally at the end of the test (rather than leaked with
    //! `mem::forget`) as proof the separate recursive-`Drop` bug covering the same document doesn't
    //! fire here either.
    use merc_syntax::Span;
    use merc_syntax::StateFrm;
    use merc_syntax::StateFrmKind;
    use merc_syntax::StateFrmUnaryOp;

    use super::StateVarIdAllocator;
    use super::VarIdAllocator;
    use super::resolve_in_state_frm;

    fn deep_negation(depth: usize) -> StateFrm {
        let mut formula = StateFrmKind::True.spanned(Span::default());
        for _ in 0..depth {
            formula = StateFrmKind::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(formula),
            }
            .spanned(Span::default());
        }
        formula
    }

    #[test]
    fn deeply_nested_negation_does_not_overflow_the_stack() {
        let mut ids = VarIdAllocator::default();
        let mut state_var_ids = StateVarIdAllocator::default();

        // 100,000 nested negations, exactly as `modal::check`/`process::check`/`pres::check`'s own
        // probes do.
        let mut formula = deep_negation(100_000);
        resolve_in_state_frm(&mut formula, &mut ids, &mut state_var_ids);
    }
}
