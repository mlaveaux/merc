//! Renders a checked equation's expressions annotated with every sub-expression's resolved
//! sort, for a stricter regression signal than the plain (unannotated) equation text gives —
//! see [`crate::DataSpecification::to_typed_string`]'s doc comment for why this exists.

use std::fmt;

use merc_syntax::DataExpr;
use merc_syntax::DataExprKind;
use merc_syntax::UntypedDataSpecification;

use crate::DisplaySortContext;
use crate::EquationTyping;
use crate::ExprId;
use crate::ResolvedSort;
use crate::Roots;
use crate::TypeCheckContext;
use crate::for_each_expr_child;
use crate::subtree_sizes;

/// A displayable wrapper around the expressions of one inference run.
pub(crate) struct TypedRoots<'a> {
    roots: Roots<'a>,
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    typing: &'a EquationTyping,
}

impl<'a> TypedRoots<'a> {
    /// `typing` must be the typing inferred for exactly `roots`.
    pub(crate) fn new(
        roots: Roots<'a>,
        ctx: &'a TypeCheckContext,
        spec: &'a UntypedDataSpecification,
        typing: &'a EquationTyping,
    ) -> Self {
        Self {
            roots,
            ctx,
            spec,
            typing,
        }
    }
}

impl fmt::Display for TypedRoots<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let display = TypedDisplay {
            ctx: self.ctx,
            spec: self.spec,
            typing: self.typing,
            sizes: subtree_sizes(self.roots.exprs()),
        };

        // Each root's id follows the previous root's whole subtree.
        let mut next = 0;
        let mut roots = self.roots.exprs().map(|expr| {
            let id = next;
            next += display.sizes[id];
            TypedExpr {
                expr,
                id: ExprId::new(id),
                display: &display,
            }
        });
        let mut root = || roots.next().expect("one id per root expression");

        match self.roots {
            Roots::Equation { condition: Some(_), .. } => write!(f, "{} -> {} = {}", root(), root(), root()),
            Roots::Equation { condition: None, .. } => write!(f, "{} = {}", root(), root()),
            Roots::Expression(_) | Roots::ExpressionAgainst { .. } => write!(f, "{}", root()),
        }
    }
}

/// What every [TypedExpr] of one [TypedRoots] shares.
struct TypedDisplay<'a> {
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    typing: &'a EquationTyping,
    /// See [subtree_sizes]; used to compute each child's [ExprId].
    sizes: Vec<usize>,
}

/// One expression node together with its [ExprId], rendered with its resolved sort.
struct TypedExpr<'a> {
    expr: &'a DataExpr,
    id: ExprId,
    display: &'a TypedDisplay<'a>,
}

impl<'a> TypedExpr<'a> {
    /// The children of this node that have an [ExprId], in [for_each_expr_child]'s order.
    fn children(&self) -> Vec<TypedExpr<'a>> {
        let mut children = Vec::new();
        let mut next = *self.id + 1;
        for_each_expr_child(self.expr, |expr| {
            children.push(TypedExpr {
                expr,
                id: ExprId::new(next),
                display: self.display,
            });
            next += self.display.sizes[next];
        });
        children
    }

    /// Renders just this node's shape, without the trailing `: <sort>` annotation `Display`
    /// appends — what an `Application` renders its callee with in place, and what an
    /// `Application` with no arguments collapses to.
    fn fmt_shape(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut children = self.children();
        match &self.expr.node {
            DataExprKind::EmptyList => write!(f, "[]"),
            DataExprKind::EmptyBag => write!(f, "{{:}}"),
            DataExprKind::EmptySet => write!(f, "{{}}"),
            DataExprKind::Id(name) | DataExprKind::Resolved(name, _, _) => write!(f, "{name}"),
            DataExprKind::Number(value) => write!(f, "{value}"),
            DataExprKind::Bool(value) => write!(f, "{value}"),
            DataExprKind::Set(_) => {
                write!(f, "{{ ")?;
                for (index, member) in children.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{member}")?;
                }
                write!(f, " }}")
            }
            DataExprKind::Bag(_) => {
                write!(f, "{{ ")?;
                for (index, member) in children.chunks(2).enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", member[0], member[1])?;
                }
                write!(f, " }}")
            }
            DataExprKind::SetBagComp { variable, .. } => {
                // The bound variable has no `ExprId` of its own, so only the predicate is
                // annotated.
                write!(f, "{{ {variable} | {} }}", children[0])
            }
            DataExprKind::Application { .. } => {
                let function = children.pop().expect("an application has a function");
                function.fmt_shape(f)?;

                // A defensively-made fallback for a callee taking no arguments at all (in
                // practice every parsed `Application` has at least one) collapses to exactly
                // the callee alone, matching the plain, unannotated `Display for DataExpr`.
                if children.is_empty() {
                    return Ok(());
                }
                write!(f, "(")?;
                for (index, argument) in children.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{argument}")?;
                }
                write!(f, ")")
            }
            DataExprKind::Lambda { variables, .. } => {
                let variables: Vec<String> = variables.iter().map(ToString::to_string).collect();
                write!(f, "(lambda {} . {})", variables.join(", "), children[0])
            }
            DataExprKind::Quantifier { op, variables, .. } => {
                let variables: Vec<String> = variables.iter().map(ToString::to_string).collect();
                write!(f, "({op} {} . {})", variables.join(", "), children[0])
            }
            DataExprKind::Whr { assignments, .. } => {
                let body = children.pop().expect("a whr has a body");
                let parts: Vec<String> = assignments
                    .iter()
                    .zip(&children)
                    .map(|(assignment, value)| format!("{} = {value}", assignment.identifier))
                    .collect();
                write!(f, "{body} whr {} end", parts.join(", "))
            }
            DataExprKind::List(_)
            | DataExprKind::Unary { .. }
            | DataExprKind::Binary { .. }
            | DataExprKind::FunctionUpdate { .. } => {
                unreachable!("typed-display requires a lowered expression, exactly like inference itself")
            }
        }
    }
}

impl fmt::Display for TypedExpr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An `Application`'s trailing annotation is the *applied* (base) callee's sort — a full
        // domain `#`-separated `-> range` arrow — rather than this node's own (just the range),
        // so `f(x)` reads as `f(x: S): (S -> T)` instead of the more redundant
        // `f: (S -> T)(x: S): T`. For a spine like `b(i)(j)` the base callee is `b` itself —
        // the inner `b(i)` node's sort is only the arrow's prefix — reached by descending each
        // `Application`'s own function, its last child.
        let mut base = self.id;
        if let DataExprKind::Application { .. } = &self.expr.node {
            let mut callee = self.children().pop().expect("an application has a function");
            while let DataExprKind::Application { .. } = &callee.expr.node {
                callee = callee.children().pop().expect("an application has a function");
            }
            base = callee.id;
        }

        let TypedDisplay { ctx, spec, typing, .. } = self.display;
        let sort = typing.sorts[*base];
        self.fmt_shape(f)?;
        let display = DisplaySortContext::new(ctx, spec, sort);
        if matches!(ctx.sorts.get(sort), ResolvedSort::Function { .. }) {
            write!(f, ": ({display})")
        } else {
            write!(f, ": {display}")
        }
    }
}
