//! Conversions: `ToSymbol` and `FromSymbol` for `Box`, `ToSymbol`
//! for `Rc` and `Arc`, and hand-written impls that report
//! mismatches with `Error::conversion`.
//!
//! Symbol texts were checked against the Python module `clingo` 5.8.2.

#![forbid(unsafe_code)]

use std::rc::Rc;
use std::sync::Arc;

use clingox::{Control, Error, ErrorKind, FromSymbol, Outcome, Part, Predicate, Symbol, ToSymbol};

fn text(value: &impl ToSymbol) -> String {
    value.to_symbol().expect("the value converts").to_string()
}

// ---------------------------------------------------------------------------
// Smart pointers

#[test]
fn a_box_converts_as_its_content() {
    assert_eq!(text(&Box::new(5)), "5");
    assert_eq!(text(&Box::new((1, 2))), "(1,2)");
    let slice: Box<[i32]> = vec![1, 2, 3].into_boxed_slice();
    assert_eq!(text(&slice), "(1,2,3)");
    let symbol: Box<Symbol> = Box::new("p(1)".parse().unwrap());
    assert_eq!(text(&symbol), "p(1)");

    let back = Box::<i32>::from_symbol(Symbol::number(7)).unwrap();
    assert_eq!(*back, 7);
    let err = Box::<i32>::from_symbol(Symbol::string("x").unwrap()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
}

#[test]
fn rc_and_arc_convert_as_their_content() {
    assert_eq!(text(&Rc::new(-4)), "-4");
    assert_eq!(text(&Arc::new(vec![1, 2])), "(1,2)");
    let shared: Rc<[i32]> = Rc::from(vec![3, 4]);
    assert_eq!(text(&shared), "(3,4)");
    let shared: Arc<[i32]> = Arc::from(vec![5]);
    assert_eq!(text(&shared), "(5,)");
}

#[test]
fn a_range_error_passes_through_the_pointer() {
    let err = Box::new(u64::MAX).to_symbol().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    let err = Arc::new(i64::MIN).to_symbol().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
}

#[cfg(feature = "derive")]
mod derived {
    use super::*;

    /// A recursive term needs `Box`.
    #[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
    enum Tree {
        Leaf(i32),
        Node(Box<Tree>, Box<Tree>),
    }

    #[test]
    fn a_recursive_enum_converts_through_box() {
        let tree = Tree::Node(
            Box::new(Tree::Leaf(1)),
            Box::new(Tree::Node(Box::new(Tree::Leaf(2)), Box::new(Tree::Leaf(3)))),
        );
        let symbol = tree.to_symbol().unwrap();
        assert_eq!(symbol.to_string(), "node(leaf(1),node(leaf(2),leaf(3)))");
        assert_eq!(Tree::from_symbol(symbol).unwrap(), tree);
    }

    #[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
    struct Shared {
        id: Box<i32>,
    }

    #[test]
    fn a_boxed_field_reads_and_writes_as_its_content() {
        let fact = Shared { id: Box::new(4) };
        assert_eq!(fact.to_symbol().unwrap().to_string(), "shared(4)");
        assert_eq!(
            Shared::from_symbol("shared(4)".parse().unwrap()).unwrap(),
            fact
        );
    }
}

// ---------------------------------------------------------------------------
// Hand-written impls with Error::conversion

/// A colour, written by hand as the constants `red` and `green`.
#[derive(Debug, PartialEq)]
enum Colour {
    Red,
    Green,
}

impl ToSymbol for Colour {
    fn to_symbol(&self) -> clingox::Result<Symbol> {
        Symbol::function(
            match self {
                Colour::Red => "red",
                Colour::Green => "green",
            },
            &[],
        )
    }
}

impl FromSymbol for Colour {
    fn from_symbol(symbol: Symbol) -> clingox::Result<Colour> {
        match symbol.to_string().as_str() {
            "red" => Ok(Colour::Red),
            "green" => Ok(Colour::Green),
            _ => Err(Error::conversion(format!("`{symbol}` is not a colour"))),
        }
    }
}

/// `colour(Node, Colour)`, by hand.
#[derive(Debug, PartialEq)]
struct Coloured(i32, Colour);

impl FromSymbol for Coloured {
    fn from_symbol(symbol: Symbol) -> clingox::Result<Coloured> {
        match symbol.arguments() {
            Some([node, colour]) if symbol.name() == Some("colour") => Ok(Coloured(
                i32::from_symbol(*node)?,
                Colour::from_symbol(*colour)?,
            )),
            _ => Err(Error::conversion(format!("`{symbol}` is not colour/2"))),
        }
    }
}

impl Predicate for Coloured {
    const NAME: &'static str = "colour";
    const ARITY: u32 = 2;
}

#[test]
fn a_hand_written_impl_reports_a_mismatch_as_a_conversion_error() {
    assert_eq!(
        Colour::from_symbol("green".parse().unwrap()).unwrap(),
        Colour::Green
    );
    let err = Colour::from_symbol("blue".parse().unwrap()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(err.to_string(), "`blue` is not a colour");
}

#[test]
fn typed_reading_passes_a_hand_written_conversion_error_on() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("colour(1,red). colour(2,blue).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
        panic!("the program has a model");
    };
    let err = model.atoms::<Coloured>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert!(err.to_string().contains("`blue` is not a colour"), "{err}");

    let mut ctl = Control::new().unwrap();
    ctl.add_base("colour(1,red). colour(2,green).").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let Outcome::Sat(model, _) = ctl.solve_first().unwrap() else {
        panic!("the program has a model");
    };
    let read: Vec<(i32, Colour)> = model
        .atoms::<Coloured>()
        .unwrap()
        .into_iter()
        .map(|Coloured(node, colour)| (node, colour))
        .collect();
    assert_eq!(read, [(1, Colour::Red), (2, Colour::Green)]);
}

#[test]
fn a_hand_written_impl_checks_the_shape_itself() {
    let err = Coloured::from_symbol("colour(1)".parse().unwrap()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conversion);
    assert_eq!(err.to_string(), "`colour(1)` is not colour/2");
}

#[test]
fn hand_written_facts_are_added_like_derived_ones() {
    let mut ctl = Control::new().unwrap();
    let paint = |c: Colour| Symbol::function("paint", &[c.to_symbol()?]);
    ctl.add_facts([paint(Colour::Red).unwrap(), paint(Colour::Green).unwrap()])
        .unwrap();
    let (_, models) = ctl.solve_all().unwrap();
    let texts: Vec<String> = models[0]
        .symbols()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(texts, ["paint(green)", "paint(red)"]);
}
