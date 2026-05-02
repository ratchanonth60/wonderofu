//! Async helpers for sequential execution.

use std::future::Future;

/// Runs async functions sequentially and collects each result.
pub async fn run_sequential<I, F, Fut, T, E>(fns: I) -> Vec<Result<T, E>>
where
    I: IntoIterator<Item = F>,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut results = Vec::new();

    for f in fns {
        results.push(f().await);
    }

    results
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use futures::executor::block_on;

    use super::run_sequential;

    #[test]
    fn runs_tasks_in_order() {
        let visited = Rc::new(RefCell::new(Vec::new()));
        let tasks = (0..3)
            .map(|index| {
                let visited = Rc::clone(&visited);
                move || async move {
                    visited.borrow_mut().push(index);
                    Ok::<_, ()>(index * 2)
                }
            })
            .collect::<Vec<_>>();

        let results = block_on(run_sequential(tasks));

        assert_eq!(visited.borrow().as_slice(), &[0, 1, 2]);
        assert_eq!(results, vec![Ok(0), Ok(2), Ok(4)]);
    }
}
