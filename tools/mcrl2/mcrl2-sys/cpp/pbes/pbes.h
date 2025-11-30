#pragma once
#include <memory>
#include <string>

#include "rust/cxx.h"

#include "mcrl2/pbes/pbes.h"
#include "mcrl2/pbes/io.h"

namespace mcrl2::pbes_system
{

std::unique_ptr<pbes> load_pbes_from_file(rust::Str filename)
{
    pbes result;
    load_pbes(result, static_cast<std::string>(filename));
    return std::make_unique<pbes>(result);
}

} // namespace mcrl2::pbes_system