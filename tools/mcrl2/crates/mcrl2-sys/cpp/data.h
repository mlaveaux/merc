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

#include "rust/cxx.h"

namespace mcrl2::data
{

inline
rust::String mcrl2_variable_to_string(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(data::is_variable(variable));
    std::stringstream ss;
    ss << atermpp::down_cast<data::variable>(variable);
    return ss.str();
}

inline
rust::String mcrl2_data_expression_to_string(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(data::is_data_expression(variable));
    std::stringstream ss;
    ss << atermpp::down_cast<data::data_expression>(variable);
    return ss.str();
}

inline
rust::String mcrl2_sort_to_string(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(data::is_sort_expression(variable));
    std::stringstream ss;
    ss << atermpp::down_cast<data::sort_expression>(variable);
    return ss.str();
}

inline
std::unique_ptr<data_specification> mcrl2_data_specification_from_string(rust::Str input)
{
  return std::make_unique<data_specification>(parse_data_specification(std::string(input)));
}

inline
std::unique_ptr<atermpp::aterm> mcrl2_variable_sort(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(data::is_variable(variable));
  return std::make_unique<atermpp::aterm>(atermpp::down_cast<data::variable>(variable).sort());
}

inline
std::unique_ptr<atermpp::aterm> mcrl2_variable_name(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(data::is_variable(variable));
  return std::make_unique<atermpp::aterm>(atermpp::down_cast<data::variable>(variable).name());
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

bool mcrl2_data_expression_is_variable(const atermpp::aterm& input)
{
  return data::is_variable(input);
}

bool mcrl2_data_expression_is_application(const atermpp::aterm& input)
{
  return data::is_application(input);
}

bool mcrl2_data_expression_is_abstraction(const atermpp::aterm& input)
{
  return data::is_abstraction(input);
}

bool mcrl2_data_expression_is_function_symbol(const atermpp::aterm& input)
{
  return data::is_function_symbol(input);
}

bool mcrl2_data_expression_is_where_clause(const atermpp::aterm& input)
{
  return data::is_where_clause(input);
}

bool mcrl2_data_expression_is_machine_number(const atermpp::aterm& input)
{
  return data::is_machine_number(input);
}

bool mcrl2_data_expression_is_untyped_identifier(const atermpp::aterm& input)
{
  return data::is_untyped_identifier(input);
}

bool mcrl2_data_expression_is_data_expression(const atermpp::aterm& input)
{
  return data::is_data_expression(input);
}

} // namespace mcrl2::data