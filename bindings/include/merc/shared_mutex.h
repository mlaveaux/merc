// Author(s): Maurice Laveaux
// Copyright: see the accompanying file COPYING or copy at
// https://github.com/mCRL2org/mCRL2/blob/master/COPYING
//
// Distributed under the Boost Software License, Version 1.0.
// (See accompanying file LICENSE_1_0.txt or copy at
// http://www.boost.org/LICENSE_1_0.txt)
//

#ifndef MERC_SHARED_MUTEX_H
#define MERC_SHARED_MUTEX_H

#include <merc_ffi.h>

namespace merc
{

/// A shared lock guard for the shared_mutex.
class shared_guard
{
  friend shared_guard global_lock_shared();

public:
  shared_guard(const shared_guard&) = delete;
  shared_guard(shared_guard&&) = delete;
  shared_guard& operator=(const shared_guard&) = delete;
  shared_guard& operator=(shared_guard&&) = delete;

  /// Locks the guard again explicitly.
  inline void lock_shared()
  {
    merc::ffi::global_lock_shared();
    is_locked = true;
  }

  /// Unlocks the acquired shared guard explicitly. Otherwise, performed in destructor.
  inline void unlock_shared()
  {
    merc::ffi::global_unlock_shared();
    is_locked = false;
  }

  ~shared_guard()
  {
    if (is_locked)
    {
      unlock_shared();
    }
  }

private:
  shared_guard() noexcept = default;

  bool is_locked = true;
};

/// An exclusive lock guard for the shared_mutex.
class lock_guard
{
  friend lock_guard global_lock_exclusive();

public:
  lock_guard(const lock_guard&) = delete;
  lock_guard(lock_guard&&) = delete;
  lock_guard& operator=(const lock_guard&) = delete;
  lock_guard& operator=(lock_guard&&) = delete;

  /// Unlocks the acquired shared guard explicitly. Otherwise, performed in destructor.
  void unlock()
  {
    merc::ffi::global_unlock_exclusive();
    is_locked = false;
  }

  ~lock_guard()
  {
    if (is_locked)
    {
      unlock();
    }
  }

private:
  lock_guard() noexcept = default;

  bool is_locked = true;
};

inline
shared_guard global_lock_shared()
{
  merc::ffi::global_lock_exclusive();
  return shared_guard();
}

inline
lock_guard global_lock_exclusive()
{
  merc::ffi::global_lock_exclusive();
  return lock_guard();
}

} // namespace merc

#endif // MERC_SHARED_MUTEX_H
