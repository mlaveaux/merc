use log::debug;
use log::trace;
use merc_utilities::MercIndex;

use crate::BlockIndex;
use crate::Graph;
use crate::IndexedPartition;
use merc_io::LargeFormatter;

/// Computes the strongly connected component partitioning of the given graph.
///
/// Uses recursive Tarjan's algorithm. For large graphs that may cause a stack overflow,
/// prefer [`scc_decomposition_iterative`] instead.
///
/// # Panics
///
/// In debug builds, panics if a vertex index is `>= graph.num_of_vertices()`.
pub fn scc_decomposition<F, G>(graph: &G, filter: F) -> IndexedPartition
where
    F: Fn(G::VertexIndex, G::LabelIndex, G::VertexIndex) -> bool,
    G: Graph,
    G::VertexIndex: MercIndex<Target = usize>,
{
    // We assume that the graph is dense.
    debug_assert!(
        graph.iter_vertices().all(|v| v.index() < graph.num_of_vertices()),
        "The graph contains vertices with indices larger than the number of vertices"
    );

    let mut partition =
        IndexedPartition::with_subset(graph.num_of_vertices(), graph.iter_vertices().map(|v| v.index()));

    // The stack for the depth first search.
    let mut stack = Vec::new();

    // Keep track of already visited states.
    let mut state_info: Vec<Option<StateInfo>> = vec![None; graph.num_of_vertices()];

    let mut smallest_index = 0;
    let mut next_block_number = BlockIndex::new(0);

    // The outer depth first search used to traverse all the states.
    for state_index in graph.iter_vertices() {
        if state_info[state_index.index()].is_none() {
            trace!("State {}", state_index.index());

            strongly_connect(
                state_index,
                graph,
                &filter,
                &mut partition,
                &mut smallest_index,
                &mut next_block_number,
                &mut stack,
                &mut state_info,
            )
        }
    }

    trace!("SCC partition {partition}");
    debug!(
        "Found {} strongly connected components",
        LargeFormatter(partition.num_of_blocks())
    );
    partition
}

/// Computes the strongly connected component partitioning of the given graph using an iterative
/// algorithm, based on the iterative Tarjan's SCC algorithm from the mCRL2 toolset (Wouter Schols).
///
/// # Panics
///
/// In debug builds, panics if a vertex index is `>= graph.num_of_vertices()`.
pub fn scc_decomposition_iterative<F, G>(graph: &G, filter: F) -> IndexedPartition
where
    F: Fn(G::VertexIndex, G::LabelIndex, G::VertexIndex) -> bool,
    G: Graph,
    G::VertexIndex: MercIndex<Target = usize>,
{
    debug_assert!(
        graph.iter_vertices().all(|v| v.index() < graph.num_of_vertices()),
        "The graph contains vertices with indices larger than the number of vertices"
    );

    let n = graph.num_of_vertices();

    let mut partition = IndexedPartition::with_subset(n, graph.iter_vertices().map(|v| v.index()));

    // Sentinel value indicating not yet visited.
    const UNVISITED: usize = usize::MAX;

    let mut low = vec![UNVISITED; n];
    // disc[v] == UNVISITED means never queued; disc[v] == 0 means queued but not yet initialized.
    let mut disc = vec![UNVISITED; n];
    let mut on_scc_stack = vec![false; n];
    let mut scc_stack: Vec<usize> = Vec::new();
    let mut discovery_time = 0usize;
    let mut eq_class = 0usize;

    // Work stack: (vertex, the remaining outgoing edges of that vertex). Keeping
    // the iterator (instead of an offset that is skipped on resumption) makes
    // the traversal linear in the number of edges, also for high out-degrees.
    let mut work = Vec::new();

    for root in graph.iter_vertices() {
        let s0 = root.index();
        if low[s0] != UNVISITED {
            continue;
        }

        work.push((root, graph.outgoing_edges(root)));

        while let Some((s_vertex, mut edges)) = work.pop() {
            let s = s_vertex.index();

            if low[s] == UNVISITED {
                disc[s] = discovery_time;
                low[s] = discovery_time;
                discovery_time += 1;
                scc_stack.push(s);
                on_scc_stack[s] = true;
            }

            let mut child = None;
            for (label, to) in edges.by_ref() {
                if filter(s_vertex, label, to) {
                    let v = to.index();
                    if disc[v] == UNVISITED {
                        disc[v] = 0; // Mark as queued to prevent double-pushing.
                        child = Some(to);
                        break;
                    } else if on_scc_stack[v] {
                        low[s] = low[s].min(disc[v]);
                    }
                }
            }

            if let Some(child_vertex) = child {
                // Push current state continuation, then recurse on child_vertex.
                work.push((s_vertex, edges));
                work.push((child_vertex, graph.outgoing_edges(child_vertex)));
            } else {
                if disc[s] == low[s] {
                    // s is the root of an SCC; pop all members off the SCC stack.
                    loop {
                        let u = scc_stack.pop().expect("scc_stack must not be empty");
                        on_scc_stack[u] = false;
                        trace!("Added state {} to block {}", u, eq_class);
                        partition.set_block(u, BlockIndex::new(eq_class));
                        if u == s {
                            break;
                        }
                    }
                    eq_class += 1;
                }
                // Propagate lowlink to parent.
                if let Some((parent_vertex, _)) = work.last() {
                    let p = parent_vertex.index();
                    low[p] = low[p].min(low[s]);
                }
            }
        }
    }

    trace!("SCC partition {partition}");
    debug!(
        "Found {} strongly connected components",
        LargeFormatter(partition.num_of_blocks())
    );
    partition
}

/// Information about a state during the SCC computation.
#[derive(Clone, Debug)]
struct StateInfo {
    /// A unique index for every state.
    index: usize,

    /// Keeps track of the lowest state that can be reached on the stack.
    lowlink: usize,

    /// Keeps track of whether this state is on the stack.
    on_stack: bool,
}

/// Tarjan's strongly connected components algorithm.
///
/// The `filter` determines which (from, label, to) edges are followed.
///
/// The `smallest_index`, `stack` and `state_info` are updated in each recursive
/// call to keep track of the current SCC.
#[allow(clippy::too_many_arguments)]
fn strongly_connect<F, G>(
    vertex_index: G::VertexIndex,
    lts: &G,
    filter: &F,
    partition: &mut IndexedPartition,
    smallest_index: &mut usize,
    next_block_number: &mut BlockIndex,
    stack: &mut Vec<G::VertexIndex>,
    state_info: &mut Vec<Option<StateInfo>>,
) where
    F: Fn(G::VertexIndex, G::LabelIndex, G::VertexIndex) -> bool,
    G: Graph,
    G::VertexIndex: MercIndex<Target = usize>,
{
    trace!("Visiting state {}", vertex_index.index());

    state_info[vertex_index.index()] = Some(StateInfo {
        index: *smallest_index,
        lowlink: *smallest_index,
        on_stack: true,
    });

    *smallest_index += 1;

    // Start a depth first search from the current state.
    stack.push(vertex_index);

    // Consider successors of the current state.
    for (label, to) in lts.outgoing_edges(vertex_index) {
        if filter(vertex_index, label, to) {
            if let Some(meta) = &mut state_info[to.index()] {
                if meta.on_stack {
                    // Successor w is in stack S and hence in the current SCC
                    // If w is not on stack, then (v, w) is an edge pointing to an SCC already found and must be ignored
                    // v.lowlink := min(v.lowlink, w.lowlink);
                    let w_index = state_info[to.index()]
                        .as_ref()
                        .expect("The state must be visited in the recursive call")
                        .index;
                    let info = state_info[vertex_index.index()]
                        .as_mut()
                        .expect("This state was added before");
                    info.lowlink = info.lowlink.min(w_index);
                }
            } else {
                // Successor w has not yet been visited; recurse on it
                strongly_connect(
                    to,
                    lts,
                    filter,
                    partition,
                    smallest_index,
                    next_block_number,
                    stack,
                    state_info,
                );

                // v.lowlink := min(v.lowlink, w.lowlink);
                let w_lowlink = state_info[to.index()]
                    .as_ref()
                    .expect("The state must be visited in the recursive call")
                    .lowlink;
                let info = state_info[vertex_index.index()]
                    .as_mut()
                    .expect("This state was added before");
                info.lowlink = info.lowlink.min(w_lowlink);
            }
        }
    }

    let info = state_info[vertex_index.index()]
        .as_ref()
        .expect("This state was added before");
    if info.lowlink == info.index {
        // Start a new strongly connected component.
        while let Some(index) = stack.pop() {
            let info = state_info[index.index()].as_mut().expect("This state was on the stack");
            info.on_stack = false;

            trace!("Added state {} to block {}", index.index(), next_block_number);
            partition.set_block(index.index(), *next_block_number);

            if index == vertex_index || stack.is_empty() {
                *next_block_number = BlockIndex::new(next_block_number.value() + 1);
                break;
            }
        }
    }
}
