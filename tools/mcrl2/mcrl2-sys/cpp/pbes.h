/// Wrapper around the PBES library of the mCRL2 toolset.

#pragma once
#include <memory>
#include <string>
#include <cstddef>
#include <vector>

#include "rust/cxx.h"

#include "mcrl2/pbes/detail/stategraph_local_algorithm.h"
#include "mcrl2/pbes/io.h"
#include "mcrl2/pbes/pbes.h"
#include "mcrl2/pbes/srf_pbes.h"
#include "mcrl2/pbes/unify_parameters.h"

namespace mcrl2::pbes_system
{

std::unique_ptr<pbes> mcrl2_load_pbes_from_file(rust::Str filename)
{
  pbes result;
  load_pbes(result, static_cast<std::string>(filename));
  return std::make_unique<pbes>(result);
}

class stategraph_algorithm : private detail::stategraph_local_algorithm 
{
    using super = detail::stategraph_local_algorithm;
public:

  stategraph_algorithm(const pbes& input)
    : super(input, pbesstategraph_options{.print_influence_graph = true})
  {}
  
  void run() override
  {
    // We explicitly ignore the virtual call to run in the base class
    detail::stategraph_algorithm::stategraph_algorithm::run(); // NOLINT(bugprone-parent-virtual-call)

    compute_local_control_flow_graphs();

    for (decltype(m_local_control_flow_graphs)::iterator i = m_local_control_flow_graphs.begin();
      i != m_local_control_flow_graphs.end();
      ++i)
    {
      mCRL2log(log::verbose) << "--- computed local control flow graph " << (i - m_local_control_flow_graphs.begin())
                             << " --- \n"
                             << *i << std::endl;
    }
  }

  const std::vector<detail::local_control_flow_graph>& local_control_flow_graphs() const
  { 
    return m_local_control_flow_graphs;
  }
};

std::unique_ptr<stategraph_algorithm> mcrl2_pbes_stategraph_local_algorithm_run(const pbes& p)
{
  auto algorithm = std::make_unique<stategraph_algorithm>(p);
  algorithm->run();
  return algorithm;
}

std::size_t mcrl2_pbes_stategraph_local_algorithm_cfgs_size(const stategraph_algorithm& algorithm)
{
  return algorithm.local_control_flow_graphs().size();
}

std::unique_ptr<srf_pbes> mcrl2_pbes_to_srf_pbes(const pbes& p)
{
  return std::make_unique<srf_pbes>(pbes2srf(p));
}

void mcrl2_unify_parameters(srf_pbes& p, bool ignore_ce_equations, bool reset)
{
  unify_parameters(p, ignore_ce_equations, reset);
}

std::unique_ptr<pbes> mcrl2_srf_pbes_to_pbes(const srf_pbes& p)
{
  return std::make_unique<pbes>(p.to_pbes());
}


} // namespace mcrl2::pbes_system