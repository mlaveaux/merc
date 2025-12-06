/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/atermpp/aterm_string.h"
#include "mcrl2/data/data_expression.h"
#include "mcrl2/data/sort_expression.h"
#include "mcrl2/data/variable.h"

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

} // namespace mcrl2::data