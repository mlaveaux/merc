//! The PBES syntax as an [EquationSystem], whose embedded data expressions are Boolean.

use merc_syntax::PbesExpr;
use merc_syntax::PbesExprKind;
use merc_typecheck::DataSpecification;
use merc_typecheck::internal::ResolvedSortId;
use merc_typecheck::internal::bool_sort;

use crate::EquationSystem;
use crate::EquationSystemError;
use crate::ExprNode;
use crate::ExprNodeMut;
use crate::Specification;

/// The [EquationSystem] of a PBES.
pub enum PbesSystem {}

impl EquationSystem for PbesSystem {
    type Expr = PbesExpr;

    fn value_sort(data: &DataSpecification) -> ResolvedSortId {
        bool_sort(data)
    }

    fn node(expr: &PbesExpr) -> ExprNode<'_> {
        match &expr.node {
            PbesExprKind::DataValExpr(data_expr) => ExprNode::Data(data_expr),
            PbesExprKind::PropVarInst(inst) => ExprNode::Instantiation(inst),
            PbesExprKind::Quantifier { variables, .. } => ExprNode::Binder(variables),
            PbesExprKind::True | PbesExprKind::False | PbesExprKind::Negation(_) | PbesExprKind::Binary { .. } => {
                ExprNode::Other
            }
        }
    }

    fn node_mut(expr: &mut PbesExpr) -> ExprNodeMut<'_> {
        match &mut expr.node {
            PbesExprKind::DataValExpr(data_expr) => ExprNodeMut::Data(data_expr),
            PbesExprKind::PropVarInst(inst) => ExprNodeMut::Instantiation(inst),
            PbesExprKind::Quantifier { variables, .. } => ExprNodeMut::Binder(variables),
            PbesExprKind::True | PbesExprKind::False | PbesExprKind::Negation(_) | PbesExprKind::Binary { .. } => {
                ExprNodeMut::Other
            }
        }
    }
}

/// A type-checked mCRL2 PBES, see [Specification].
pub type PbesSpecification = Specification<PbesSystem>;

/// An error type checking a PBES, see [EquationSystemError].
pub type PbesError = EquationSystemError;
