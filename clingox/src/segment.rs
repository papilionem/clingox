//! One step of a configuration or statistics path.

use std::fmt;

/// One step of a configuration or statistics path: a map entry's name or an
/// array element's index. `Display` writes it as a path writes it, so
/// segments joined with `.` form a path the path methods accept.
///
/// The children iterators [`ConfigChildren`](crate::ConfigChildren) and
/// [`StatsChildren`](crate::StatsChildren) yield one with each child, so that
/// a caller who needs the path of an entry, to [`set`](crate::Configuration::set)
/// a value for instance, can build it without the entry storing one.
///
/// The derived [`Ord`] puts every [`Name`](PathSegment::Name) before every
/// [`Index`](PathSegment::Index) and compares like with like. It is a stable
/// order for keys of a map or set and nothing more: it is not the order of a
/// path, and clingo's own order is the one the iterators yield.
///
/// # Examples
///
/// ```
/// use clingox::PathSegment;
///
/// let segments = [
///     PathSegment::Name("solver".to_owned()),
///     PathSegment::Index(0),
///     PathSegment::Name("seed".to_owned()),
/// ];
/// let path: Vec<String> = segments.iter().map(ToString::to_string).collect();
/// assert_eq!(path.join("."), "solver.0.seed");
///
/// let mut ctl = clingox::Control::new()?;
/// assert_eq!(ctl.configuration().get(&path.join("."))?.as_deref(), Some("1"));
/// # Ok::<(), clingox::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathSegment {
    /// The name of a map entry, as clingo lists it.
    Name(String),
    /// The index of an array element.
    Index(usize),
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathSegment::Name(name) => f.write_str(name),
            PathSegment::Index(index) => write!(f, "{index}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PathSegment::{Index, Name};
    use super::*;

    #[test]
    fn a_name_displays_as_it_is() {
        assert_eq!(Name("models".to_owned()).to_string(), "models");
    }

    #[test]
    fn an_index_displays_in_decimal() {
        assert_eq!(Index(0).to_string(), "0");
        assert_eq!(Index(3).to_string(), "3");
        assert_eq!(Index(usize::MAX).to_string(), usize::MAX.to_string());
    }

    #[test]
    fn a_name_that_looks_like_a_number_is_still_a_name() {
        assert_eq!(Name("0".to_owned()).to_string(), Index(0).to_string());
        assert_ne!(Name("0".to_owned()), Index(0));
    }

    #[test]
    fn segments_joined_with_a_period_form_a_path() {
        let segments = [Name("solver".to_owned()), Index(0), Name("seed".to_owned())];
        let parts: Vec<String> = segments.iter().map(ToString::to_string).collect();
        assert_eq!(parts.join("."), "solver.0.seed");
    }

    #[test]
    fn path_segments_are_send_sync_and_static() {
        fn traits<T: Send + Sync + 'static + Clone + Eq + std::hash::Hash + Ord + fmt::Debug>() {}
        traits::<PathSegment>();
    }
}
