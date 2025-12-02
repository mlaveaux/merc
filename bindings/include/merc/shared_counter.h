// Author(s): Maurice Laveaux
// Copyright: see the accompanying file COPYING or copy at
// https://github.com/mCRL2org/mCRL2/blob/master/COPYING
//
// Distributed under the Boost Software License, Version 1.0.
// (See accompanying file LICENSE_1_0.txt or copy at
// http://www.boost.org/LICENSE_1_0.txt)
//

#ifndef MERC_SHARED_COUNTER_H
#define MERC_SHARED_COUNTER_H

#include <merc_ffi.h>

namespace merc
{

class shared_counter
{

public:
  shared_counter() noexcept = default;
  shared_counter(merc::ffi::prefix_shared_counter_t counter) noexcept
      : m_counter(counter)
  {
    merc::ffi::shared_counter_add_ref(m_counter);
  }

  ~shared_counter() noexcept { merc::ffi::shared_counter_unref(m_counter); }

  shared_counter(const shared_counter& other) noexcept
      : m_counter(other.m_counter)
  {
    merc::ffi::shared_counter_add_ref(m_counter);
  }

  shared_counter& operator=(const shared_counter& other) noexcept
  {
    if (this != &other)
    {
      m_counter = other.m_counter;
    }
    merc::ffi::shared_counter_add_ref(m_counter);
    return *this;
  }

  auto operator*() const -> std::size_t { return merc::ffi::shared_counter_value(m_counter); }

  auto operator*() -> decltype(auto) { return merc::ffi::shared_counter_value(m_counter); }

private:
  merc::ffi::prefix_shared_counter_t m_counter;
};

}

#endif // MERC_SHARED_COUNTER_H