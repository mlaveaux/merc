use std::cell::RefCell;
use std::hash::Hash;
use std::hash::Hasher;
use std::rc::Rc;

use ldd::LddIndex;
use ldd::SharedProtectionSet;
use merc_collections::IndexedSet;
use merc_collections::ProtectionSet;

use crate::operations::height;

mod cache;
mod ldd;

pub use self::cache::*;
pub use self::ldd::Ldd;
pub use self::ldd::LddRef;

pub type Value = u32;

/// This is the LDD node(value, down, right) with some additional meta data.
#[derive(Clone)]
pub struct Node {
    value: Value,
    down: LddIndex,
    right: LddIndex,

    marked: bool,
}

/// Check that the node size has the expected size.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<Node>() == std::mem::size_of::<(usize, usize, usize)>());

impl Node {
    fn new(value: Value, down: LddIndex, right: LddIndex) -> Node {
        Node {
            value,
            down,
            right,
            marked: false,
        }
    }

    /// Returns false if the node has been garbage collected.
    pub fn is_valid(&self) -> bool {
        true
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.down == other.down && self.right == other.right
    }
}

impl Eq for Node {}

impl Hash for Node {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        self.down.hash(state);
        self.right.hash(state);
    }
}

/// This is the user facing data of a [Node].
pub struct Data(pub Value, pub Ldd, pub Ldd);

/// This is the user facing data of a [Node] as references.
pub struct DataRef<'a>(pub Value, pub LddRef<'a>, pub LddRef<'a>);

/// The storage that implements the maximal sharing behaviour. Meaning that
/// identical nodes (same value, down and right) have a unique index in the node
/// table. Therefore guaranteeing that Ldds n and m are identical iff their
/// indices in the node table match.
pub struct Storage {
    protection_set: SharedProtectionSet, // Every Ldd points to the underlying protection set.
    nodes: IndexedSet<Node>,
    cache: OperationCache,

    count_until_collection: u64,     // Count down until the next garbage collection.
    enable_garbage_collection: bool, // Whether to enable automatic garbage collection based on heuristics.
    enable_performance_metrics: bool,
    empty_set: Ldd,
    empty_vector: Ldd,
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage {
    pub fn new() -> Self {
        let shared = Rc::new(RefCell::new(ProtectionSet::new()));
        // Add two nodes representing 'false' and 'true' respectively; these cannot be created using insert.
        let mut nodes = IndexedSet::new();
        let empty_set = nodes.insert(Node::new(0, LddIndex::default(), LddIndex::default())).0;
        let empty_vector = nodes.insert(Node::new(1, LddIndex::default(), LddIndex::default())).0;

        Self {
            protection_set: shared.clone(),
            nodes,
            cache: OperationCache::new(Rc::clone(&shared)),

            count_until_collection: 10000,
            enable_garbage_collection: true,
            enable_performance_metrics: false,
            empty_set: Ldd::new(&shared, empty_set),
            empty_vector: Ldd::new(&shared, empty_vector),
        }
    }

    /// Provides access to the underlying operation cache.
    pub fn operation_cache(&mut self) -> &mut OperationCache {
        &mut self.cache
    }

    /// Create a new LDD node(value, down, right)
    pub fn insert(&mut self, value: Value, down: &LddRef, right: &LddRef) -> Ldd {
        // These invariants ensure that the result is a valid LDD.
        debug_assert_ne!(down, self.empty_set(), "down node can never be the empty set.");
        debug_assert_ne!(right, self.empty_vector(), "right node can never be the empty vector.");
        debug_assert!(*down.index() < self.nodes.len(), "down node not in table.");
        debug_assert!(*right.index() < self.nodes.len(), "right not not in table.");

        if right != self.empty_set() {
            debug_assert_eq!(
                height(self, down) + 1,
                height(self, right),
                "height of node {} should match the right node {} height.",
                down.index(),
                right.index()
            );
            debug_assert!(value < self.value(right), "value should be less than right node value.");
        }

        if self.count_until_collection == 0 {
            if self.enable_garbage_collection {
                self.garbage_collect();
            }
            self.count_until_collection = self.nodes.len() as u64;
        }

        let (index, _inserted) = self.nodes.insert(Node::new(value, down.index(), right.index()));

        Ldd::new(&self.protection_set, index)
    }

    /// Upgrade an [LddRef] to a protected [Ldd] instance.
    pub fn protect(&mut self, ldd: &LddRef) -> Ldd {
        Ldd::new(&self.protection_set, ldd.index())
    }

    /// Cleans up all LDDs that are unreachable from the root LDDs.
    pub fn garbage_collect(&mut self) {
        // Clear the cache since it contains unprotected LDDs, and keep track of size before clearing.
        let size_of_cache = self.cache.len();
        self.cache.clear();
        self.cache.limit(self.nodes.len());

        // Mark all nodes that are (indirect) children of nodes with positive reference count.
        let mut stack: Vec<LddIndex> = Vec::new();
        for (_root, index) in self.protection_set.borrow().iter() {
            mark_node(&mut self.nodes, &mut stack, *index);
        }

        // Collect all garbage nodes.
        let mut number_of_collections: usize = 0;
        self.nodes.retain_mut(|_, node| {
            if node.marked {
                debug_assert!(node.is_valid(), "Should never mark a node that is not valid.");
                node.marked = false;
                true
            } else {
                number_of_collections += 1;
                false
            }
        });

        // Check whether the direct children of a valid node are valid (this implies that the whole tree is valid if the root is valid).
        for (_, node) in &self.nodes {
            // Special cases for the empty set and empty vector.
            debug_assert!(
                *node.down == 0 || self.nodes.get(node.down).is_some(),
                "The down node of a valid node must be valid."
            );
            debug_assert!(
                *node.right == 0 || self.nodes.get(node.right).is_some(),
                "The right node of a valid node must be valid."
            );
        }

        if self.enable_performance_metrics {
            println!(
                "Collected {number_of_collections} elements and {} elements remaining",
                self.nodes.len()
            );
            println!("Operation cache contains {size_of_cache} elements");
        }
    }

    /// Enables automatic garbage collection, which is enabled by default.
    pub fn enable_garbage_collection(&mut self, enabled: bool) {
        self.enable_garbage_collection = enabled;
    }

    pub fn enable_performance_metrics(&mut self, enabled: bool) {
        self.enable_performance_metrics = enabled;
    }

    /// The 'false' LDD.
    pub fn empty_set(&self) -> &Ldd {
        &self.empty_set
    }

    /// The 'true' LDD.
    pub fn empty_vector(&self) -> &Ldd {
        &self.empty_vector
    }

    /// The value of an LDD node(value, down, right). Note, ldd cannot be 'true' or 'false.
    pub fn value(&self, ldd: &LddRef) -> Value {
        self.verify_ldd(ldd);
        let node = &self.nodes[ldd.index()];
        node.value
    }

    /// The down of an LDD node(value, down, right). Note, ldd cannot be 'true' or 'false.
    pub fn down(&self, ldd: &LddRef) -> Ldd {
        self.verify_ldd(ldd);
        let node = &self.nodes[ldd.index()];
        Ldd::new(&self.protection_set, node.down)
    }

    /// The right of an LDD node(value, down, right). Note, ldd cannot be 'true' or 'false.
    pub fn right(&self, ldd: &LddRef) -> Ldd {
        self.verify_ldd(ldd);
        let node = &self.nodes[ldd.index()];
        Ldd::new(&self.protection_set, node.right)
    }

    /// Returns a Data tuple for the given LDD node(value, down, right). Note, ldd cannot be 'true' or 'false.
    pub fn get(&self, ldd: &LddRef) -> Data {
        self.verify_ldd(ldd);
        let node = &self.nodes[ldd.index()];
        Data(
            node.value,
            Ldd::new(&self.protection_set, node.down),
            Ldd::new(&self.protection_set, node.right),
        )
    }

    /// Returns a DataRef tuple for the given LDD node(value, down, right). Note, ldd cannot be 'true' or 'false.
    pub fn get_ref<'a>(&self, ldd: &'a LddRef) -> DataRef<'a> {
        self.verify_ldd(ldd);
        let node = &self.nodes[ldd.index()];
        DataRef(node.value, LddRef::new(node.down), LddRef::new(node.right))
    }

    // Asserts whether the given ldd is valid.
    fn verify_ldd(&self, ldd: &LddRef) {
        debug_assert_ne!(ldd, self.empty_set(), "Cannot inspect empty set.");
        debug_assert_ne!(ldd, self.empty_vector(), "Cannot inspect empty vector.");
        debug_assert!(
            self.nodes.get(ldd.index()).is_some(),
            "Node {} should not have been garbage collected",
            ldd.index()
        );
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if self.enable_performance_metrics {
            println!(
                "There were {} insertions into the protection set.",
                self.protection_set.borrow().number_of_insertions()
            );
            println!(
                "There were at most {} root variables.",
                self.protection_set.borrow().maximum_size()
            );
            println!("There were at most {} nodes.", self.nodes.capacity());
        }
    }
}

/// Mark all LDDs reachable from the given root index.
///
/// Reuses the stack for the depth-first exploration.
fn mark_node(nodes: &mut IndexedSet<Node>, stack: &mut Vec<LddIndex>, root: LddIndex) {
    stack.push(root);
    while let Some(current) = stack.pop() {
        let node = &mut nodes[current];
        debug_assert!(node.is_valid(), "Should never mark a node that is not valid.");
        if node.marked {
            continue;
        } else {
            node.marked = true;
            if *current != 0 && *current != 1 {
                stack.push(node.down);
                stack.push(node.right);
            }
        }
    }

    debug_assert!(stack.is_empty(), "When marking finishes the stack should be empty.");
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::operations::singleton;
    use crate::test_utility::*;

    use merc_utilities::random_test;

    #[test]
    fn test_random_garbage_collection_small() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let _child: Ldd;
            {
                // Make sure that this set goes out of scope, but keep a reference to some child ldd.
                let vector = random_vector(rng, 10, 10);
                let ldd = singleton(&mut storage, &vector);

                _child = storage.get(&ldd).1;
                storage.garbage_collect();
            }

            storage.garbage_collect();
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_garbage_collection() {
        random_test(20, |rng| {
            let mut storage = Storage::new();

            let _child: Ldd;
            {
                // Make sure that this set goes out of scope, but keep a reference to some child ldd.
                let vector = random_vector_set(rng, 2000, 10, 2);
                let ldd = from_iter(&mut storage, vector.iter());

                _child = storage.get(&storage.get(&ldd).1).1;
                storage.garbage_collect();
            }

            storage.garbage_collect();
        });
    }
}
