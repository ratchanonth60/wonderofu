//! Thin iterator helpers that mirror common utility wrappers.

/// Returns the first `n` items from an iterator.
pub fn take<I>(n: usize, iter: I) -> std::iter::Take<I::IntoIter>
where
    I: IntoIterator,
{
    iter.into_iter().take(n)
}

/// Zips two iterators together.
pub fn zip<A, B>(left: A, right: B) -> std::iter::Zip<A::IntoIter, B::IntoIter>
where
    A: IntoIterator,
    B: IntoIterator,
{
    left.into_iter().zip(right)
}

/// Enumerates an iterator.
pub fn enumerate<I>(iter: I) -> std::iter::Enumerate<I::IntoIter>
where
    I: IntoIterator,
{
    iter.into_iter().enumerate()
}

#[cfg(test)]
mod tests {
    use super::{enumerate, take, zip};

    #[test]
    fn takes_items() {
        assert_eq!(take(2, [1, 2, 3]).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn zips_iterators() {
        assert_eq!(
            zip(["a", "b"], [1, 2]).collect::<Vec<_>>(),
            vec![("a", 1), ("b", 2)]
        );
    }

    #[test]
    fn enumerates_iterators() {
        assert_eq!(
            enumerate(["x", "y"]).collect::<Vec<_>>(),
            vec![(0, "x"), (1, "y")]
        );
    }
}
