/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"

#include "rust/cxx.h"

#include <cstddef>

namespace atermpp
{

inline
std::size_t mcrl2_aterm_list_size(const aterm& term)
{
  return term.size();
}

inline
std::unique_ptr<aterm> mcrl2_aterm_argument(const aterm& term, std::size_t index)
{
  return std::make_unique<aterm>(term[index]);
}

inline
rust::String mcrl2_aterm_to_string(const aterm& term)
{
    std::stringstream ss;
    ss << term;
    return ss.str();
}

} // namespace atermpp