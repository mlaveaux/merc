/// Wrapper around the log library of the mCRL2 toolset.

#pragma once


#include "mcrl2/utilities/logger.h"

namespace mcrl2::log
{

inline
void mcrl2_set_reporting_level(std::size_t level)
{
    mcrl2::log::logger::set_reporting_level(static_cast<mcrl2::log::log_level_t>(level));
}

} // namespace mcrl2::log