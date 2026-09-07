#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod closed;
mod data_expression;
mod data_terms;
mod machine_word_evaluation;
mod mcrl2_data_specification;
mod sort_terms;
mod undefined;
mod visitor;

// Explicit pub(crate) re-exports.
pub(crate) use data_terms::DATA_SYMBOLS;
pub(crate) use data_terms::is_basic_sort;
pub(crate) use data_terms::is_data_equation;
pub(crate) use data_terms::is_data_expression;
pub(crate) use data_terms::is_data_whr_decl;
pub(crate) use data_terms::is_sort_alias;
pub(crate) use data_terms::is_sort_expression;

// Public API
pub use closed::is_closed;
pub use data_expression::BinderType;
pub use data_expression::DataAbstraction;
pub use data_expression::DataApplication;
pub use data_expression::DataApplicationRef;
pub use data_expression::DataEquation;
pub use data_expression::DataExpression;
pub use data_expression::DataExpressionRef;
pub use data_expression::DataFunctionSymbol;
pub use data_expression::DataFunctionSymbolRef;
pub use data_expression::DataVariable;
pub use data_expression::DataVariableRef;
pub use data_expression::DataWhereClause;
pub use data_expression::DataWhrDecl;
pub use data_expression::MachineNumber;
pub use data_expression::MachineNumberRef;
pub use data_expression::to_untyped_data_expression;
pub use data_terms::is_container_sort;
pub use data_terms::is_data_application;
pub use data_terms::is_data_binder;
pub use data_terms::is_data_function_symbol;
pub use data_terms::is_data_machine_number;
pub use data_terms::is_data_variable;
pub use data_terms::is_data_where_clause;
pub use data_terms::is_function_sort;
pub use machine_word_evaluation::MachineWordOp;
pub use machine_word_evaluation::try_evaluate_machine_word;
pub use mcrl2_data_specification::Mcrl2DataSpecification;
pub use sort_terms::BasicSort;
pub use sort_terms::BasicSortRef;
pub use sort_terms::ContainerSortKind;
pub use sort_terms::SortAlias;
pub use sort_terms::SortArrow;
pub use sort_terms::SortArrowRef;
pub use sort_terms::SortCons;
pub use sort_terms::SortConsRef;
pub use sort_terms::SortExpression;
pub use sort_terms::SortExpressionRef;
pub use undefined::is_undefined_real;
pub use undefined::is_undefined_real_ref;
pub use undefined::undefined_real;
pub use visitor::ClosureVisitor;
pub use visitor::DataExpressionVisitor;
pub use visitor::try_visit_data_expr;
pub use visitor::visit_data_expr;
