//!
//! See the export module for the function documentation.
//!

#[link(name = "sabre-ffi")]
unsafe extern "C" {
    /// Returns the argument of a data expression.
    fn data_expression_arg(term: DataExpressionRefFFI<'_>, index: usize) -> DataExpressionRefFFI<'_>;

    /// Returns the data function symbol of a data expression.
    fn data_expression_symbol(term: DataExpressionRefFFI<'_>) -> DataFunctionSymbolRefFFI<'_>;

    /// Protects the data expression.
    fn data_expression_protect(term: DataExpressionRefFFI<'_>) -> DataExpressionFFI;
}
