/// Wrapper around the log library of the mCRL2 toolset.

#pragma once
#include <memory>
#include <string>

#include "rust/cxx.h"

#include "mcrl2/utilities/logger.h"

namespace mcrl2::log
{

void mcrl2_set_reporting_level(std::size_t level)
{
    mcrl2::log::logger::set_reporting_level(static_cast<mcrl2::log::log_level_t>(level));
}

} // namespace mcrl2::pbes_system