// Author(s): Maurice Laveaux
// Copyright: see the accompanying file COPYING or copy at
// https://github.com/mCRL2org/mCRL2/blob/master/COPYING
//
// Distributed under the Boost Software License, Version 1.0.
// (See accompanying file LICENSE_1_0.txt or copy at
// http://www.boost.org/LICENSE_1_0.txt)
//

#include <merc/string_view.h>

#include <merc_ffi.h>

#include <iostream>

using namespace merc;
using namespace merc::ffi;

int main()
{
    function_symbol_t symbol = function_symbol_create("test", 4, 0, false);

    std::cout << "Function symbol created with name: " << to_string_view(function_symbol_get_name(symbol)) << std::endl;
    
}