/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/atermpp/aterm_list.h"
#include "mcrl2/atermpp/aterm_string.h"

#include "rust/cxx.h"

#include <cstddef>
#include <stack>
#include <memory>
#include <new>

namespace atermpp
{
  
// Leaks the inner type because unions are not destructed automatically.
template<typename T>
class Forget
{
public:
  union { T m_val; char dummy; };
  template<typename... Args>
  explicit Forget(Args&&... inputArgs)
  {
    new(&m_val) T(std::forward<Args>(inputArgs)...);
  }
  ~Forget() { }
};

// Type definition
using term_mark_stack = std::stack<std::reference_wrapper<detail::_aterm>>;

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

inline
void mcrl2_lock_shared() 
{
  detail::g_thread_term_pool().shared_mutex().lock_shared_impl();
}

bool mcrl2_unlock_shared() 
{
  detail::g_thread_term_pool().shared_mutex().unlock_shared();
  return !detail::g_thread_term_pool().is_shared_locked();
}

inline
void mcrl2_lock_exclusive() 
{
  detail::g_thread_term_pool().shared_mutex().lock_impl();
}

void mcrl2_unlock_exclusive() 
{
  detail::g_thread_term_pool().shared_mutex().unlock();
}

inline
void enable_automatic_garbage_collection(bool enabled)
{
  detail::g_term_pool().enable_garbage_collection(enabled);
}

inline
rust::Str mcrl2_function_symbol_name(const detail::_function_symbol* symbol)
{
  return symbol->name();
}

inline
std::size_t mcrl2_function_symbol_arity(const detail::_function_symbol* symbol)
{
  return symbol->arity();
}

inline
void mcrl2_protect_function_symbol(const detail::_function_symbol* symbol)
{
  symbol->increment_reference_count();
}

inline
void mcrl2_drop_function_symbol(const detail::_function_symbol* symbol)
{
  symbol->decrement_reference_count();
}


} // namespace atermpp