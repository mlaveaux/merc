/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once
#include <cstddef>

#include "rust/cxx.h"

#include "mcrl2/atermpp/aterm.h"

namespace atermpp
{

std::size_t mcrl2_aterm_list_size(const aterm& term)
{
  return term.size();
}

std::unique_ptr<aterm> mcrl2_aterm_argument(const aterm& term, std::size_t index)
{
  return std::make_unique<aterm>(term[index]);
}

} // namespace atermpp