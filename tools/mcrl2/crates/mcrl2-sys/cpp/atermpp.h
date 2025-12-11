/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/atermpp/aterm_list.h"
#include "mcrl2/atermpp/aterm_string.h"

#include "rust/cxx.h"

#include <cstddef>
#include <memory>

namespace atermpp
{

// atermpp::aterm_list

inline
std::unique_ptr<aterm> mcrl2_aterm_list_front(const aterm& term)
{
  return std::make_unique<aterm>(down_cast<aterm_list>(term).front());
}

inline
std::unique_ptr<aterm> mcrl2_aterm_list_tail(const aterm& term)
{
  return std::make_unique<aterm>(down_cast<aterm_list>(term).tail());
}

inline
std::unique_ptr<aterm> mcrl2_aterm_argument(const aterm& term, std::size_t index)
{
  return std::make_unique<aterm>(term[index]);
}

inline
std::unique_ptr<aterm> mcrl2_aterm_clone(const aterm& term)
{
    return std::make_unique<aterm>(term);
}

inline
bool mcrl2_aterm_list_is_empty(const aterm& term)
{
    return down_cast<aterm_list>(term).empty();
}

// atermpp::aterm

inline
rust::String mcrl2_aterm_to_string(const aterm& term)
{
    std::stringstream ss;
    ss << term;
    return ss.str();
}

inline
bool mcrl2_aterm_are_equal(const aterm& left, const aterm& right)
{
    return left == right;
}

// atermpp::aterm_string

inline
rust::String mcrl2_aterm_string_to_string(const aterm& term)
{
    std::stringstream ss;
    ss << down_cast<aterm_string>(term);
    return ss.str();
}

} // namespace atermpp