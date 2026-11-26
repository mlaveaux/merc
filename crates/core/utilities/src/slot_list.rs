/// A doubly linked list stored in a slot vector.
///
/// Every element is identified by a stable slot index, returned by
/// [SlotList::push_back], so that it can be moved or removed in constant time
/// without searching the list. Slots of removed elements are reused before
/// the vector grows.
#[derive(Debug)]
pub struct SlotList<T> {
    nodes: Vec<Node<T>>,
    /// Slots of removed elements, reused before growing `nodes`.
    free: Vec<usize>,
    /// The first slot, or [NIL].
    head: usize,
    /// The last slot, or [NIL].
    tail: usize,
}

/// Marks the absence of a neighbouring slot in [SlotList].
const NIL: usize = usize::MAX;

#[derive(Debug)]
struct Node<T> {
    /// `None` once the slot is free, so it does not keep the value alive.
    value: Option<T>,
    prev: usize,
    next: usize,
}

impl<T> SlotList<T> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
            head: NIL,
            tail: NIL,
        }
    }

    /// Returns true iff the list contains no elements.
    pub fn is_empty(&self) -> bool {
        self.head == NIL
    }

    /// Removes all elements.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.free.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    /// Appends `value` at the back and returns its slot.
    pub fn push_back(&mut self, value: T) -> usize {
        let node = Node {
            value: Some(value),
            prev: NIL,
            next: NIL,
        };

        let slot = match self.free.pop() {
            Some(slot) => {
                self.nodes[slot] = node;
                slot
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        };

        self.link_back(slot);
        slot
    }

    /// Moves the element in `slot` to the back.
    pub fn move_to_back(&mut self, slot: usize) {
        debug_assert!(self.nodes[slot].value.is_some(), "slot {slot} is not in use");
        self.unlink(slot);
        self.link_back(slot);
    }

    /// Removes and returns the element in `slot`.
    pub fn remove(&mut self, slot: usize) -> T {
        self.unlink(slot);
        self.free.push(slot);
        self.nodes[slot]
            .value
            .take()
            .expect("a linked slot always holds a value")
    }

    /// Removes and returns the first element, if any.
    pub fn pop_front(&mut self) -> Option<T> {
        (self.head != NIL).then(|| self.remove(self.head))
    }

    /// Removes the element in `slot` from the linked list, but does not free
    /// the slot.
    fn unlink(&mut self, slot: usize) {
        let (prev, next) = (self.nodes[slot].prev, self.nodes[slot].next);
        if prev == NIL {
            self.head = next;
        } else {
            self.nodes[prev].next = next;
        }

        if next == NIL {
            self.tail = prev;
        } else {
            self.nodes[next].prev = prev;
        }
    }

    /// Links the element in `slot` at the back of the linked list.
    fn link_back(&mut self, slot: usize) {
        self.nodes[slot].prev = self.tail;
        self.nodes[slot].next = NIL;
        if self.tail == NIL {
            self.head = slot;
        } else {
            self.nodes[self.tail].next = slot;
        }
        self.tail = slot;
    }
}

impl<T> Default for SlotList<T> {
    fn default() -> Self {
        Self::new()
    }
}
