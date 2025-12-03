/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/data/variable.h"

#include "rust/cxx.h"

namespace mcrl2::data
{

inline
rust::String mcrl2_variable_to_string(const variable& var)
{
    std::stringstream ss;
    ss << var;
    return ss.str();
}

inline
bool mcrl2_data_is_variable(const atermpp::aterm& term)
{
    return data::is_variable(term);
}


} // namespace mcrl2::data