/// Converts a vector to an empty one of another element type, dropping its elements.
///
/// The standard library collects `into_iter().map(..)` in place when both element types have the
/// same size and alignment, which keeps the allocation; otherwise this merely frees it.
pub fn recycle_vec<T, U>(mut vec: Vec<T>) -> Vec<U> {
    vec.clear();
    vec.into_iter().map(|_| unreachable!("the vector is empty")).collect()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::recycle_vec;

    #[test]
    fn test_recycle_vec_keeps_allocation_for_same_layout() {
        let mut vec: Vec<u64> = Vec::with_capacity(16);
        vec.extend([1, 2, 3]);
        let ptr = vec.as_ptr() as usize;

        let recycled: Vec<i64> = recycle_vec(vec);
        assert!(recycled.is_empty());
        assert_eq!(recycled.capacity(), 16);
        assert_eq!(recycled.as_ptr() as usize, ptr);
    }

    #[test]
    fn test_recycle_vec_drops_elements() {
        let shared = Rc::new(());
        let vec = vec![shared.clone(), shared.clone()];
        assert_eq!(Rc::strong_count(&shared), 3);

        let recycled: Vec<Rc<()>> = recycle_vec(vec);
        assert!(recycled.is_empty());
        assert_eq!(Rc::strong_count(&shared), 1);
    }
}
