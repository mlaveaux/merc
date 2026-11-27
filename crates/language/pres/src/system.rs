//! The PRES syntax as an [EquationSystem], whose embedded data expressions are real-valued.

use merc_pbes::EquationSystem;
use merc_pbes::EquationSystemError;
use merc_pbes::ExprNode;
use merc_pbes::ExprNodeMut;
use merc_pbes::Specification;
use merc_syntax::PresExpr;
use merc_syntax::PresExprKind;
use merc_typecheck::DataSpecification;
use merc_typecheck::internal::ResolvedSortId;
use merc_typecheck::internal::real_sort;

/// The [EquationSystem] of a PRES. Besides `val(...)`, the constant of a constant multiplication is
/// a real-valued data expression too.
pub enum PresSystem {}

impl EquationSystem for PresSystem {
    type Expr = PresExpr;

    fn value_sort(data: &DataSpecification) -> ResolvedSortId {
        real_sort(data)
    }

    fn node(expr: &PresExpr) -> ExprNode<'_> {
        match &expr.node {
            PresExprKind::DataValExpr(data_expr) => ExprNode::Data(data_expr),
            PresExprKind::RightConstantMultiply { constant, .. }
            | PresExprKind::LeftConstantMultiply { constant, .. } => ExprNode::Data(constant),
            PresExprKind::PropVarInst(inst) => ExprNode::Instantiation(inst),
            PresExprKind::Bound { variables, .. } => ExprNode::Binder(variables),
            PresExprKind::True
            | PresExprKind::False
            | PresExprKind::Negation(_)
            | PresExprKind::Binary { .. }
            | PresExprKind::Equal { .. }
            | PresExprKind::Condition { .. } => ExprNode::Other,
        }
    }

    fn node_mut(expr: &mut PresExpr) -> ExprNodeMut<'_> {
        match &mut expr.node {
            PresExprKind::DataValExpr(data_expr) => ExprNodeMut::Data(data_expr),
            PresExprKind::RightConstantMultiply { constant, .. }
            | PresExprKind::LeftConstantMultiply { constant, .. } => ExprNodeMut::Data(constant),
            PresExprKind::PropVarInst(inst) => ExprNodeMut::Instantiation(inst),
            PresExprKind::Bound { variables, .. } => ExprNodeMut::Binder(variables),
            PresExprKind::True
            | PresExprKind::False
            | PresExprKind::Negation(_)
            | PresExprKind::Binary { .. }
            | PresExprKind::Equal { .. }
            | PresExprKind::Condition { .. } => ExprNodeMut::Other,
        }
    }
}

/// A type-checked mCRL2 PRES, see [Specification].
pub type PresSpecification = Specification<PresSystem>;

/// An error type checking a PRES, see [EquationSystemError].
pub type PresError = EquationSystemError;
