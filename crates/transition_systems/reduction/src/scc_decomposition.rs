#![forbid(unsafe_code)]

use merc_collections::IndexedPartition;

use merc_collections::scc_decomposition;
use merc_collections::scc_decomposition_iterative;
use merc_lts::AsGraph;
use merc_lts::LTS;

/// Computes the strongly connected tau component partitioning of the given LTS.
pub fn tau_scc_decomposition<L: LTS>(lts: &L) -> IndexedPartition {
    scc_decomposition(&AsGraph(lts), |_, label_index, _| lts.is_hidden_label(label_index))
}

/// Computes the strongly connected tau component partitioning of the given LTS using the iterative algorithm.
///
/// Prefer this over [`tau_scc_decomposition`] on LTSs whose size is not bounded ahead of time:
/// the recursive algorithm's stack depth grows with the longest tau-path, which can overflow the
/// native stack on large inputs.
pub fn tau_scc_decomposition_iterative<L: LTS>(lts: &L) -> IndexedPartition {
    scc_decomposition_iterative(&AsGraph(lts), |_, label_index, _| lts.is_hidden_label(label_index))
}
