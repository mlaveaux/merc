/// Wrapper around the PBES library of the mCRL2 toolset.

#pragma once

#include "mcrl2/atermpp/aterm.h"
#include "mcrl2/data/data_specification.h"
#include "mcrl2/pbes/detail/stategraph_local_algorithm.h"
#include "mcrl2/pbes/detail/stategraph_pbes.h"
#include "mcrl2/pbes/io.h"
#include "mcrl2/pbes/pbes.h"
#include "mcrl2/pbes/propositional_variable.h"
#include "mcrl2/pbes/srf_pbes.h"
#include "mcrl2/pbes/unify_parameters.h"

#include "mcrl2-sys/cpp/assert.h"
#include "mcrl2-sys/cpp/atermpp.h"
#include "rust/cxx.h"

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

namespace mcrl2::pbes_system
{

/// Alias for templated type.
using srf_equation = detail::pre_srf_equation<false>;

struct vertex_outgoing_edge;
struct assignment_pair;

// mcrl2::pbes_system::pbes

inline 
std::unique_ptr<pbes> mcrl2_load_pbes_from_pbes_file(rust::Str filename)
{
  pbes result;
  load_pbes(result, static_cast<std::string>(filename));
  return std::make_unique<pbes>(result);
}

inline 
std::unique_ptr<pbes> mcrl2_load_pbes_from_text_file(rust::Str filename)
{
  pbes result;
  load_pbes(result, static_cast<std::string>(filename), pbes_format_text());
  return std::make_unique<pbes>(result);
}

inline 
std::unique_ptr<pbes> mcrl2_load_pbes_from_text(rust::Str input)
{
  pbes result;
  std::istringstream stream(static_cast<std::string>(input));
  load_pbes(result, stream, pbes_format_text());
  return std::make_unique<pbes>(result);
}

inline
std::unique_ptr<data::data_specification> mcrl2_pbes_data_specification(const pbes& pbesspec)
{
  return std::make_unique<data::data_specification>(pbesspec.data());
}

inline
rust::String mcrl2_pbes_to_string(const pbes& pbesspec)
{
  std::stringstream ss;
  ss << pbesspec;
  return ss.str();
}

inline
rust::String mcrl2_pbes_expression_to_string(const atermpp::aterm& expr)
{
  std::stringstream ss;
  ss << expr;
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

  const std::vector<detail::stategraph_equation>& equations() const
  {
    return m_pbes.equations();
  }
};

inline
std::unique_ptr<stategraph_algorithm> mcrl2_stategraph_local_algorithm_run(const pbes& p)
{
  auto algorithm = std::make_unique<stategraph_algorithm>(p);
  algorithm->run();
  return algorithm;
}

inline
void mcrl2_local_control_flow_graph_vertices(std::vector<detail::local_control_flow_graph_vertex>& result,
    const detail::local_control_flow_graph& cfg)
{
  for (const auto& v : cfg.vertices)
  {
    result.push_back(v);
  }
}

// namespace mcrl2::pbes_system::detail::local_control_flow_graph_vertex

inline
std::size_t mcrl2_local_control_flow_graph_vertex_index(
    const detail::local_control_flow_graph_vertex& vertex)
{
  return vertex.index();
}

inline
const atermpp::detail::_aterm* mcrl2_local_control_flow_graph_vertex_name(
    const detail::local_control_flow_graph_vertex& vertex)
{
  return atermpp::detail::address(vertex.name());
}

inline
const atermpp::detail::_aterm* mcrl2_local_control_flow_graph_vertex_value(
    const detail::local_control_flow_graph_vertex& vertex)
{
  return atermpp::detail::address(vertex.value());
}

void mcrl2_local_control_flow_graph_vertex_outgoing_edges(std::vector<vertex_outgoing_edge>& result,
    const detail::local_control_flow_graph_vertex& vertex);

void mcrl2_local_control_flow_graph_vertex_incoming_edges(std::vector<vertex_outgoing_edge>& result,
    const detail::local_control_flow_graph_vertex& vertex);


inline
void mcrl2_stategraph_local_algorithm_cfgs(std::vector<detail::local_control_flow_graph>& result,
    const stategraph_algorithm& algorithm)
{
  for (const auto& cfg : algorithm.local_control_flow_graphs())
  {
    result.push_back(cfg);
  }
}

inline
void mcrl2_stategraph_local_algorithm_equations(std::vector<detail::stategraph_equation>& result,
    const stategraph_algorithm& algorithm)
{
  for (const auto& eqn : algorithm.equations())
  {
    result.push_back(eqn);
  }
}


inline
const atermpp::detail::_aterm* mcrl2_stategraph_equation_variable(const detail::stategraph_equation& equation)
{
  return atermpp::detail::address(equation.variable());
}

inline
std::unique_ptr<srf_pbes> mcrl2_pbes_to_srf_pbes(const pbes& p)
{
  return std::make_unique<srf_pbes>(pbes2srf(p));
}

inline
void mcrl2_srf_pbes_unify_parameters(srf_pbes& p, bool ignore_ce_equations, bool reset)
{
  unify_parameters(p, ignore_ce_equations, reset);
}

// mcrl2::pbes_system::detail::predicate_variable

inline
void mcrl2_stategraph_equation_predicate_variables(std::vector<detail::predicate_variable>& result,
    const detail::stategraph_equation& eqn)
{
  for (const auto& v : eqn.predicate_variables())
  {
    result.push_back(v);
  }
}

inline
rust::Vec<std::size_t> mcrl2_predicate_variable_used(const detail::predicate_variable& v)
{
  rust::Vec<std::size_t> result;
  for (const auto& index : v.used())
  {
    result.push_back(index);
  }
  return result;
}

inline
rust::Vec<std::size_t> mcrl2_predicate_variable_changed(const detail::predicate_variable& v)
{
  rust::Vec<std::size_t> result;
  for (const auto& index : v.changed())
  {
    result.push_back(index);
  }
  return result;
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
const atermpp::detail::_aterm* mcrl2_srf_pbes_equation_variable(const srf_equation& equation)
{
  return atermpp::detail::address(equation.variable());
}

// mcrl2::pbes_system::propositional_variable

inline
bool mcrl2_pbes_is_propositional_variable(const atermpp::detail::_aterm& variable)
{
  atermpp::unprotected_aterm_core tmp(&variable);
  return pbes_system::is_propositional_variable(atermpp::down_cast<atermpp::aterm>(tmp));
}

inline
rust::String mcrl2_propositional_variable_to_string(const atermpp::aterm& variable)
{
  MCRL2_ASSERT(pbes_system::is_propositional_variable(variable));
  std::stringstream ss;
  ss << atermpp::down_cast<propositional_variable>(variable);
  return ss.str();
}

inline
void mcrl2_srf_equations_summands(std::vector<srf_summand>& result, const srf_equation& equation)
{
  for (const auto& summand : equation.summands())
  {
    result.push_back(summand);
  }
}

inline
const atermpp::detail::_aterm* mcrl2_srf_summand_variable(const srf_summand& summand)
{
  return atermpp::detail::address(summand.variable());
}

inline
const atermpp::detail::_aterm* mcrl2_srf_summand_condition(const srf_summand& summand)
{
  return atermpp::detail::address(summand.variable());
}

std::unique_ptr<atermpp::aterm> mcrl2_pbes_expression_replace_variables(const atermpp::detail::_aterm& expr, const rust::Vec<assignment_pair>& sigma);

std::unique_ptr<atermpp::aterm> mcrl2_pbes_expression_replace_propositional_variables(const atermpp::detail::_aterm& expr, const rust::Vec<std::size_t>& pi);

} // namespace mcrl2::pbes_system