use std::slice::SliceIndex;

/// An enum used to indicate an edge or a self loop.
pub enum Edge<T> {
    Regular(T, T),

    /// For a self loop we only provide a mutable reference to the single state.
    Selfloop(T),
}

/// Index two locations (from, to) of an edge, returns mutable references to it.
pub fn index_edge<T, I: PartialEq + PartialOrd<usize> + SliceIndex<[T], Output = T>>(
    slice: &mut [T],
    a: I,
    b: I,
) -> Edge<&mut T> {
    if a == b {
        assert!(a < slice.len());
        Edge::Selfloop(slice.get_mut(a).expect("The index is in bounds, asserted above"))
    } else {
        assert!(a < slice.len() && b < slice.len());

        // SAFETY: `a` and `b` are in bounds (asserted) and distinct, so the two
        // `&mut` borrow disjoint elements and do not alias.
        unsafe {
            let ar = &mut *(slice.get_unchecked_mut(a) as *mut _);
            let br = &mut *(slice.get_unchecked_mut(b) as *mut _);
            Edge::Regular(ar, br)
        }
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    const LEN: usize = 4;

    #[kani::proof]
    fn index_edge_self_loop_returns_selfloop() {
        let mut data: [u32; LEN] = kani::any();
        let i: usize = kani::any();
        kani::assume(i < LEN);

        match index_edge(&mut data, i, i) {
            Edge::Selfloop(_) => {}
            Edge::Regular(_, _) => panic!("self-loop produced Regular"),
        }
    }

    #[kani::proof]
    fn index_edge_distinct_returns_regular() {
        let mut data: [u32; LEN] = kani::any();
        let a: usize = kani::any();
        let b: usize = kani::any();
        kani::assume(a < LEN);
        kani::assume(b < LEN);
        kani::assume(a != b);

        match index_edge(&mut data, a, b) {
            Edge::Regular(_, _) => {}
            Edge::Selfloop(_) => panic!("distinct indices produced Selfloop"),
        }
    }

    #[kani::proof]
    fn index_edge_regular_writes_are_independent() {
        let mut data: [u32; LEN] = [0; LEN];
        let a: usize = kani::any();
        let b: usize = kani::any();
        kani::assume(a < LEN);
        kani::assume(b < LEN);
        kani::assume(a != b);

        let va: u32 = kani::any();
        let vb: u32 = kani::any();

        if let Edge::Regular(ar, br) = index_edge(&mut data, a, b) {
            *ar = va;
            *br = vb;
        }

        assert_eq!(data[a], va);
        assert_eq!(data[b], vb);
    }
}
