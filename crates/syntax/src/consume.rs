use std::iter;

use itertools::Itertools;
use pest::error::ErrorVariant;

use merc_pest_consume::Error;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;

use crate::ActDecl;
use crate::ActFrm;
use crate::Action;
use crate::ActionName;
use crate::ActionRHS;
use crate::ActionRenameDecl;
use crate::ActionRenameRule;
use crate::Assignment;
use crate::AssignmentData;
use crate::BagElement;
use crate::CommExpr;
use crate::ComplexSort;
use crate::Condition;
use crate::ConstructorDecl;
use crate::ConstructorId;
use crate::DataExpr;
use crate::DataExprKind;
use crate::DataExprUnaryOp;
use crate::DataExprUpdate;
use crate::Eq;
use crate::EqnDecl;
use crate::EqnSpec;
use crate::EqnSpecData;
use crate::FixedPointOperator;
use crate::IdDecl;
use crate::MapId;
use crate::Mcrl2Parser;
use crate::MultiAction;
use crate::MultiActionLabel;
use crate::PbesEquation;
use crate::PbesExpr;
use crate::PresEquation;
use crate::PresExpr;
use crate::PresExprKind;
use crate::ProcDecl;
use crate::ProcessExpr;
use crate::ProcessExprKind;
use crate::PropVarDecl;
use crate::PropVarInst;
use crate::PropVarInstData;
use crate::RegFrm;
use crate::Rename;
use crate::Rule;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::Spanned;
use crate::StateFrm;
use crate::StateFrmKind;
use crate::StateVarAssignment;
use crate::StateVarDecl;
use crate::TypeVarDecl;
use crate::UntypedActionRenameSpec;
use crate::UntypedDataSpecification;
use crate::UntypedPbes;
use crate::UntypedPres;
use crate::UntypedProcessSpecification;
use crate::UntypedStateFrmSpec;
use crate::bind_type_vars;
use crate::parse_actfrm;
use crate::parse_dataexpr;
use crate::parse_pbesexpr;
use crate::parse_presexpr;
use crate::parse_process_expr;
use crate::parse_regfrm;
use crate::parse_sortexpr;
use crate::parse_sortexpr_primary;
use crate::parse_statefrm;

/// The error type produced while consuming the parse tree.
pub(crate) type ParseResult<T> = std::result::Result<T, Error<Rule>>;
pub(crate) type ParseNode<'i> = merc_pest_consume::Node<'i, Rule, ()>;

/// Consumes the pest parse tree into syntax tree nodes.
///
/// Private functions are only called from `match_nodes!` arms elsewhere in this module, which
/// apply a consume function to a parse node's children once their rules match the arm's shape.
///
/// `pub(crate)` functions are also called from the Pratt parsers in `precedence.rs` to consume
/// operands while building the syntax tree with the correct priority and associativity.
#[merc_pest_consume::parser]
impl Mcrl2Parser {
    // Although these are not public, they are the main entry points for consuming the parse tree.
    pub(crate) fn MCRL2Spec(spec: ParseNode) -> ParseResult<UntypedProcessSpecification> {
        let mut action_declarations = Vec::new();
        let mut map_declarations = Vec::new();
        let mut constructor_declarations = Vec::new();
        let mut equation_declarations = Vec::new();
        let mut global_variables = Vec::new();
        let mut process_declarations = Vec::new();
        let mut sort_declarations = Vec::new();
        let mut type_var_declarations = Vec::new();

        let mut init = None;

        for child in spec.into_children() {
            match child.as_rule() {
                Rule::ActSpec => {
                    action_declarations.extend(Mcrl2Parser::ActSpec(child)?);
                }
                Rule::ConsSpec => {
                    constructor_declarations.append(&mut Mcrl2Parser::ConsSpec(child)?);
                }
                Rule::MapSpec => {
                    map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
                }
                Rule::GlobVarSpec => {
                    global_variables.append(&mut Mcrl2Parser::GlobVarSpec(child)?);
                }
                Rule::EqnSpec => {
                    equation_declarations.append(&mut Mcrl2Parser::EqnSpec(child)?);
                }
                Rule::ProcSpec => {
                    process_declarations.append(&mut Mcrl2Parser::ProcSpec(child)?);
                }
                Rule::SortSpec => {
                    sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
                }
                Rule::TypeVarSpec => {
                    type_var_declarations.append(&mut Mcrl2Parser::TypeVarSpec(child)?);
                }
                Rule::Init => {
                    if init.is_some() {
                        return Err(Error::new_from_span(
                            ErrorVariant::CustomError {
                                message: "Multiple init expressions are not allowed".to_string(),
                            },
                            child.as_span(),
                        ));
                    }

                    init = Some(Mcrl2Parser::Init(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        let mut data_specification = UntypedDataSpecification {
            map_declarations,
            constructor_declarations,
            equation_declarations,
            sort_declarations,
            type_var_declarations,
        };
        bind_type_vars(&mut data_specification);

        Ok(UntypedProcessSpecification {
            data_specification,
            global_variables,
            action_declarations,
            process_declarations,
            init,
        })
    }

    pub fn PbesSpec(spec: ParseNode) -> ParseResult<UntypedPbes> {
        let mut data_specification = None;
        let mut global_variables = None;
        let mut equations = None;
        let mut init = None;

        let span = spec.as_span();
        for child in spec.into_children() {
            match child.as_rule() {
                Rule::DataSpecBody => {
                    data_specification = Some(Mcrl2Parser::DataSpecBody(child)?);
                }
                Rule::GlobVarSpec => {
                    global_variables = Some(Mcrl2Parser::GlobVarSpec(child)?);
                }
                Rule::PbesEqnSpec => {
                    equations = Some(Mcrl2Parser::PbesEqnSpec(child)?);
                }
                Rule::PbesInit => {
                    init = Some(Mcrl2Parser::PbesInit(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        Ok(UntypedPbes {
            data_specification: data_specification.unwrap_or_default(),
            global_variables: global_variables.unwrap_or_default(),
            equations: equations.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PBES requires a (possibly empty) pbes equation section".to_string(),
                    },
                    span,
                )
            })?,
            init: init.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PBES requires an init declaration".to_string(),
                    },
                    span,
                )
            })?,
        })
    }

    fn PbesInit(init: ParseNode) -> ParseResult<PropVarInst> {
        match_nodes!(init.into_children();
            [PropVarInst(inst)] => {
                Ok(inst)
            }
        )
    }

    fn PbesEqnSpec(spec: ParseNode) -> ParseResult<Vec<PbesEquation>> {
        match_nodes!(spec.into_children();
            [PbesEqnDecl(equations)..] => {
                Ok(equations.collect())
            },
        )
    }

    fn PbesEqnDecl(decl: ParseNode) -> ParseResult<PbesEquation> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [FixedPointOperator(operator), PropVarDecl(variable), PbesExpr(formula)] => {
                Ok(PbesEquation {
                    operator,
                    variable,
                    formula,
                    span: span.into(),
                })
            },
        )
    }

    fn FixedPointOperator(op: ParseNode) -> ParseResult<FixedPointOperator> {
        match op.into_children().next().unwrap().as_rule() {
            Rule::FixedPointMu => Ok(FixedPointOperator::Least),
            Rule::FixedPointNu => Ok(FixedPointOperator::Greatest),
            x => unimplemented!("This is not a fixed point operator: {:?}", x),
        }
    }

    fn PropVarDecl(decl: ParseNode) -> ParseResult<PropVarDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [Id(identifier), VarsDeclList(params)] => {
                Ok(PropVarDecl {
                    identifier,
                    parameters: params,
                    span: span.into(),
                })
            },
            [Id(identifier)] => {
                let span = identifier.span.clone();
                Ok(PropVarDecl {
                    identifier,
                    parameters: Vec::new(),
                    span,
                })
            }
        )
    }

    pub(crate) fn PropVarInst(inst: ParseNode) -> ParseResult<PropVarInst> {
        let span = inst.as_span();
        match_nodes!(inst.into_children();
            [Id(identifier)] => {
                Ok(PropVarInstData {
                    identifier,
                    arguments: Vec::new(),
                }.spanned(span.into()))
            },
            [Id(identifier), DataExprList(arguments)] => {
                Ok(PropVarInstData {
                    identifier,
                    arguments,
                }.spanned(span.into()))
            }
        )
    }

    fn PbesExpr(expr: ParseNode) -> ParseResult<PbesExpr> {
        parse_pbesexpr(expr.children().as_pairs().clone())
    }

    pub fn PresSpec(spec: ParseNode) -> ParseResult<UntypedPres> {
        let mut data_specification = None;
        let mut global_variables = None;
        let mut equations = None;
        let mut init = None;

        let span = spec.as_span();
        for child in spec.into_children() {
            match child.as_rule() {
                Rule::DataSpecBody => {
                    data_specification = Some(Mcrl2Parser::DataSpecBody(child)?);
                }
                Rule::GlobVarSpec => {
                    global_variables = Some(Mcrl2Parser::GlobVarSpec(child)?);
                }
                Rule::PresEqnSpec => {
                    equations = Some(Mcrl2Parser::PresEqnSpec(child)?);
                }
                Rule::PbesInit => {
                    init = Some(Mcrl2Parser::PbesInit(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        Ok(UntypedPres {
            data_specification: data_specification.unwrap_or_default(),
            global_variables: global_variables.unwrap_or_default(),
            equations: equations.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PRES requires a (possibly empty) pres equation section".to_string(),
                    },
                    span,
                )
            })?,
            init: init.ok_or_else(|| {
                Error::new_from_span(
                    ErrorVariant::CustomError {
                        message: "A PRES requires an init declaration".to_string(),
                    },
                    span,
                )
            })?,
        })
    }

    fn PresEqnSpec(spec: ParseNode) -> ParseResult<Vec<PresEquation>> {
        match_nodes!(spec.into_children();
            [PresEqnDecl(equations)..] => {
                Ok(equations.collect())
            },
        )
    }

    fn PresEqnDecl(decl: ParseNode) -> ParseResult<PresEquation> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [FixedPointOperator(operator), PropVarDecl(variable), PresExpr(formula)] => {
                Ok(PresEquation {
                    operator,
                    variable,
                    formula,
                    span: span.into(),
                })
            },
        )
    }

    fn PresExpr(expr: ParseNode) -> ParseResult<PresExpr> {
        parse_presexpr(expr.children().as_pairs().clone())
    }

    fn ActSpec(spec: ParseNode) -> ParseResult<Vec<ActDecl>> {
        match_nodes!(spec.into_children();
            [ActDecl(decls)..] => {
                Ok(decls.flatten().collect())
            },
        )
    }

    fn ActDecl(decl: ParseNode) -> ParseResult<Vec<ActDecl>> {
        // Shared by every identifier in the `a, b: Nat` group below: there is no narrower
        // per-name "whole declaration" extent than the group itself.
        let span: Span = decl.as_span().into();
        match_nodes!(decl.into_children();
            [IdList(identifiers)] => {
                Ok(identifiers.into_iter().map(|(name, id_span)| ActDecl {
                    identifier: ActionName { node: name, span: id_span },
                    args: Vec::new(),
                    span: span.clone(),
                }).collect())
            },
            [IdList(identifiers), SortProduct(args)] => {
                Ok(identifiers.into_iter().map(|(name, id_span)| ActDecl {
                    identifier: ActionName { node: name, span: id_span },
                    args: args.clone(),
                    span: span.clone(),
                }).collect())
            },
        )
    }

    fn SortProduct(sort: ParseNode) -> ParseResult<Vec<SortExpression>> {
        let mut iter = sort.into_children();

        // An expression of the shape SortExprPrimary ~ (SortExprProduct ~ SortExprPrimary)*
        let mut result = vec![parse_sortexpr_primary(iter.next().unwrap().as_pair().clone())?];

        for mut chunk in &iter.chunks(2) {
            if chunk.next().unwrap().as_rule() == Rule::SortExprProduct {
                let sort = parse_sortexpr_primary(chunk.next().unwrap().as_pair().clone())?;
                result.push(sort);
            }
        }

        Ok(result)
    }

    fn GlobVarSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(spec.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            }
        )
    }

    fn SortExprPrimary(sort: ParseNode) -> ParseResult<SortExpression> {
        parse_sortexpr(sort.children().as_pairs().clone())
    }

    pub(crate) fn DataSpec(spec: ParseNode) -> ParseResult<UntypedDataSpecification> {
        // `DataSpecBody` always matches (its repetition allows zero declarations), so it is
        // always the first child, ahead of `EOI`.
        let Some(child) = spec.into_children().next() else {
            return Ok(UntypedDataSpecification::default());
        };

        match child.as_rule() {
            Rule::DataSpecBody => Mcrl2Parser::DataSpecBody(child),
            rule => unimplemented!("Unexpected rule: {:?}", rule),
        }
    }

    pub(crate) fn DataSpecBody(spec: ParseNode) -> ParseResult<UntypedDataSpecification> {
        let mut map_declarations = Vec::new();
        let mut equation_declarations = Vec::new();
        let mut constructor_declarations = Vec::new();
        let mut sort_declarations = Vec::new();
        let mut type_var_declarations = Vec::new();

        for child in spec.into_children() {
            match child.as_rule() {
                Rule::ConsSpec => {
                    constructor_declarations.append(&mut Mcrl2Parser::ConsSpec(child)?);
                }
                Rule::MapSpec => {
                    map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
                }
                Rule::EqnSpec => {
                    equation_declarations.append(&mut Mcrl2Parser::EqnSpec(child)?);
                }
                Rule::SortSpec => {
                    sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
                }
                Rule::TypeVarSpec => {
                    type_var_declarations.append(&mut Mcrl2Parser::TypeVarSpec(child)?);
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        let mut data_specification = UntypedDataSpecification {
            map_declarations,
            equation_declarations,
            constructor_declarations,
            sort_declarations,
            type_var_declarations,
        };
        bind_type_vars(&mut data_specification);

        Ok(data_specification)
    }

    pub fn ActionRenameSpec(spec: ParseNode) -> ParseResult<UntypedActionRenameSpec> {
        let mut map_declarations = Vec::new();
        let mut equation_declarations = Vec::new();
        let mut constructor_declarations = Vec::new();
        let mut sort_declarations = Vec::new();
        let mut type_var_declarations = Vec::new();
        let mut action_declarations = Vec::new();
        let mut rename_declarations = Vec::new();

        for child in spec.into_children() {
            match child.as_rule() {
                Rule::ConsSpec => {
                    constructor_declarations.append(&mut Mcrl2Parser::ConsSpec(child)?);
                }
                Rule::MapSpec => {
                    map_declarations.append(&mut Mcrl2Parser::MapSpec(child)?);
                }
                Rule::EqnSpec => {
                    equation_declarations.append(&mut Mcrl2Parser::EqnSpec(child)?);
                }
                Rule::SortSpec => {
                    sort_declarations.append(&mut Mcrl2Parser::SortSpec(child)?);
                }
                Rule::TypeVarSpec => {
                    type_var_declarations.append(&mut Mcrl2Parser::TypeVarSpec(child)?);
                }
                Rule::ActSpec => {
                    action_declarations.append(&mut Mcrl2Parser::ActSpec(child)?);
                }
                Rule::ActionRenameRuleSpec => {
                    rename_declarations.append(&mut Mcrl2Parser::ActionRenameRuleSpec(child)?)
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        let mut data_specification = UntypedDataSpecification {
            map_declarations,
            equation_declarations,
            constructor_declarations,
            sort_declarations,
            type_var_declarations,
        };
        bind_type_vars(&mut data_specification);

        Ok(UntypedActionRenameSpec {
            data_specification,
            action_declarations,
            rename_declarations,
        })
    }

    pub(crate) fn StateFrmId(id: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = id.as_span().into();
        match_nodes!(id.into_children();
            [Id(identifier)] => {
                Ok(StateFrmKind::Id(identifier.node, Vec::new()).spanned(span))
            },
            [Id(identifier), DataExprList(expressions)] => {
                Ok(StateFrmKind::Id(identifier.node, expressions).spanned(span))
            },
        )
    }

    fn MapSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl<MapId>>> {
        match_nodes!(spec.into_children();
            [IdsDecl(decls)..] => {
                Ok(decls.flatten().map(IdDecl::retag).collect())
            }
        )
    }

    fn SortSpec(spec: ParseNode) -> ParseResult<Vec<SortDecl>> {
        match_nodes!(spec.into_children();
            [SortDecl(decls)..] => {
                Ok(decls.flatten().collect())
            }
        )
    }

    fn SortDecl(decl: ParseNode) -> ParseResult<Vec<SortDecl>> {
        match_nodes!(decl.into_children();
            // The alias form (`sort A = Bool;`) always names exactly one sort per node.
            [IdAt(identifier), SortExpr(expr)] => {
                Ok(vec![SortDecl::new(identifier.node, Some(expr), identifier.span)])
            },
            // `sort A, B, C;`: each gets its own precise identifier span (see `IdList`).
            [IdList(ids)] => {
                Ok(ids.into_iter().map(|(identifier, span)| SortDecl::new(identifier, None, span)).collect())
            },
        )
    }

    fn TypeVarSpec(spec: ParseNode) -> ParseResult<Vec<TypeVarDecl>> {
        match_nodes!(spec.into_children();
            [IdList(ids)..] => {
                Ok(ids.flatten().map(|(identifier, span)| TypeVarDecl::new(identifier, span)).collect())
            }
        )
    }

    fn ConsSpec(spec: ParseNode) -> ParseResult<Vec<IdDecl<ConstructorId>>> {
        match_nodes!(spec.into_children();
            [IdsDecl(decls)..] => {
                Ok(decls.flatten().map(IdDecl::retag).collect())
            }
        )
    }

    fn Init(init: ParseNode) -> ParseResult<ProcessExpr> {
        match_nodes!(init.into_children();
            [ProcExpr(expr)] => {
                Ok(expr)
            }
        )
    }

    fn ProcSpec(spec: ParseNode) -> ParseResult<Vec<ProcDecl>> {
        match_nodes!(spec.into_children();
            [ProcDecl(decls)..] => {
                Ok(decls.collect())
            },
        )
    }

    fn ProcDecl(decl: ParseNode) -> ParseResult<ProcDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [Id(identifier), VarsDeclList(params), ProcExpr(body)] => {
                Ok(ProcDecl {
                    identifier,
                    params,
                    body,
                    span: span.into(),
                })
            },
            [Id(identifier), ProcExpr(body)] => {
                Ok(ProcDecl {
                    identifier,
                    params: Vec::new(),
                    body,
                    span: span.into(),
                })
            }
        )
    }

    pub(crate) fn ProcExprAt(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataExprUnit(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn StateFrmExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmMu(input: ParseNode) -> ParseResult<StateVarDecl> {
        match_nodes!(input.into_children();
            [StateVarDecl(variable)] => {
                Ok(variable)
            },
        )
    }

    pub(crate) fn StateFrmNu(input: ParseNode) -> ParseResult<StateVarDecl> {
        match_nodes!(input.into_children();
            [StateVarDecl(variable)] => {
                Ok(variable)
            },
        )
    }

    pub(crate) fn ActFrmExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn ActFrmForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn DataExpr(expr: ParseNode) -> ParseResult<DataExpr> {
        parse_dataexpr(expr.children().as_pairs().clone())
    }

    pub(crate) fn DataExprUnit(expr: ParseNode) -> ParseResult<DataExpr> {
        parse_dataexpr(expr.children().as_pairs().clone())
    }

    pub(crate) fn DataValExpr(expr: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn DataExprUpdate(expr: ParseNode) -> ParseResult<DataExprUpdate> {
        match_nodes!(expr.into_children();
            [DataExpr(expr), DataExpr(update)] => {
                Ok(DataExprUpdate { expr, update })
            },
        )
    }

    pub(crate) fn DataExprApplication(expr: ParseNode) -> ParseResult<Vec<DataExpr>> {
        match_nodes!(expr.into_children();
            [DataExprList(expressions)] => {
                Ok(expressions)
            },
        )
    }

    pub(crate) fn DataExprWhr(expr: ParseNode) -> ParseResult<Vec<Assignment>> {
        match_nodes!(expr.into_children();
            [AssignmentList(assignments)] => {
                Ok(assignments)
            },
        )
    }

    pub(crate) fn AssignmentList(assignments: ParseNode) -> ParseResult<Vec<Assignment>> {
        match_nodes!(assignments.into_children();
            [Assignment(assignment)] => {
                Ok(vec![assignment])
            },
            [Assignment(assignment)..] => {
                Ok(assignment.collect())
            },
        )
    }

    pub(crate) fn Assignment(assignment: ParseNode) -> ParseResult<Assignment> {
        match_nodes!(assignment.into_children();
            [IdAt(identifier), DataExpr(expr)] => {
                Ok(AssignmentData { identifier: identifier.node, expr, id: None }.spanned(identifier.span))
            },
        )
    }

    pub(crate) fn DataExprSize(expr: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = expr.as_span().into();
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(DataExprKind::Unary { op: DataExprUnaryOp::Size, expr: Box::new(expr) }.spanned(span))
            },
        )
    }

    fn DataExprList(expr: ParseNode) -> ParseResult<Vec<DataExpr>> {
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(vec![expr])
            },
            [DataExpr(expr)..] => {
                Ok(expr.collect())
            },
        )
    }

    fn VarSpec(vars: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(vars.into_children();
            [VarsDeclList(ids)..] => {
                Ok(ids.flatten().collect())
            },
        )
    }

    pub(crate) fn VarsDeclList(vars: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(vars.into_children();
            [VarsDecl(decl)..] => {
                Ok(decl.flatten().collect())
            },
        )
    }

    fn VarsDecl(decl: ParseNode) -> ParseResult<Vec<IdDecl>> {
        let mut vars = Vec::new();

        match_nodes!(decl.into_children();
            [IdList(identifiers), SortExpr(sort)] => {
                for (id, span) in identifiers {
                    vars.push(IdDecl::new(id, sort.clone(), span));
                }
            },
        );

        Ok(vars)
    }

    pub(crate) fn SortExpr(expr: ParseNode) -> ParseResult<SortExpression> {
        parse_sortexpr(expr.children().as_pairs().clone())
    }

    pub(crate) fn Id(identifier: ParseNode) -> ParseResult<Spanned<String>> {
        Ok(Spanned {
            node: identifier.as_str().to_string(),
            span: identifier.as_span().into(),
        })
    }

    pub(crate) fn IdAt(identifier: ParseNode) -> ParseResult<Spanned<String>> {
        Ok(Spanned {
            node: identifier.as_str().to_string(),
            span: identifier.as_span().into(),
        })
    }

    pub(crate) fn IdList(identifiers: ParseNode) -> ParseResult<Vec<(String, Span)>> {
        Ok(identifiers
            .into_children()
            .map(|node| (node.as_str().to_string(), node.as_span().into()))
            .collect())
    }

    fn IdInfix(identifier: ParseNode) -> ParseResult<String> {
        Ok(identifier.as_str().to_string())
    }

    fn IdInfixList(identifiers: ParseNode) -> ParseResult<Vec<(String, Span)>> {
        Ok(identifiers
            .into_children()
            .map(|node| (node.as_str().to_string(), node.as_span().into()))
            .collect())
    }

    // Complex sorts
    pub(crate) fn SortExprList(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::List,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprSet(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::Set,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprBag(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::Bag,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprFSet(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::FSet,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprFBag(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::FBag,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprStruct(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        match_nodes!(inner.into_children();
            [ConstrDeclList(inner)] => {
                Ok(SortExpressionKind::Struct { inner }.spanned(span))
            },
        )
    }

    pub(crate) fn ConstrDeclList(input: ParseNode) -> ParseResult<Vec<ConstructorDecl>> {
        match_nodes!(input.into_children();
            [ConstrDecl(decl)..] => {
                Ok(decl.collect())
            },
        )
    }

    pub(crate) fn ProjDeclList(input: ParseNode) -> ParseResult<Vec<(Option<Spanned<String>>, SortExpression)>> {
        match_nodes!(input.into_children();
            [ProjDecl(decl)..] => {
                Ok(decl.collect())
            },
        )
    }

    // `ConstrDecl = { IdAt ~ ( "(" ~ ProjDeclList ~ ")" )? ~ ( "?" ~ IdAt )? }`: one arm per
    // combination of the two optional groups. The leading name and the trailing recogniser each
    // keep their own span.
    pub(crate) fn ConstrDecl(input: ParseNode) -> ParseResult<ConstructorDecl> {
        match_nodes!(input.into_children();
            [IdAt(name)] => {
                Ok(ConstructorDecl { name, args: Vec::new(), projection: None })
            },
            [IdAt(name), ProjDeclList(args)] => {
                Ok(ConstructorDecl { name, args, projection: None })
            },
            [IdAt(name), IdAt(projection)] => {
                Ok(ConstructorDecl { name, args: Vec::new(), projection: Some(projection) })
            },
            [IdAt(name), ProjDeclList(args), IdAt(projection)] => {
                Ok(ConstructorDecl { name, args, projection: Some(projection) })
            },
        )
    }

    pub(crate) fn ProjDecl(input: ParseNode) -> ParseResult<(Option<Spanned<String>>, SortExpression)> {
        match_nodes!(input.into_children();
            [SortExpr(sort)] => {
                Ok((None, sort))
            },
            [Id(name), SortExpr(sort)] => {
                Ok((Some(name), sort))
            },
        )
    }

    pub(crate) fn DataExprListEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [DataExprList(expressions)] => {
                Ok(DataExprKind::List(expressions).spanned(span))
            },
        )
    }

    pub(crate) fn DataExprBagEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [BagEnumEltList(elements)] => {
                Ok(DataExprKind::Bag(elements).spanned(span))
            },
        )
    }

    fn BagEnumEltList(input: ParseNode) -> ParseResult<Vec<BagElement>> {
        match_nodes!(input.into_children();
            [BagEnumElt(elements)..] => {
                Ok(elements.collect())
            },
        )
    }

    fn BagEnumElt(input: ParseNode) -> ParseResult<BagElement> {
        match_nodes!(input.into_children();
            [DataExpr(expr), DataExpr(multiplicity)] => {
                Ok(BagElement { expr, multiplicity })
            },
        )
    }

    pub(crate) fn DataExprSetEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [DataExprList(expressions)] => {
                Ok(DataExprKind::Set(expressions).spanned(span))
            },
        )
    }

    pub(crate) fn DataExprSetBagComp(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [VarDecl(variable), DataExpr(predicate)] => {
                Ok(DataExprKind::SetBagComp { variable, predicate: Box::new(predicate) }.spanned(span))
            },
        )
    }

    pub(crate) fn Number(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        Ok(DataExprKind::Number(input.as_str().into()).spanned(span))
    }

    fn VarDecl(decl: ParseNode) -> ParseResult<IdDecl> {
        match_nodes!(decl.into_children();
            [IdAt(identifier), SortExpr(sort)] => {
                Ok(IdDecl::new(identifier.node, sort, identifier.span))
            },
        )
    }

    pub(crate) fn DataExprLambda(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn DataExprForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn DataExprExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn ActFrm(input: ParseNode) -> ParseResult<ActFrm> {
        parse_actfrm(input.children().as_pairs().clone())
    }

    pub(crate) fn ActIdSet(actions: ParseNode) -> ParseResult<Vec<ActionName>> {
        match_nodes!(actions.into_children();
            [IdList(list)] => {
                Ok(list.into_iter().map(|(node, span)| ActionName { node, span }).collect())
            },
        )
    }

    fn MultActId(actions: ParseNode) -> ParseResult<MultiActionLabel> {
        match_nodes!(actions.into_children();
            [Id(actions)..] => {
                Ok(MultiActionLabel { actions: actions.collect() })
            },
        )
    }

    fn MultActIdList(actions: ParseNode) -> ParseResult<Vec<MultiActionLabel>> {
        match_nodes!(actions.into_children();
            [MultActId(action), MultActId(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    pub(crate) fn MultActIdSet(actions: ParseNode) -> ParseResult<Vec<MultiActionLabel>> {
        match_nodes!(actions.into_children();
            [MultActIdList(list)] => {
                Ok(list)
            },
        )
    }

    fn ProcExpr(input: ParseNode) -> ParseResult<ProcessExpr> {
        parse_process_expr(input.children().as_pairs().clone())
    }

    fn ProcExprNoIf(input: ParseNode) -> ParseResult<ProcessExpr> {
        parse_process_expr(input.children().as_pairs().clone())
    }

    pub(crate) fn ProcExprId(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [Id(identifier)] => {
                Ok(ProcessExprKind::Id(identifier, Vec::new()).spanned(span))
            },
            [Id(identifier), AssignmentList(assignments)] => {
                Ok(ProcessExprKind::Id(identifier, assignments).spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprBlock(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [ActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Block {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprIfPrefix(input: ParseNode) -> ParseResult<(DataExpr, Option<ProcessExpr>)> {
        match_nodes!(input.into_children();
            [DataExpr(condition)] => {
                Ok((condition, None))
            },
            [DataExpr(condition), ProcExprNoIf(then)] => {
                Ok((condition, Some(then)))
            },
        )
    }

    pub(crate) fn ProcExprIfThen(input: ParseNode) -> ParseResult<(DataExpr, ProcessExpr)> {
        match_nodes!(input.into_children();
            [DataExpr(condition), ProcExprNoIf(expr)] => {
                Ok((condition, expr))
            },
        )
    }

    pub(crate) fn ProcExprAllow(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [MultActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Allow {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprHide(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [ActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Hide {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    fn ActionList(actions: ParseNode) -> ParseResult<Vec<Action>> {
        match_nodes!(actions.into_children();
            [Action(action), Action(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    fn MultiActTau(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    fn ProcExprDelta(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    pub(crate) fn MultAct(input: ParseNode) -> ParseResult<MultiAction> {
        match_nodes!(input.into_children();
            [MultiActTau(_)] => {
                Ok(MultiAction { actions: Vec::new() })
            },
            [ActionList(actions)] => {
                Ok(MultiAction { actions })
            },
        )
    }

    fn CommExpr(action: ParseNode) -> ParseResult<CommExpr> {
        match_nodes!(action.into_children();
            [Id(first), MultActId(mut multiact), Id(to)] => {
                multiact.actions.insert(0, first);
                Ok(CommExpr { from: multiact, to })
            },
        )
    }

    fn CommExprList(actions: ParseNode) -> ParseResult<Vec<CommExpr>> {
        match_nodes!(actions.into_children();
            [CommExpr(action), CommExpr(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    pub(crate) fn CommExprSet(actions: ParseNode) -> ParseResult<Vec<CommExpr>> {
        match_nodes!(actions.into_children();
            [CommExprList(list)] => {
                Ok(list)
            },
        )
    }

    pub(crate) fn ProcExprRename(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [RenExprSet(renames), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Rename {
                    renames,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprComm(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [CommExprSet(comm), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Comm {
                    comm,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn Action(input: ParseNode) -> ParseResult<Action> {
        match_nodes!(input.into_children();
            [Id(id)] => {
                Ok(Action { id, args: Vec::new() })
            },
            [Id(id), DataExprList(args)] => {
                Ok(Action { id, args })
            },
        )
    }

    fn RenExprSet(renames: ParseNode) -> ParseResult<Vec<Rename>> {
        match_nodes!(renames.into_children();
            [RenExprList(renames)] => {
                Ok(renames)
            },
        )
    }

    fn RenExprList(renames: ParseNode) -> ParseResult<Vec<Rename>> {
        match_nodes!(renames.into_children();
            [RenExpr(renames)..] => {
                Ok(renames.collect())
            },
        )
    }

    fn RenExpr(renames: ParseNode) -> ParseResult<Rename> {
        match_nodes!(renames.into_children();
            [Id(from), Id(to)] => {
                Ok(Rename { from, to })
            },
        )
    }

    pub(crate) fn ProcExprSum(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn ProcExprDist(input: ParseNode) -> ParseResult<(Vec<IdDecl>, DataExpr)> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables), DataExpr(expr)] => {
                Ok((variables, expr))
            },
        )
    }

    pub(crate) fn StateFrmDelay(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        // The `@`-time argument is optional, so there may be zero or one child.
        match input.into_children().next() {
            Some(child) => Ok(StateFrmKind::Delay(Some(Mcrl2Parser::DataExpr(child)?)).spanned(span)),
            None => Ok(StateFrmKind::Delay(None).spanned(span)),
        }
    }

    pub(crate) fn StateFrmYaled(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        // The `@`-time argument is optional, so there may be zero or one child.
        match input.into_children().next() {
            Some(child) => Ok(StateFrmKind::Yaled(Some(Mcrl2Parser::DataExpr(child)?)).spanned(span)),
            None => Ok(StateFrmKind::Yaled(None).spanned(span)),
        }
    }

    pub(crate) fn StateFrmNegation(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [StateFrm(state)] => {
                Ok(StateFrmKind::Unary { op: crate::StateFrmUnaryOp::Negation, expr: Box::new(state) }.spanned(span))
            },
        )
    }

    pub(crate) fn StateFrmLeftConstantMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn StateFrmRightConstantMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn StateFrmDiamond(input: ParseNode) -> ParseResult<RegFrm> {
        match_nodes!(input.into_children();
            [RegFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmBox(input: ParseNode) -> ParseResult<RegFrm> {
        match_nodes!(input.into_children();
            [RegFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmSpec(spec: ParseNode) -> ParseResult<UntypedStateFrmSpec> {
        let mut map_declarations = Vec::new();
        let mut equation_declarations = Vec::new();
        let mut constructor_declarations = Vec::new();
        let mut sort_declarations = Vec::new();
        let mut type_var_declarations = Vec::new();
        let mut action_declarations = Vec::new();

        let mut form_spec = None;

        let span = spec.as_span();
        for child in spec.into_children() {
            match child.as_rule() {
                Rule::StateFrmSpecElt => {
                    let element = child
                        .into_children()
                        .next()
                        .expect("StateFrmSpecElt has exactly one child");
                    match element.as_rule() {
                        Rule::ConsSpec => {
                            constructor_declarations.append(&mut Mcrl2Parser::ConsSpec(element)?);
                        }
                        Rule::MapSpec => {
                            map_declarations.append(&mut Mcrl2Parser::MapSpec(element)?);
                        }
                        Rule::EqnSpec => {
                            equation_declarations.append(&mut Mcrl2Parser::EqnSpec(element)?);
                        }
                        Rule::SortSpec => {
                            sort_declarations.append(&mut Mcrl2Parser::SortSpec(element)?);
                        }
                        Rule::TypeVarSpec => {
                            type_var_declarations.append(&mut Mcrl2Parser::TypeVarSpec(element)?);
                        }
                        Rule::ActSpec => {
                            action_declarations.append(&mut Mcrl2Parser::ActSpec(element)?);
                        }
                        _ => {
                            unimplemented!("Unexpected rule in StateFrmSpecElt: {:?}", element.as_rule());
                        }
                    }
                }
                Rule::StateFrm => {
                    if form_spec.is_some() {
                        return Err(Error::new_from_span(
                            ErrorVariant::CustomError {
                                message: "Multiple state formula specifications are not allowed".to_string(),
                            },
                            child.as_span(),
                        ));
                    }
                    form_spec = Some(Mcrl2Parser::StateFrm(child)?);
                }
                Rule::FormSpec => {
                    if form_spec.is_some() {
                        return Err(Error::new_from_span(
                            ErrorVariant::CustomError {
                                message: "Multiple state formula specifications are not allowed".to_string(),
                            },
                            child.as_span(),
                        ));
                    }
                    form_spec = Some(Mcrl2Parser::FormSpec(child)?);
                }
                Rule::EOI => {
                    // End of input
                    break;
                }
                _ => {
                    unimplemented!("Unexpected rule: {:?}", child.as_rule());
                }
            }
        }

        let mut data_specification = UntypedDataSpecification {
            map_declarations,
            equation_declarations,
            constructor_declarations,
            sort_declarations,
            type_var_declarations,
        };
        bind_type_vars(&mut data_specification);

        Ok(UntypedStateFrmSpec {
            data_specification,
            action_declarations,
            formula: form_spec.ok_or(Error::new_from_span(
                ErrorVariant::CustomError {
                    message: "No state formula found in the state formula specification".to_string(),
                },
                span,
            ))?,
        })
    }

    pub(crate) fn PbesExprForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn PbesExprExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn PresExprEqinf(input: ParseNode) -> ParseResult<PresExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [PresExpr(body)] => {
                Ok(PresExprKind::Equal {
                    eq: Eq::EqInf,
                    body: Box::new(body),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn PresExprEqninf(input: ParseNode) -> ParseResult<PresExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [PresExpr(body)] => {
                Ok(PresExprKind::Equal {
                    eq: Eq::EqnInf,
                    body: Box::new(body),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn PresExprCondsm(input: ParseNode) -> ParseResult<PresExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [PresExpr(expr), PresExpr(then), PresExpr(else_)] => {
                Ok(PresExprKind::Condition{
                    condition: Condition::Condsm,
                    lhs: Box::new(expr),
                    then: Box::new(then),
                    else_: Box::new(else_),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn PresExprCondeq(input: ParseNode) -> ParseResult<PresExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [PresExpr(expr), PresExpr(then), PresExpr(else_)] => {
                Ok(PresExprKind::Condition{
                    condition: Condition::Condeq,
                    lhs: Box::new(expr),
                    then: Box::new(then),
                    else_: Box::new(else_),
                }.spanned(span))
            },
        )
    }

    fn IdsDecl(decl: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(decl.into_children();
            [IdInfixList(identifiers), SortExpr(sort)] => {
                let id_decls = identifiers.into_iter().map(|(identifier, span)| {
                    IdDecl::new(identifier, sort.clone(), span)
                }).collect();

                Ok(id_decls)
            },
        )
    }

    fn EqnSpec(spec: ParseNode) -> ParseResult<Vec<EqnSpec>> {
        let span = spec.as_span();
        let mut ids = Vec::new();

        match_nodes!(spec.into_children();
            [VarSpec(variables), EqnDecl(decls)..] => {
                ids.push(EqnSpecData {
                    variables,
                    equations: decls.collect(),
                    id: None,
                }.spanned(span.into()));
            },
            [EqnDecl(decls)..] => {
                ids.push(EqnSpecData { variables: Vec::new(), equations: decls.collect(), id: None }.spanned(span.into()));
            },
        );

        Ok(ids)
    }

    fn EqnDecl(decl: ParseNode) -> ParseResult<EqnDecl> {
        let span = decl.as_span();
        match_nodes!(decl.into_children();
            [DataExpr(condition), DataExpr(lhs), DataExpr(rhs)] => {
                Ok(EqnDecl { condition: Some(condition), lhs, rhs, span: span.into(), id: None })
            },
            [DataExpr(lhs), DataExpr(rhs)] => {
                Ok(EqnDecl { condition: None, lhs, rhs, span: span.into(), id: None })
            },
        )
    }

    fn StateFrm(input: ParseNode) -> ParseResult<StateFrm> {
        parse_statefrm(input.children().as_pairs().clone())
    }

    fn RegFrm(input: ParseNode) -> ParseResult<RegFrm> {
        parse_regfrm(input.children().as_pairs().clone())
    }

    fn StateVarDecl(input: ParseNode) -> ParseResult<StateVarDecl> {
        let span = input.as_span();
        match_nodes!(input.into_children();
            [Id(identifier), StateVarAssignmentList(arguments)] => {
                Ok(StateVarDecl {
                    identifier: identifier.node,
                    arguments,
                    span: span.into(),
                    id: None,
                })
            },
            [Id(identifier)] => {
                Ok(StateVarDecl {
                    identifier: identifier.node,
                    arguments: Vec::new(),
                    span: span.into(),
                    id: None,
                })
            }
        )
    }

    fn StateVarAssignmentList(input: ParseNode) -> ParseResult<Vec<StateVarAssignment>> {
        match_nodes!(input.into_children();
            [StateVarAssignment(assignments)..] => {
                Ok(assignments.collect())
            }
        )
    }

    fn StateVarAssignment(input: ParseNode) -> ParseResult<StateVarAssignment> {
        match_nodes!(input.into_children();
            [Id(identifier), SortExpr(sort), DataExpr(expr)] => {
                Ok(StateVarAssignment { identifier, sort, expr, id: None })
            }
        )
    }

    fn ActionRenameRuleSpec(spec: ParseNode) -> ParseResult<Vec<ActionRenameDecl>> {
        match_nodes!(spec.into_children();
            [VarSpec(variables_specification), ActionRenameRule(renames)..] => {
                Ok(renames.map(|rename_rule| {
                    ActionRenameDecl { variables_specification: variables_specification.clone(), rename_rule }
                }).collect())
            },
            [ActionRenameRule(renames)..] => {
                Ok(renames.map(|rename_rule| {
                    ActionRenameDecl { variables_specification: Vec::new(), rename_rule }
                }).collect())
            },
        )
    }

    fn ActionRenameRule(input: ParseNode) -> ParseResult<ActionRenameRule> {
        match_nodes!(input.into_children();
            [DataExpr(condition), Action(action), ActionRenameRuleRHS(rhs)] => {
                Ok(ActionRenameRule { condition: Some(condition), action, rhs })
            },
            [Action(action), ActionRenameRuleRHS(rhs)] => {
                Ok(ActionRenameRule { condition: None, action, rhs })
            },
        )
    }

    fn ActionRenameRuleRHS(input: ParseNode) -> ParseResult<ActionRHS> {
        match_nodes!(input.into_children();
            [Action(action)] => {
                Ok(ActionRHS::Action(action))
            },
            [MultiActTau(_)] => {
                Ok(ActionRHS::Tau)
            },
            [ProcExprDelta(_)] => {
                Ok(ActionRHS::Delta)
            },
        )
    }

    fn FormSpec(input: ParseNode) -> ParseResult<StateFrm> {
        match_nodes!(input.into_children();
            [StateFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmSup(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmInf(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmSum(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn PresExprInf(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn PresExprSup(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn PresExprSum(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn PresExprLeftConstantMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(constant)] => {
                Ok(constant)
            },
        )
    }

    pub(crate) fn PresExprRightConstMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(constant)] => {
                Ok(constant)
            },
        )
    }

    fn EOI(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }
}
