//! The position state the two children iterators share.

use crate::error::Result;

/// Where a children iterator stands: reading position `next` of `len`, or
/// finished.
///
/// An error from a read ends the walk, so an iterator returns it once and
/// then `None` (as `SymbolicAtomIter` does), and the end is reached the same
/// way whether it came from the last child or from a failure. This lives
/// apart from the iterators because no operation of an entry can fail while
/// its borrow lives, so only a unit test, which feeds it an `Err`, can reach
/// the error arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Walk {
    Running { next: usize, len: usize },
    Done,
}

impl Walk {
    /// A walk over `len` positions.
    pub(crate) fn new(len: usize) -> Walk {
        Walk::Running { next: 0, len }
    }

    /// A walk with nothing to visit.
    pub(crate) fn finished() -> Walk {
        Walk::Done
    }

    /// Reads the next position with `read`, or returns `None` at the end.
    /// After an `Err` the walk is over.
    pub(crate) fn step<T>(&mut self, read: impl FnOnce(usize) -> Result<T>) -> Option<Result<T>> {
        let Walk::Running { next, len } = *self else {
            return None;
        };
        if next >= len {
            *self = Walk::Done;
            return None;
        }
        let item = read(next);
        *self = match item {
            Ok(_) => Walk::Running {
                next: next + 1,
                len,
            },
            Err(_) => Walk::Done,
        };
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Error, ErrorKind};

    fn fail(_: usize) -> Result<usize> {
        Err(Error::new(ErrorKind::Runtime, "no"))
    }

    #[test]
    fn a_walk_visits_each_position_once_and_ends() {
        let mut walk = Walk::new(3);
        let seen: Vec<usize> = std::iter::from_fn(|| walk.step(Ok))
            .map(Result::unwrap)
            .collect();
        assert_eq!(seen, [0, 1, 2]);
        for _ in 0..3 {
            assert!(walk.step(Ok).is_none());
        }
    }

    #[test]
    fn an_empty_walk_has_nothing() {
        for mut walk in [Walk::new(0), Walk::finished()] {
            for _ in 0..3 {
                assert!(walk.step(Ok).is_none());
            }
        }
    }

    #[test]
    fn an_error_is_returned_once_and_the_walk_ends() {
        let mut walk = Walk::new(5);
        assert_eq!(walk.step(Ok).unwrap().unwrap(), 0);
        assert!(walk.step(fail).unwrap().is_err());
        assert_eq!(walk, Walk::Done);
        for _ in 0..3 {
            let mut read = false;
            let item = walk.step(|i| {
                read = true;
                Ok(i)
            });
            assert!(item.is_none());
            assert!(!read, "a finished walk must not read again");
        }
    }
}
