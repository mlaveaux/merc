use std::hash::Hash;

/// A complete mCRL2 process specification.
#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedProcessSpecification {
    pub data_specification: UntypedDataSpecification,
    pub global_variables: Vec<VarDecl>,
    pub action_declarations: Vec<ActDecl>,
    pub process_declarations: Vec<ProcDecl>,
    pub init: Option<ProcessExpr>,
}

/// An mCRL2 data specification.
#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedDataSpecification {
    pub sort_declarations: Vec<SortDecl>,
    pub constructor_declarations: Vec<IdDecl>,
    pub map_declarations: Vec<IdDecl>,
    pub equation_declarations: Vec<EqnSpec>,
}

impl UntypedDataSpecification {
    /// Returns true if the data specification is empty.
    pub fn is_empty(&self) -> bool {
        self.sort_declarations.is_empty()
            && self.constructor_declarations.is_empty()
            && self.map_declarations.is_empty()
            && self.equation_declarations.is_empty()
    }
}

/// An mCRL2 parameterised boolean equation system (PBES).
#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub struct UntypedPbes {
    pub data_specification: UntypedDataSpecification,
    pub global_variables: Vec<VarDecl>,
    pub equations: Vec<PbesEquation>,
    pub init: PropVarInst,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct PropVarDecl {
    pub identifier: String,
    pub parameters: Vec<VarDecl>,
    pub span: Span,
}

#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub struct PropVarInst {
    pub identifier: String,
    pub arguments: Vec<DataExpr>,
}

/// A declaration of an identifier with its sort.
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct IdDecl {
    /// Identifier being declared
    pub identifier: String,
    /// Sort expression for this identifier
    pub sort: SortExpression,
    /// Source location information
    pub span: Span,
}

/// Expression representing a sort (type).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum SortExpression {
    /// Product of two sorts (A # B)
    Product {
        lhs: Box<SortExpression>,
        rhs: Box<SortExpression>,
    },
    /// Function sort (A -> B)
    Function {
        domain: Box<SortExpression>,
        range: Box<SortExpression>,
    },
    Struct {
        inner: Vec<ConstructorDecl>,
    },
    /// Reference to a named sort    
    Reference(String),
    /// Built-in simple sort
    Simple(Sort),
    /// Parameterized complex sort
    Complex(ComplexSort, Box<SortExpression>),
}

/// Constructor declaration
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ConstructorDecl {
    pub name: String,
    pub args: Vec<(Option<String>, SortExpression)>,
    pub projection: Option<String>,
}

/// Built-in simple sorts.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum Sort {
    Bool,
    Pos,
    Int,
    Nat,
    Real,
}

/// Complex (parameterized) sorts.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum ComplexSort {
    List,
    Set,
    FSet,
    FBag,
    Bag,
}

/// Sort declaration
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct SortDecl {
    /// Sort identifier
    pub identifier: String,
    /// Sort expression (if structured)
    pub expr: Option<SortExpression>,
    /// Where the sort is defined
    pub span: Span,
}

/// Variable declaration
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct VarDecl {
    pub identifier: String,
    pub sort: SortExpression,
    pub span: Span,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct EqnSpec {
    pub variables: Vec<VarDecl>,
    pub equations: Vec<EqnDecl>,
}

/// Equation declaration
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct EqnDecl {
    pub condition: Option<DataExpr>,
    pub lhs: DataExpr,
    pub rhs: DataExpr,
    pub span: Span,
}

/// Action declaration
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ActDecl {
    pub identifier: String,
    pub args: Vec<SortExpression>,
    pub span: Span,
}

/// Process declaration
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ProcDecl {
    pub identifier: String,
    pub params: Vec<VarDecl>,
    pub body: ProcessExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExprUnaryOp {
    Negation,
    Minus,
    Size,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExprBinaryOp {
    Conj,
    Disj,
    Implies,
    Equal,
    NotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,
    Cons,
    Snoc,
    In,
    Concat,
    Add,
    Subtract,
    Div,
    IntDiv,
    Mod,
    Multiply,
    At,
}

/// Data expression
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExpr {
    Id(String),
    Number(String), // Is string because the number can be any size.
    Bool(bool),
    Application {
        function: Box<DataExpr>,
        arguments: Vec<DataExpr>,
    },
    EmptyList,
    List(Vec<DataExpr>),
    EmptySet,
    Set(Vec<DataExpr>),
    EmptyBag,
    Bag(Vec<BagElement>),
    SetBagComp {
        variable: VarDecl,
        predicate: Box<DataExpr>,
    },
    Lambda {
        variables: Vec<VarDecl>,
        body: Box<DataExpr>,
    },
    Quantifier {
        op: Quantifier,
        variables: Vec<VarDecl>,
        body: Box<DataExpr>,
    },
    Unary {
        op: DataExprUnaryOp,
        expr: Box<DataExpr>,
    },
    Binary {
        op: DataExprBinaryOp,
        lhs: Box<DataExpr>,
        rhs: Box<DataExpr>,
    },
    FunctionUpdate {
        expr: Box<DataExpr>,
        update: Box<DataExprUpdate>,
    },
    Whr {
        expr: Box<DataExpr>,
        assignments: Vec<Assignment>,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct BagElement {
    pub expr: DataExpr,
    pub multiplicity: DataExpr,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct DataExprUpdate {
    pub expr: DataExpr,
    pub update: DataExpr,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Assignment {
    pub identifier: String,
    pub expr: DataExpr,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ProcExprBinaryOp {
    Sequence,
    Choice,
    Parallel,
    LeftMerge,
    CommMerge,
}

/// Process expression
#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ProcessExpr {
    Id(String, Vec<Assignment>),
    Action(String, Vec<DataExpr>),
    Delta,
    Tau,
    Sum {
        variables: Vec<VarDecl>,
        operand: Box<ProcessExpr>,
    },
    Dist {
        variables: Vec<VarDecl>,
        expr: DataExpr,
        operand: Box<ProcessExpr>,
    },
    Binary {
        op: ProcExprBinaryOp,
        lhs: Box<ProcessExpr>,
        rhs: Box<ProcessExpr>,
    },
    Hide {
        actions: Vec<String>,
        operand: Box<ProcessExpr>,
    },
    Rename {
        renames: Vec<Rename>,
        operand: Box<ProcessExpr>,
    },
    Allow {
        actions: Vec<MultiActionLabel>,
        operand: Box<ProcessExpr>,
    },
    Block {
        actions: Vec<String>,
        operand: Box<ProcessExpr>,
    },
    Comm {
        comm: Vec<Comm>,
        operand: Box<ProcessExpr>,
    },
    Condition {
        condition: DataExpr,
        then: Box<ProcessExpr>,
        else_: Option<Box<ProcessExpr>>,
    },
    At {
        expr: Box<ProcessExpr>,
        operand: DataExpr,
    },
}

/// Communication action
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct CommAction {
    pub inputs: Vec<String>,
    pub output: String,
    pub span: Span,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct UntypedStateFrmSpec {
    pub data_specification: UntypedDataSpecification,
    pub action_declarations: Vec<ActDecl>,
    pub formula: StateFrm,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum StateFrmUnaryOp {
    Minus,
    Negation,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum StateFrmOp {
    Addition,
    Implies,
    Disjunction,
    Conjunction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FixedPointOperator {
    Least,
    Greatest,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StateVarDecl {
    pub identifier: String,
    pub arguments: Vec<StateVarAssignment>,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StateVarAssignment {
    pub identifier: String,
    pub sort: SortExpression,
    pub expr: DataExpr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ModalityOperator {
    Diamond,
    Box,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum StateFrm {
    True,
    False,
    Delay(DataExpr),
    Yaled(DataExpr),
    Id(String, Vec<DataExpr>),
    DataValExprLeftMult(DataExpr, Box<StateFrm>),
    DataValExprRightMult(Box<StateFrm>, DataExpr),
    DataValExpr(DataExpr),
    Modality {
        operator: ModalityOperator,
        formula: RegFrm,
        expr: Box<StateFrm>,
    },
    Unary {
        op: StateFrmUnaryOp,
        expr: Box<StateFrm>,
    },
    Binary {
        op: StateFrmOp,
        lhs: Box<StateFrm>,
        rhs: Box<StateFrm>,
    },
    Quantifier {
        quantifier: Quantifier,
        variables: Vec<VarDecl>,
        body: Box<StateFrm>,
    },
    Bound {
        bound: Bound,
        variables: Vec<VarDecl>,
        body: Box<StateFrm>,
    },
    FixedPoint {
        operator: FixedPointOperator,
        variable: StateVarDecl,
        body: Box<StateFrm>,
    },
}

/// Represents a multi action label `a | b | c ...`.
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct MultiActionLabel {
    pub actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Action {
    pub id: String,
    pub args: Vec<DataExpr>,
}

#[derive(Clone, Debug, Eq)]
pub struct MultiAction {
    pub actions: Vec<Action>,
}

impl PartialEq for MultiAction {
    fn eq(&self, other: &Self) -> bool {
        // Check whether both multi-actions contain the same actions
        if self.actions.len() != other.actions.len() {
            return false;
        }

        // Map every action onto the other, equal length means they must be the same.
        for action in self.actions.iter() {
            if !other.actions.contains(action) {
                return false;
            }
        }

        true
    }
}

impl Hash for MultiAction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut actions = self.actions.clone();
        // Sort the action ids to ensure that the hash is independent of the order.
        actions.sort();
        for action in actions {
            action.hash(state);
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum Quantifier {
    Exists,
    Forall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ActFrmBinaryOp {
    Implies,
    Union,
    Intersect,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ActFrm {
    True,
    False,
    MultAct(MultiAction),
    DataExprVal(DataExpr),
    Negation(Box<ActFrm>),
    Quantifier {
        quantifier: Quantifier,
        variables: Vec<VarDecl>,
        body: Box<ActFrm>,
    },
    Binary {
        op: ActFrmBinaryOp,
        lhs: Box<ActFrm>,
        rhs: Box<ActFrm>,
    },
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum PbesExpr {
    DataValExpr(DataExpr),
    PropVarInst(PropVarInst),
    Quantifier {
        quantifier: Quantifier,
        variables: Vec<VarDecl>,
        body: Box<PbesExpr>,
    },
    Negation(Box<PbesExpr>),
    Binary {
        op: PbesExprBinaryOp,
        lhs: Box<PbesExpr>,
        rhs: Box<PbesExpr>,
    },
    True,
    False,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum Eq {
    EqInf,
    EqnInf,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum Condition {
    Condsm,
    Condeq,
}

// TODO: What should this be called?
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bound {
    Inf,
    Sup,
    Sum,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum PresExpr {
    DataValExpr(DataExpr),
    PropVarInst(PropVarInst),
    LeftConstantMultiply {
        constant: DataExpr,
        expr: Box<PresExpr>,
    },
    Bound {
        op: Bound,
        variables: Vec<VarDecl>,
        expr: Box<PresExpr>,
    },
    Equal {
        eq: Eq,
        body: Box<PbesExpr>,
    },
    Condition {
        condition: Condition,
        lhs: Box<PresExpr>,
        then: Box<PresExpr>,
        else_: Box<PresExpr>,
    },
    Negation(Box<PresExpr>),
    Binary {
        op: PbesExprBinaryOp,
        lhs: Box<PresExpr>,
        rhs: Box<PresExpr>,
    },
    True,
    False,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct PbesEquation {
    pub operator: FixedPointOperator,
    pub variable: PropVarDecl,
    pub formula: PbesExpr,
    pub span: Span,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum PbesExprBinaryOp {
    Implies,
    Disjunction,
    Conjunction,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum RegFrm {
    Action(ActFrm),
    Iteration(Box<RegFrm>),
    Plus(Box<RegFrm>),
    Sequence { lhs: Box<RegFrm>, rhs: Box<RegFrm> },
    Choice { lhs: Box<RegFrm>, rhs: Box<RegFrm> },
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Comm {
    pub from: MultiActionLabel,
    pub to: String,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct UntypedActionRenameSpec {
    pub data_specification: UntypedDataSpecification,
    pub action_declarations: Vec<ActDecl>,
    pub rename_declarations: Vec<ActionRenameDecl>,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ActionRenameDecl {
    pub variables_specification: Vec<VarDecl>,
    pub rename_rule: ActionRenameRule,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct ActionRenameRule {
    pub condition: Option<DataExpr>,
    pub action: Action,
    pub rhs: ActionRHS,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ActionRHS {
    Tau,
    Delta,
    Action(Action),
}

/// Source location information, spanning from start to end in the source text.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<pest::Span<'_>> for Span {
    fn from(span: pest::Span) -> Self {
        Span {
            start: span.start(),
            end: span.end(),
        }
    }
}
