#pragma once
#include <memory>
#include <string>

#include "rust/cxx.h"

#include "mcrl2/pbes/detail/stategraph_local_algorithm.h"
#include "mcrl2/pbes/io.h"
#include "mcrl2/pbes/pbes.h"

namespace mcrl2::pbes_system
{

std::unique_ptr<pbes> load_pbes_from_file(rust::Str filename)
{
  pbes result;
  load_pbes(result, static_cast<std::string>(filename));
  return std::make_unique<pbes>(result);
}


class cliques_algorithm : private detail::stategraph_local_algorithm 
{
    using super = detail::stategraph_local_algorithm;
public:

  cliques_algorithm(const pbes& input)
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
};

std::unique_ptr<cliques_algorithm> run_stategraph_local_algorithm(const pbes& p)
{
  auto algorithm = std::make_unique<cliques_algorithm>(p);
  algorithm->run();
  return algorithm;
}

} // namespace mcrl2::pbes_system