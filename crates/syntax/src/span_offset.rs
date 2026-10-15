use merc_utilities::Span;

use crate::ActDecl;
use crate::ActFrm;
use crate::ActFrmKind;
use crate::Assignment;
use crate::BagElement;
use crate::ConstructorDecl;
use crate::DataExpr;
use crate::DataExprKind;
use crate::EqnDecl;
use crate::EqnSpec;
use crate::IdDecl;
use crate::MultiAction;
use crate::ProcDecl;
use crate::ProcessExpr;
use crate::ProcessExprKind;
use crate::RegFrm;
use crate::RegFrmKind;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::StateFrm;
use crate::StateFrmKind;
use crate::StateVarAssignment;
use crate::StateVarDecl;
use crate::UntypedDataSpecification;
use crate::UntypedProcessSpecification;
use crate::UntypedStateFrmSpec;

/// Implemented by every top-level parsed specification [`crate::imports`] and
/// the bundled/generated template machinery need to rebase into a shared
/// [`SourceMap`](merc_utilities::SourceMap).
pub trait OffsetSpans {
    /// Applies `f` to every span reachable from `self`.
    fn map_spans<F: FnMut(&mut Span)>(&mut self, f: &mut F);

    /// Shifts every span reachable from `self` by `delta`.
    fn offset_spans(&mut self, delta: usize) {
        self.map_spans(&mut |span| span.shift(delta));
    }
}

impl OffsetSpans for UntypedDataSpecification {
    fn map_spans<F: FnMut(&mut Span)>(&mut self, f: &mut F) {
        for decl in &mut self.sort_declarations {
            offset_sort_decl(decl, f);
        }
        for decl in &mut self.constructor_declarations {
            offset_id_decl(decl, f);
        }
        for decl in &mut self.map_declarations {
            offset_id_decl(decl, f);
        }
        for eqn_spec in &mut self.equation_declarations {
            offset_eqn_spec(eqn_spec, f);
        }
        for decl in &mut self.type_var_declarations {
            f(&mut decl.span);
        }
    }
}

impl OffsetSpans for UntypedProcessSpecification {
    fn map_spans<F: FnMut(&mut Span)>(&mut self, f: &mut F) {
        self.data_specification.map_spans(f);
        for decl in &mut self.global_variables {
            offset_id_decl(decl, f);
        }
        for decl in &mut self.action_declarations {
            offset_act_decl(decl, f);
        }
        for decl in &mut self.process_declarations {
            offset_proc_decl(decl, f);
        }
        if let Some(init) = &mut self.init {
            offset_process_expr(init, f);
        }
    }
}

impl OffsetSpans for UntypedStateFrmSpec {
    fn map_spans<F: FnMut(&mut Span)>(&mut self, f: &mut F) {
        self.data_specification.map_spans(f);
        for decl in &mut self.action_declarations {
            offset_act_decl(decl, f);
        }
        offset_state_frm(&mut self.formula, f);
    }
}

fn offset_sort_decl<F: FnMut(&mut Span)>(decl: &mut SortDecl, f: &mut F) {
    f(&mut decl.span);
    if let Some(expr) = &mut decl.expr {
        offset_sort_expression(expr, f);
    }
}

fn offset_id_decl<Id, F: FnMut(&mut Span)>(decl: &mut IdDecl<Id>, f: &mut F) {
    f(&mut decl.identifier.span);
    offset_sort_expression(&mut decl.sort, f);
}

fn offset_sort_expression<F: FnMut(&mut Span)>(sort: &mut SortExpression, f: &mut F) {
    f(&mut sort.span);
    match &mut sort.node {
        SortExpressionKind::Product { lhs, rhs } => {
            offset_sort_expression(lhs, f);
            offset_sort_expression(rhs, f);
        }
        SortExpressionKind::Function { domain, range } => {
            offset_sort_expression(domain, f);
            offset_sort_expression(range, f);
        }
        SortExpressionKind::FlattenedFunction { domain, range } => {
            for sort in domain {
                offset_sort_expression(sort, f);
            }
            offset_sort_expression(range, f);
        }
        SortExpressionKind::Struct { inner } => {
            for constructor in inner {
                offset_constructor_decl(constructor, f);
            }
        }
        SortExpressionKind::Complex(_, sort) => offset_sort_expression(sort, f),
        SortExpressionKind::Reference(_)
        | SortExpressionKind::TypeVar(_)
        | SortExpressionKind::ResolvedTypeVar(_)
        | SortExpressionKind::Simple(_)
        | SortExpressionKind::Resolved(_, _) => {}
    }
}

fn offset_constructor_decl<F: FnMut(&mut Span)>(constructor: &mut ConstructorDecl, f: &mut F) {
    f(&mut constructor.name.span);
    for (name, sort) in &mut constructor.args {
        if let Some(name) = name {
            f(&mut name.span);
        }
        offset_sort_expression(sort, f);
    }
    if let Some(recogniser) = &mut constructor.recogniser {
        f(&mut recogniser.span);
    }
}

fn offset_eqn_spec<F: FnMut(&mut Span)>(eqn_spec: &mut EqnSpec, f: &mut F) {
    f(&mut eqn_spec.span);
    for variable in &mut eqn_spec.variables {
        offset_id_decl(variable, f);
    }
    for equation in &mut eqn_spec.equations {
        offset_eqn_decl(equation, f);
    }
}

fn offset_eqn_decl<F: FnMut(&mut Span)>(equation: &mut EqnDecl, f: &mut F) {
    f(&mut equation.span);
    if let Some(condition) = &mut equation.condition {
        offset_data_expr(condition, f);
    }
    offset_data_expr(&mut equation.lhs, f);
    offset_data_expr(&mut equation.rhs, f);
}

fn offset_data_expr<F: FnMut(&mut Span)>(expr: &mut DataExpr, f: &mut F) {
    f(&mut expr.span);
    match &mut expr.node {
        DataExprKind::Resolved(_, _, declaration) => f(declaration),
        DataExprKind::Id(_)
        | DataExprKind::Number(_)
        | DataExprKind::Bool(_)
        | DataExprKind::EmptyList
        | DataExprKind::EmptySet
        | DataExprKind::EmptyBag => {}
        DataExprKind::Application { function, arguments } => {
            offset_data_expr(function, f);
            for argument in arguments {
                offset_data_expr(argument, f);
            }
        }
        DataExprKind::List(exprs) | DataExprKind::Set(exprs) => {
            for expr in exprs {
                offset_data_expr(expr, f);
            }
        }
        DataExprKind::Bag(elements) => {
            for element in elements {
                offset_bag_element(element, f);
            }
        }
        DataExprKind::SetBagComp { variable, predicate } => {
            offset_id_decl(variable, f);
            offset_data_expr(predicate, f);
        }
        DataExprKind::Lambda { variables, body } | DataExprKind::Quantifier { variables, body, .. } => {
            for variable in variables {
                offset_id_decl(variable, f);
            }
            offset_data_expr(body, f);
        }
        DataExprKind::Unary { expr, .. } => offset_data_expr(expr, f),
        DataExprKind::Binary { lhs, rhs, .. } => {
            offset_data_expr(lhs, f);
            offset_data_expr(rhs, f);
        }
        DataExprKind::FunctionUpdate { expr, update } => {
            offset_data_expr(expr, f);
            offset_data_expr(&mut update.expr, f);
            offset_data_expr(&mut update.update, f);
        }
        DataExprKind::Whr { expr, assignments } => {
            offset_data_expr(expr, f);
            for assignment in assignments {
                offset_assignment(assignment, f);
            }
        }
    }
}

fn offset_bag_element<F: FnMut(&mut Span)>(element: &mut BagElement, f: &mut F) {
    offset_data_expr(&mut element.expr, f);
    offset_data_expr(&mut element.multiplicity, f);
}

fn offset_assignment<F: FnMut(&mut Span)>(assignment: &mut Assignment, f: &mut F) {
    f(&mut assignment.span);
    offset_data_expr(&mut assignment.expr, f);
}

fn offset_act_decl<F: FnMut(&mut Span)>(decl: &mut ActDecl, f: &mut F) {
    f(&mut decl.span);
    f(&mut decl.identifier.span);
    for sort in &mut decl.args {
        offset_sort_expression(sort, f);
    }
}

fn offset_proc_decl<F: FnMut(&mut Span)>(decl: &mut ProcDecl, f: &mut F) {
    f(&mut decl.span);
    f(&mut decl.identifier.span);
    for param in &mut decl.params {
        offset_id_decl(param, f);
    }
    offset_process_expr(&mut decl.body, f);
}

fn offset_process_expr<F: FnMut(&mut Span)>(expr: &mut ProcessExpr, f: &mut F) {
    f(&mut expr.span);
    match &mut expr.node {
        ProcessExprKind::Delta | ProcessExprKind::Tau => {}
        ProcessExprKind::Id(name, assignments) => {
            f(&mut name.span);
            for assignment in assignments {
                offset_assignment(assignment, f);
            }
        }
        ProcessExprKind::Action(name, args) => {
            f(&mut name.span);
            for arg in args {
                offset_data_expr(arg, f);
            }
        }
        ProcessExprKind::Sum { variables, operand } => {
            for variable in variables {
                offset_id_decl(variable, f);
            }
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Dist {
            variables,
            expr,
            operand,
        } => {
            for variable in variables {
                offset_id_decl(variable, f);
            }
            offset_data_expr(expr, f);
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Binary { lhs, rhs, .. } => {
            offset_process_expr(lhs, f);
            offset_process_expr(rhs, f);
        }
        ProcessExprKind::Hide { actions, operand } | ProcessExprKind::Block { actions, operand } => {
            for action in actions {
                f(&mut action.span);
            }
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Rename { renames, operand } => {
            for rename in renames {
                f(&mut rename.from.span);
                f(&mut rename.to.span);
            }
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Allow { actions, operand } => {
            for label in actions {
                for action in &mut label.actions {
                    f(&mut action.span);
                }
            }
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Comm { comm, operand } => {
            for expr in comm {
                for action in &mut expr.from.actions {
                    f(&mut action.span);
                }
                f(&mut expr.to.span);
            }
            offset_process_expr(operand, f);
        }
        ProcessExprKind::Condition { condition, then, else_ } => {
            offset_data_expr(condition, f);
            offset_process_expr(then, f);
            if let Some(operand) = else_ {
                offset_process_expr(operand, f);
            }
        }
        ProcessExprKind::At { expr, operand } => {
            offset_process_expr(expr, f);
            offset_data_expr(operand, f);
        }
    }
}

fn offset_state_frm<F: FnMut(&mut Span)>(formula: &mut StateFrm, f: &mut F) {
    f(&mut formula.span);
    match &mut formula.node {
        StateFrmKind::True | StateFrmKind::False => {}
        StateFrmKind::Delay(time) | StateFrmKind::Yaled(time) => {
            if let Some(expr) = time {
                offset_data_expr(expr, f);
            }
        }
        StateFrmKind::Id(name, args) | StateFrmKind::Resolved(name, args, _) => {
            f(&mut name.span);
            for arg in args {
                offset_data_expr(arg, f);
            }
        }
        StateFrmKind::DataValExprLeftMult(expr, formula) => {
            offset_data_expr(expr, f);
            offset_state_frm(formula, f);
        }
        StateFrmKind::DataValExprRightMult(formula, expr) => {
            offset_state_frm(formula, f);
            offset_data_expr(expr, f);
        }
        StateFrmKind::DataValExpr(expr) => offset_data_expr(expr, f),
        StateFrmKind::Modality {
            formula: reg_frm, expr, ..
        } => {
            offset_reg_frm(reg_frm, f);
            offset_state_frm(expr, f);
        }
        StateFrmKind::Unary { expr, .. } => offset_state_frm(expr, f),
        StateFrmKind::Binary { lhs, rhs, .. } => {
            offset_state_frm(lhs, f);
            offset_state_frm(rhs, f);
        }
        StateFrmKind::Quantifier { variables, body, .. } | StateFrmKind::Bound { variables, body, .. } => {
            for variable in variables {
                offset_id_decl(variable, f);
            }
            offset_state_frm(body, f);
        }
        StateFrmKind::FixedPoint { variable, body, .. } => {
            offset_state_var_decl(variable, f);
            offset_state_frm(body, f);
        }
    }
}

fn offset_state_var_decl<F: FnMut(&mut Span)>(decl: &mut StateVarDecl, f: &mut F) {
    f(&mut decl.span);
    f(&mut decl.identifier.span);
    for argument in &mut decl.arguments {
        offset_state_var_assignment(argument, f);
    }
}

fn offset_state_var_assignment<F: FnMut(&mut Span)>(assignment: &mut StateVarAssignment, f: &mut F) {
    offset_id_decl(&mut assignment.decl, f);
    offset_data_expr(&mut assignment.expr, f);
}

fn offset_reg_frm<F: FnMut(&mut Span)>(formula: &mut RegFrm, f: &mut F) {
    f(&mut formula.span);
    match &mut formula.node {
        RegFrmKind::Action(act_frm) => offset_act_frm(act_frm, f),
        RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => offset_reg_frm(inner, f),
        RegFrmKind::Sequence { lhs, rhs } | RegFrmKind::Choice { lhs, rhs } => {
            offset_reg_frm(lhs, f);
            offset_reg_frm(rhs, f);
        }
    }
}

fn offset_act_frm<F: FnMut(&mut Span)>(formula: &mut ActFrm, f: &mut F) {
    f(&mut formula.span);
    match &mut formula.node {
        ActFrmKind::True | ActFrmKind::False => {}
        ActFrmKind::MultAct(multi_action) => offset_multi_action(multi_action, f),
        ActFrmKind::DataExprVal(expr) => offset_data_expr(expr, f),
        ActFrmKind::Negation(inner) => offset_act_frm(inner, f),
        ActFrmKind::Quantifier { variables, body, .. } => {
            for variable in variables {
                offset_id_decl(variable, f);
            }
            offset_act_frm(body, f);
        }
        ActFrmKind::Binary { lhs, rhs, .. } => {
            offset_act_frm(lhs, f);
            offset_act_frm(rhs, f);
        }
        ActFrmKind::At { expr, operand } => {
            offset_act_frm(expr, f);
            offset_data_expr(operand, f);
        }
    }
}

fn offset_multi_action<F: FnMut(&mut Span)>(multi_action: &mut MultiAction, f: &mut F) {
    for action in &mut multi_action.actions {
        f(&mut action.id.span);
        for arg in &mut action.args {
            offset_data_expr(arg, f);
        }
    }
}
