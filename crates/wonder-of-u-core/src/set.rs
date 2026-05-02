//! HashSet convenience helpers.

use std::{collections::HashSet, hash::Hash};

/// Returns the union of two sets.
pub fn union<T>(left: &HashSet<T>, right: &HashSet<T>) -> HashSet<T>
where
    T: Clone + Eq + Hash,
{
    let mut merged = left.clone();
    merged.extend(right.iter().cloned());
    merged
}

/// Returns the intersection of two sets.
pub fn intersection<T>(left: &HashSet<T>, right: &HashSet<T>) -> HashSet<T>
where
    T: Clone + Eq + Hash,
{
    let (small, large) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };

    small
        .iter()
        .filter(|item| large.contains(*item))
        .cloned()
        .collect()
}

/// Returns the items in `left` that are not present in `right`.
pub fn difference<T>(left: &HashSet<T>, right: &HashSet<T>) -> HashSet<T>
where
    T: Clone + Eq + Hash,
{
    left.iter()
        .filter(|item| !right.contains(*item))
        .cloned()
        .collect()
}

/// Returns whether two sets share at least one element.
pub fn intersects<T>(left: &HashSet<T>, right: &HashSet<T>) -> bool
where
    T: Eq + Hash,
{
    let (small, large) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };

    small.iter().any(|item| large.contains(item))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{difference, intersection, intersects, union};

    #[test]
    fn performs_set_operations() {
        let left = HashSet::from([1, 2, 3]);
        let right = HashSet::from([3, 4]);

        assert_eq!(union(&left, &right), HashSet::from([1, 2, 3, 4]));
        assert_eq!(intersection(&left, &right), HashSet::from([3]));
        assert_eq!(difference(&left, &right), HashSet::from([1, 2]));
        assert!(intersects(&left, &right));
    }
}
