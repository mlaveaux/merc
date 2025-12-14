/// Wrapper around the atermpp library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/atermpp/aterm_io.h"
#include "mcrl2/atermpp/aterm_list.h"
#include "mcrl2/atermpp/aterm_string.h"

#include "rust/cxx.h"

#include <cstddef>
#include <memory>
#include <new>
#include <stack>

namespace atermpp
{

using void_callback = rust::Fn<void(term_mark_stack&)>;
using size_callback = rust::Fn<std::size_t()>;

// This has the same layout as function_symbol, but does not manage reference counting.
// It is used for cheap casting, and although that is definitely UB it is also done this ways for
// aterms in the actual toolset. So it should be fine.
struct unprotected_function_symbol
{
  unprotected_function_symbol(const detail::_function_symbol& symbol)
      : m_symbol(&symbol)
  {}

  mcrl2::utilities::shared_reference<const detail::_function_symbol> m_symbol;
};

/// Returns the internal address of a function symbol.
inline const detail::_function_symbol* mcrl2_function_symbol_address(const function_symbol& symbol)
{
  return reinterpret_cast<const unprotected_function_symbol&>(symbol).m_symbol.get();
}

// What the fuck is this. Leaks the inner type because unions are not destructed automatically.
template <typename T>
class Leaker
{
public:
  union
  {
    T m_val;
    char dummy;
  };
  template <typename... Args>
  Leaker(Args... inputArgs)
      : m_val(inputArgs...)
  {}
  ~Leaker() {}
};

/// A callback that can be used to protect additional terms during GC.
struct tls_callback_container : private mcrl2::utilities::noncopyable
{
public:
  tls_callback_container(void_callback callback_mark, size_callback callback_size)
      : m_container(std::bind(callback_mark, std::placeholders::_1), std::bind(callback_size))
  {}

private:
  detail::aterm_container m_container;
};

// Type definition
using term_mark_stack = std::stack<std::reference_wrapper<detail::_aterm>>;

// Functions for managing the aterm pool.

inline void mcrl2_aterm_pool_enable_automatic_garbage_collection(bool enabled)
{
  detail::g_term_pool().enable_garbage_collection(enabled);
}

inline std::size_t mcrl2_aterm_pool_size()
{
  return detail::g_term_pool().size();
}

inline std::size_t mcrl2_aterm_pool_capacity()
{
  return detail::g_term_pool().capacity();
}

inline void mcrl2_aterm_pool_collect_garbage()
{
  detail::g_thread_term_pool().collect();
}

inline void mcrl2_aterm_pool_test_garbage_collection()
{
  // TODO: Is this function nnecessary?
  // detail::g_thread_term_pool().test_garbage_collection();
}

inline void mcrl2_aterm_pool_lock_shared()
{
  detail::g_thread_term_pool().shared_mutex().lock_shared_impl();
}

inline bool mcrl2_aterm_pool_unlock_shared()
{
  detail::g_thread_term_pool().shared_mutex().unlock_shared();
  return !detail::g_thread_term_pool().is_shared_locked();
}

inline void mcrl2_aterm_pool_lock_exclusive()
{
  detail::g_thread_term_pool().shared_mutex().lock_impl();
}

inline void mcrl2_aterm_pool_unlock_exclusive()
{
  detail::g_thread_term_pool().shared_mutex().unlock();
}

inline std::unique_ptr<tls_callback_container> mcrl2_aterm_pool_register_mark_callback(void_callback callback_mark,
    size_callback callback_size)
{
  return std::make_unique<tls_callback_container>(callback_mark, callback_size);
}

inline void mcrl2_aterm_pool_print_metrics()
{
  detail::g_thread_term_pool().print_local_performance_statistics();
}

// Aterm related functions

inline const detail::_aterm* mcrl2_aterm_create(const detail::_function_symbol& symbol,
    rust::Slice<const detail::_aterm* const> arguments)
{
  rust::Slice<aterm> aterm_slice(const_cast<aterm*>(reinterpret_cast<const aterm*>(arguments.data())),
      arguments.length());

  unprotected_aterm_core result(nullptr);
  atermpp::unprotected_function_symbol tmp_symbol(symbol);
  make_term_appl(reinterpret_cast<aterm&>(result),
      reinterpret_cast<const function_symbol&>(tmp_symbol),
      aterm_slice.begin(),
      aterm_slice.end());
  return detail::address(result);
  return 0;
}

inline std::unique_ptr<aterm> mcrl2_aterm_from_string(rust::Str text)
{
  return std::make_unique<aterm>(read_term_from_string(static_cast<std::string>(text)));
}

inline const detail::_aterm* mcrl2_aterm_get_address(const atermpp::aterm& term)
{
  return detail::address(term);
}

inline void mcrl2_aterm_mark_address(const detail::_aterm& term, term_mark_stack& todo)
{
  atermpp::unprotected_aterm_core tmp(&term);
  mark_term(atermpp::down_cast<aterm>(tmp), todo);
}

inline bool mcrl2_aterm_is_list(const detail::_aterm& term)
{
  unprotected_aterm_core tmp(&term);
  return atermpp::down_cast<aterm>(tmp).type_is_list();
}

inline bool mcrl2_aterm_is_empty_list(const detail::_aterm& term)
{
  atermpp::unprotected_aterm_core tmp(&term);
  return atermpp::down_cast<aterm>(tmp).function() == detail::g_as_empty_list;
}

inline bool mcrl2_aterm_is_int(const detail::_aterm& term)
{
  atermpp::unprotected_aterm_core tmp(&term);
  return atermpp::down_cast<aterm>(tmp).type_is_int();
}

inline rust::String mcrl2_aterm_print(const detail::_aterm& term)
{
  std::stringstream str;
  atermpp::unprotected_aterm_core tmp(&term);
  str << atermpp::down_cast<aterm>(tmp);
  return str.str();
}

inline const detail::_function_symbol* mcrl2_aterm_get_function_symbol(const detail::_aterm& term)
{
  atermpp::unprotected_aterm_core tmp(&term);
  return mcrl2_function_symbol_address(atermpp::down_cast<aterm>(tmp).function());
}

inline const detail::_aterm* mcrl2_aterm_get_argument(const detail::_aterm& term, std::size_t index)
{
  atermpp::unprotected_aterm_core tmp(&term);
  return detail::address(atermpp::down_cast<aterm>(tmp)[index]);
}

// Function symbol related functions

inline const detail::_function_symbol* mcrl2_function_symbol_create(rust::String name, std::size_t arity)
{
  Leaker<function_symbol> leak = Leaker<function_symbol>(static_cast<std::string>(name), arity);
  return mcrl2_function_symbol_address(leak.m_val);
}

inline const detail::_function_symbol* mcrl2_function_symbol_get_address(const function_symbol& symbol)
{
  return mcrl2_function_symbol_address(symbol);
}

inline rust::Str mcrl2_function_symbol_get_name(const detail::_function_symbol& symbol)
{
  return symbol.name();
}

inline std::size_t mcrl2_function_symbol_get_arity(const detail::_function_symbol& symbol)
{
  return symbol.arity();
}

inline void mcrl2_function_symbol_protect(const detail::_function_symbol& symbol)
{
  symbol.increment_reference_count();
}

inline void mcrl2_function_symbol_drop(const detail::_function_symbol& symbol)
{
  symbol.decrement_reference_count();
}

} // namespace atermpp