/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/atermpp/aterm_string.h"
#include "mcrl2/data/data_expression.h"
#include "mcrl2/data/data_specification.h"
#include "mcrl2/data/detail/rewrite/jitty.h"
#include "mcrl2/data/parse.h"
#include "mcrl2/data/sort_expression.h"
#include "mcrl2/data/variable.h"

#ifdef MCRL2_ENABLE_JITTYC
#include "mcrl2/data/detail/rewrite/jittyc.h"
#endif // MCRL2_ENABLE_JITTYC

#include "mcrl2-sys/cpp/assert.h"
#include "mcrl2-sys/cpp/atermpp.h"

#include "rust/cxx.h"

namespace mcrl2::data
{
inline
std::unique_ptr<data_specification> mcrl2_data_specification_from_string(rust::Str input)
{
  return std::make_unique<data_specification>(parse_data_specification(std::string(input)));
}

inline
std::unique_ptr<detail::RewriterJitty> mcrl2_create_rewriter_jitty(const data::data_specification& specification)
{
  return std::make_unique<detail::RewriterJitty>(specification, used_data_equation_selector(specification));
}

#ifdef MCRL2_ENABLE_JITTYC

inline
std::unique_ptr<detail::RewriterCompilingJitty> mcrl2_create_rewriter_jittyc(const data::data_specification& specification)
{
  return std::make_unique<detail::RewriterCompilingJitty>(specification, used_data_equation_selector(specification));
}

#endif

bool mcrl2_data_expression_is_variable(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_variable(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_application(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_application(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_abstraction(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_abstraction(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_function_symbol(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_function_symbol(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_where_clause(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_where_clause(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_machine_number(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_machine_number(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_untyped_identifier(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_untyped_identifier(atermpp::down_cast<atermpp::aterm>(tmp));
}

bool mcrl2_data_expression_is_data_expression(const atermpp::detail::_aterm& input)
{
  atermpp::unprotected_aterm_core tmp(&input);
  return data::is_data_expression(atermpp::down_cast<atermpp::aterm>(tmp));
}

} // namespace mcrl2::data