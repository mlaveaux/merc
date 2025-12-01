use std::fmt;

use itertools::Itertools;

use crate::ActDecl;
use crate::ActFrm;
use crate::ActFrmBinaryOp;
use crate::Action;
use crate::Assignment;
use crate::Bound;
use crate::Comm;
use crate::ComplexSort;
use crate::ConstructorDecl;
use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::DataExprUnaryOp;
use crate::DataExprUpdate;
use crate::EqnDecl;
use crate::EqnSpec;
use crate::FixedPointOperator;
use crate::IdDecl;
use crate::ModalityOperator;
use crate::MultiAction;
use crate::MultiActionLabel;
use crate::PbesEquation;
use crate::PbesExpr;
use crate::PbesExprBinaryOp;
use crate::ProcDecl;
use crate::ProcExprBinaryOp;
use crate::ProcessExpr;
use crate::PropVarDecl;
use crate::PropVarInst;
use crate::Quantifier;
use crate::RegFrm;
use crate::Rename;
use crate::Sort;
use crate::SortDecl;
use crate::SortExpression;
use crate::Span;
use crate::StateFrm;
use crate::StateFrmOp;
use crate::StateFrmUnaryOp;
use crate::StateVarAssignment;
use crate::StateVarDecl;
use crate::UntypedDataSpecification;
use crate::UntypedPbes;
use crate::UntypedProcessSpecification;
use crate::UntypedStateFrmSpec;
use crate::VarDecl;

/// Prints location information for a span in the source.
pub fn print_location(input: &str, span: &Span) {
    input.lines().enumerate().fold(span.start, |current, (number, line)| {
        if current < line.len() {
            println!("ln {number}, col {}", span.start - current);
        }
        current - line.len()
    });
}

// Display implementations
impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for ComplexSort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} = {}", self.identifier, self.expr)
    }
}

impl fmt::Display for UntypedProcessSpecification {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;

        if !self.action_declarations.is_empty() {
            writeln!(f, "act")?;
            for act_decl in &self.action_declarations {
                writeln!(f, "   {act_decl};")?;
            }

            writeln!(f)?;
        }

        if !self.process_declarations.is_empty() {
            writeln!(f, "proc")?;
            for proc_decl in &self.process_declarations {
                writeln!(f, "   {proc_decl};")?;
            }

            writeln!(f)?;
        }

        if !self.global_variables.is_empty() {
            writeln!(f, "glob")?;
            for var_decl in &self.global_variables {
                writeln!(f, "   {var_decl};")?;
            }

            writeln!(f)?;
        }

        if let Some(init) = &self.init {
            writeln!(f, "init {init};")?;
        }
        Ok(())
    }
}

impl fmt::Display for UntypedDataSpecification {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.sort_declarations.is_empty() {
            writeln!(f, "sort")?;
            for decl in &self.sort_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        if !self.constructor_declarations.is_empty() {
            writeln!(f, "cons")?;
            for decl in &self.constructor_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        if !self.map_declarations.is_empty() {
            writeln!(f, "map")?;
            for decl in &self.map_declarations {
                writeln!(f, "   {decl};")?;
            }

            writeln!(f)?;
        }

        for decl in &self.equation_declarations {
            writeln!(f, "{decl}")?;
        }
        Ok(())
    }
}

impl fmt::Display for UntypedPbes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;
        writeln!(f)?;
        if !self.global_variables.is_empty() {
            writeln!(f, "glob")?;
            for var_decl in &self.global_variables {
                writeln!(f, "   {var_decl};")?;
            }

            writeln!(f)?;
        }
        writeln!(f)?;

        if !self.equations.is_empty() {
            writeln!(f, "pbes")?;
            for equation in &self.equations {
                writeln!(f, "   {equation};")?;
            }
        }

        writeln!(f, "init {};", self.init)
    }
}

impl fmt::Display for PropVarInst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.arguments.iter().format(", "))
        }
    }
}

impl fmt::Display for PbesEquation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} = {}", self.operator, self.variable, self.formula)
    }
}

impl fmt::Display for PropVarDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.parameters.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.parameters.iter().format(", "))
        }
    }
}

impl fmt::Display for PbesExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PbesExpr::True => write!(f, "true"),
            PbesExpr::False => write!(f, "false"),
            PbesExpr::PropVarInst(instance) => write!(f, "{instance}"),
            PbesExpr::Negation(expr) => write!(f, "(! {expr})"),
            PbesExpr::Binary { op, lhs, rhs } => write!(f, "({lhs} {op} {rhs})"),
            PbesExpr::Quantifier {
                quantifier,
                variables,
                body,
            } => write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body),
            PbesExpr::DataValExpr(data_expr) => write!(f, "val({data_expr})"),
        }
    }
}

impl fmt::Display for PbesExprBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PbesExprBinaryOp::Conjunction => write!(f, "&&"),
            PbesExprBinaryOp::Disjunction => write!(f, "||"),
            PbesExprBinaryOp::Implies => write!(f, "=>"),
        }
    }
}

impl fmt::Display for EqnSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "var")?;
        for decl in &self.variables {
            writeln!(f, "   {decl};")?;
        }

        writeln!(f, "eqn")?;
        for decl in &self.equations {
            writeln!(f, "   {decl};")?;
        }
        Ok(())
    }
}

impl fmt::Display for SortDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.identifier)?;

        if let Some(expr) = &self.expr {
            write!(f, " = {expr}")?;
        }

        Ok(())
    }
}

impl fmt::Display for ActDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.args.iter().format(", "))
        }
    }
}

impl fmt::Display for VarDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} : {}", self.identifier, self.sort)
    }
}

impl fmt::Display for EqnDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.condition {
            Some(condition) => write!(f, "{} -> {} = {}", condition, self.lhs, self.rhs),
            None => write!(f, "{} = {}", self.lhs, self.rhs),
        }
    }
}

impl fmt::Display for DataExprUnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataExprUnaryOp::Negation => write!(f, "!"),
            DataExprUnaryOp::Minus => write!(f, "-"),
            DataExprUnaryOp::Size => write!(f, "#"),
        }
    }
}

impl fmt::Display for DataExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DataExpr::EmptyList => write!(f, "[]"),
            DataExpr::EmptyBag => write!(f, "{{:}}"),
            DataExpr::EmptySet => write!(f, "{{}}"),
            DataExpr::List(expressions) => write!(f, "[{}]", expressions.iter().format(", ")),
            DataExpr::Bag(expressions) => write!(
                f,
                "{{ {} }}",
                expressions
                    .iter()
                    .format_with(", ", |e, f| f(&format_args!("{}: {}", e.expr, e.multiplicity)))
            ),
            DataExpr::Set(expressions) => write!(f, "{{ {} }}", expressions.iter().format(", ")),
            DataExpr::Id(identifier) => write!(f, "{identifier}"),
            DataExpr::Binary { op, lhs, rhs } => write!(f, "({lhs} {op} {rhs})"),
            DataExpr::Unary { op, expr } => write!(f, "({op} {expr})"),
            DataExpr::Bool(value) => write!(f, "{value}"),
            DataExpr::Quantifier { op, variables, body } => {
                write!(f, "({} {} . {})", op, variables.iter().format(", "), body)
            }
            DataExpr::Lambda { variables, body } => write!(f, "(lambda {} . {})", variables.iter().format(", "), body),
            DataExpr::Application { function, arguments } => {
                if arguments.is_empty() {
                    write!(f, "{function}")
                } else {
                    write!(f, "{}({})", function, arguments.iter().format(", "))
                }
            }
            DataExpr::Number(value) => write!(f, "{value}"),
            DataExpr::FunctionUpdate { expr, update } => write!(f, "{expr}[{update}]"),
            DataExpr::SetBagComp { variable, predicate } => write!(f, "{{ {variable} | {predicate} }}"),
            DataExpr::Whr { expr, assignments } => write!(f, "{} whr {} end", expr, assignments.iter().format(", ")),
        }
    }
}

impl fmt::Display for IdDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} : {}", self.identifier, self.sort)
    }
}

impl fmt::Display for DataExprUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.expr, self.update)
    }
}

impl fmt::Display for SortExpression {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SortExpression::Product { lhs, rhs } => write!(f, "({lhs} # {rhs})"),
            SortExpression::Function { domain, range } => write!(f, "({domain} -> {range})"),
            SortExpression::Reference(ident) => write!(f, "{ident}"),
            SortExpression::Simple(sort) => write!(f, "{sort}"),
            SortExpression::Complex(complex, inner) => write!(f, "{complex}({inner})"),
            SortExpression::Struct { inner } => {
                write!(f, "struct ")?;
                write!(f, "{}", inner.iter().format(" | "))
            }
        }
    }
}

impl fmt::Display for UntypedStateFrmSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}", self.data_specification)?;

        writeln!(f, "{}", self.formula)
    }
}

impl fmt::Display for StateFrmUnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateFrmUnaryOp::Minus => write!(f, "-"),
            StateFrmUnaryOp::Negation => write!(f, "!"),
        }
    }
}

impl fmt::Display for FixedPointOperator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FixedPointOperator::Greatest => write!(f, "nu"),
            FixedPointOperator::Least => write!(f, "mu"),
        }
    }
}

impl fmt::Display for StateFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateFrm::True => write!(f, "true"),
            StateFrm::False => write!(f, "false"),
            StateFrm::DataValExpr(expr) => write!(f, "val({expr})"),
            StateFrm::Id(identifier, args) => {
                if args.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, args.iter().format(", "))
                }
            }
            StateFrm::Unary { op, expr } => write!(f, "({op} {expr})"),
            StateFrm::Modality {
                operator,
                formula,
                expr,
            } => match operator {
                ModalityOperator::Box => write!(f, "[{formula}]{expr}"),
                ModalityOperator::Diamond => write!(f, "<{formula}>{expr}"),
            },
            StateFrm::Quantifier {
                quantifier,
                variables,
                body,
            } => {
                write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body)
            }
            StateFrm::Bound {
                bound: quantifier,
                variables,
                body,
            } => {
                write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body)
            }
            StateFrm::Binary { op, lhs, rhs } => {
                write!(f, "({lhs} {op} {rhs})")
            }
            StateFrm::FixedPoint {
                operator,
                variable,
                body,
            } => {
                write!(f, "({operator} {variable} . {body})")
            }
            StateFrm::Delay(expr) => write!(f, "delay@({expr})"),
            StateFrm::Yaled(expr) => write!(f, "yaled@({expr})"),
            StateFrm::DataValExprLeftMult(value, expr) => write!(f, "({value} * {expr})"),
            StateFrm::DataValExprRightMult(expr, value) => write!(f, "({expr} * {value})"),
        }
    }
}

impl fmt::Display for StateVarDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.arguments.iter().format(","))
        }
    }
}

impl fmt::Display for StateVarAssignment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} : {} = {}", self.identifier, self.sort, self.expr)
    }
}

impl fmt::Display for StateFrmOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateFrmOp::Implies => write!(f, "=>"),
            StateFrmOp::Conjunction => write!(f, "&&"),
            StateFrmOp::Disjunction => write!(f, "||"),
            StateFrmOp::Addition => write!(f, "+"),
        }
    }
}

impl fmt::Display for RegFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RegFrm::Action(action) => write!(f, "{action}"),
            RegFrm::Iteration(body) => write!(f, "({body})*"),
            RegFrm::Plus(body) => write!(f, "({body})+"),
            RegFrm::Choice { lhs, rhs } => write!(f, "({lhs} + {rhs})"),
            RegFrm::Sequence { lhs, rhs } => write!(f, "({lhs} . {rhs})"),
        }
    }
}

impl fmt::Display for DataExprBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DataExprBinaryOp::At => write!(f, "."),
            DataExprBinaryOp::Concat => write!(f, "++"),
            DataExprBinaryOp::Cons => write!(f, "|>"),
            DataExprBinaryOp::Equal => write!(f, "=="),
            DataExprBinaryOp::NotEqual => write!(f, "!="),
            DataExprBinaryOp::LessThan => write!(f, "<"),
            DataExprBinaryOp::LessEqual => write!(f, "<="),
            DataExprBinaryOp::GreaterThan => write!(f, ">"),
            DataExprBinaryOp::GreaterEqual => write!(f, ">="),
            DataExprBinaryOp::Conj => write!(f, "&&"),
            DataExprBinaryOp::Disj => write!(f, "||"),
            DataExprBinaryOp::Add => write!(f, "+"),
            DataExprBinaryOp::Subtract => write!(f, "-"),
            DataExprBinaryOp::Div => write!(f, "/"),
            DataExprBinaryOp::Implies => write!(f, "=>"),
            DataExprBinaryOp::In => write!(f, "in"),
            DataExprBinaryOp::IntDiv => write!(f, "div"),
            DataExprBinaryOp::Mod => write!(f, "mod"),
            DataExprBinaryOp::Multiply => write!(f, "*"),
            DataExprBinaryOp::Snoc => write!(f, "<|"),
        }
    }
}

impl fmt::Display for ActFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ActFrm::False => write!(f, "false"),
            ActFrm::True => write!(f, "true"),
            ActFrm::MultAct(action) => write!(f, "{action}"),
            ActFrm::Binary { op, lhs, rhs } => {
                write!(f, "({lhs}) {op} ({rhs})")
            }
            ActFrm::DataExprVal(expr) => write!(f, "val({expr})"),
            ActFrm::Quantifier {
                quantifier,
                variables,
                body,
            } => write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body),
            ActFrm::Negation(expr) => write!(f, "(!{expr})"),
        }
    }
}

impl fmt::Display for ActFrmBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ActFrmBinaryOp::Implies => write!(f, "=>"),
            ActFrmBinaryOp::Intersect => write!(f, "&&"),
            ActFrmBinaryOp::Union => write!(f, "||"),
        }
    }
}

impl fmt::Display for Bound {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Bound::Inf => write!(f, "inf"),
            Bound::Sum => write!(f, "sum"),
            Bound::Sup => write!(f, "sup"),
        }
    }
}

impl fmt::Display for MultiAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "tau")
        } else {
            write!(f, "{}", self.actions.iter().format("|"))
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "{}", self.id)
        } else {
            write!(f, "{}({})", self.id, self.args.iter().format(", "))
        }
    }
}

impl fmt::Display for Quantifier {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Quantifier::Exists => write!(f, "exists"),
            Quantifier::Forall => write!(f, "forall"),
        }
    }
}

impl fmt::Display for ConstructorDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}(", self.name)?;
            for (i, (name, sort)) in self.args.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                match name {
                    Some(name) => write!(f, "{name} : {sort}")?,
                    None => write!(f, "{sort}")?,
                }
            }
            write!(f, ")")?;

            if let Some(projection) = &self.projection {
                write!(f, "?{projection}")?;
            }

            Ok(())
        }
    }
}

impl fmt::Display for ProcDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.params.is_empty() {
            write!(f, "{} = {};", self.identifier, self.body)
        } else {
            write!(
                f,
                "{}({}) = {};",
                self.identifier,
                self.params.iter().format(", "),
                self.body
            )
        }
    }
}

impl fmt::Display for ProcExprBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcExprBinaryOp::Sequence => write!(f, "."),
            ProcExprBinaryOp::Choice => write!(f, "+"),
            ProcExprBinaryOp::Parallel => write!(f, "||"),
            ProcExprBinaryOp::LeftMerge => write!(f, "_||"),
            ProcExprBinaryOp::CommMerge => write!(f, "|"),
        }
    }
}

impl fmt::Display for ProcessExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessExpr::Id(identifier, assignments) => {
                if assignments.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, assignments.iter().format(", "))
                }
            }
            ProcessExpr::Action(identifier, data_exprs) => {
                if data_exprs.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, data_exprs.iter().format(", "))
                }
            }
            ProcessExpr::Delta => write!(f, "delta"),
            ProcessExpr::Tau => write!(f, "tau"),
            ProcessExpr::Sum { variables, operand } => {
                write!(f, "(sum {} . {})", variables.iter().format(", "), operand)
            }
            ProcessExpr::Dist {
                variables,
                expr,
                operand,
            } => write!(f, "(dist {} [{}] . {})", variables.iter().format(", "), expr, operand),
            ProcessExpr::Binary { op, lhs, rhs } => write!(f, "({lhs} {op} {rhs})"),
            ProcessExpr::Hide { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "hide({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExpr::Rename { renames, operand } => {
                if !renames.is_empty() {
                    write!(f, "rename({{{}}}, {})", renames.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExpr::Allow { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "allow({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExpr::Block { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "block({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExpr::Comm { comm, operand } => {
                if !comm.is_empty() {
                    write!(f, "comm({{{}}}, {})", comm.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExpr::Condition { condition, then, else_ } => {
                if let Some(else_) = else_ {
                    write!(f, "({condition}) -> ({then}) <> ({else_})")
                } else {
                    write!(f, "({condition}) -> ({then})")
                }
            }
            ProcessExpr::At { expr, operand } => write!(f, "({expr})@({operand})"),
        }
    }
}

impl fmt::Display for Comm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.from, self.to)
    }
}

impl fmt::Display for Rename {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.from, self.to)
    }
}

impl fmt::Display for MultiActionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "tau")
        } else {
            write!(f, "{}", self.actions.iter().format("|"))
        }
    }
}
