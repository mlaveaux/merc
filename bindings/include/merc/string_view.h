// Author(s): Maurice Laveaux
// Copyright: see the accompanying file COPYING or copy at
// https://github.com/mCRL2org/mCRL2/blob/master/COPYING
//
// Distributed under the Boost Software License, Version 1.0.
// (See accompanying file LICENSE_1_0.txt or copy at
// http://www.boost.org/LICENSE_1_0.txt)
//

#ifndef MERC_STRING_VIEW_H
#define MERC_STRING_VIEW_H

#include <merc_ffi.h>

namespace merc {

/// Converts a `merc::ffi::string_view_t` to a `std::string_view`.
inline
std::string_view to_string_view(merc::ffi::string_view_t view) {
    return std::string_view(view.ptr, view.length);
}

} // namespace merc

#endif // MERC_STRING_VIEW_H