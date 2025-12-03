/// Wrapper around the PBES library of the mCRL2 toolset.

#pragma once

#include "mcrl2/pbes/detail/stategraph_local_algorithm.h"
#include "mcrl2/pbes/io.h"
#include "mcrl2/pbes/pbes.h"
#include "mcrl2/pbes/srf_pbes.h"
#include "mcrl2/pbes/unify_parameters.h"

#include "rust/cxx.h"

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

namespace mcrl2::pbes_system
{

/// Alias for templated type.
using srf_equation = detail::pre_srf_equation<false>;

// mcrl2::pbes_system::pbes

inline 
std::unique_ptr<pbes> mcrl2_load_pbes_from_file(rust::Str filename)
{
  pbes result;
  load_pbes(result, static_cast<std::string>(filename));
  return std::make_unique<pbes>(result);
}

inline
rust::String mcrl2_pbes_to_string(const pbes& pbesspec)
{
  std::stringstream ss;
  ss << pbesspec;
  return ss.str();
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

inline
std::unique_ptr<stategraph_algorithm> mcrl2_pbes_stategraph_local_algorithm_run(const pbes& p)
{
  auto algorithm = std::make_unique<stategraph_algorithm>(p);
  algorithm->run();
  return algorithm;
}

inline
void mcrl2_pbes_local_control_flow_graph_vertices(std::vector<detail::local_control_flow_graph_vertex>& result,
    const detail::local_control_flow_graph& cfg)
{
  for (const auto& v : cfg.vertices)
  {
    result.push_back(v);
  }
}

inline
void mcrl2_pbes_stategraph_local_algorithm_cfgs(std::vector<detail::local_control_flow_graph>& result,
    const stategraph_algorithm& algorithm)
{
  for (const auto& cfg : algorithm.local_control_flow_graphs())
  {
    result.push_back(cfg);
  }
}

inline
std::unique_ptr<srf_pbes> mcrl2_pbes_to_srf_pbes(const pbes& p)
{
  return std::make_unique<srf_pbes>(pbes2srf(p));
}

inline
void mcrl2_unify_parameters(srf_pbes& p, bool ignore_ce_equations, bool reset)
{
  unify_parameters(p, ignore_ce_equations, reset);
}

// mcrl2::pbes_system::srf_pbes

inline
std::unique_ptr<pbes> mcrl2_srf_pbes_to_pbes(const srf_pbes& p)
{
  return std::make_unique<pbes>(p.to_pbes());
}

// mcrl2::pbes_system::srf_equation

inline
void mcrl2_srf_pbes_equations(std::vector<srf_equation>& result, const srf_pbes& p)
{
  for (const auto& eqn : p.equations())
  {
    result.push_back(eqn);
  }
}

inline
std::unique_ptr<propositional_variable> mcrl2_srf_pbes_equation_variable(const srf_equation* equation)
{
  return std::make_unique<propositional_variable>(equation->variable());
}

// mcrl2::pbes_system::propositional_variable

inline
std::unique_ptr<atermpp::aterm> mcrl2_propositional_variable_parameters(const propositional_variable& variable)
{
  return std::make_unique<atermpp::aterm>(variable.parameters());
}

inline
rust::String mcrl2_propositional_variable_to_string(const propositional_variable& variable)
{
  std::stringstream ss;
  ss << variable;
  return ss.str();
}

} // namespace mcrl2::pbes_system