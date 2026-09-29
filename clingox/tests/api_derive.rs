//! Converting Rust values to and from symbols: the `ToSymbol`, `FromSymbol`
//! and `Predicate` traits, their derives on structs and enums, and the
//! conversions of standard types.
//!
//! This file forbids `unsafe` code, so every derive below also proves that the
//! generated code compiles where `unsafe` is forbidden. The symbol texts were
//! checked against the Python module `clingo` 5.8.2.

#![forbid(unsafe_code)]

use clingox::{ErrorKind, FromSymbol, Predicate, Sign, Symbol, ToSymbol};

fn term(text: &str) -> Symbol {
    text.parse().expect("the test term parses")
}

fn text_of<T: ToSymbol + ?Sized>(value: &T) -> String {
    value
        .to_symbol()
        .expect("the value converts to a symbol")
        .to_string()
}

fn conversion_error<T: std::fmt::Debug>(result: clingox::Result<T>) -> clingox::Error {
    let err = result.expect_err("the conversion must fail");
    assert_eq!(err.kind(), ErrorKind::Conversion, "{err}");
    err
}

// ---------------------------------------------------------------------------
// Types used by the tests.

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct TaskAssignment {
    worker: i32,
    task: i32,
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
struct HTTPCheck(i32);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
#[clingo(name = "edge")]
struct Link(i32, i32);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Done;

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Empty {}

/// `asserted(comp13,encrypted,0)`.
#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Asserted {
    #[clingo(constant)]
    object: String,
    #[clingo(constant)]
    property: String,
    value: i32,
}

/// `violation("floor3_enc",comp13)`.
#[derive(FromSymbol, Debug, PartialEq)]
struct Violation {
    #[clingo(string)]
    rule: String,
    #[clingo(constant)]
    culprit: String,
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Labelled(#[clingo(string)] String);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Named(#[clingo(constant)] String);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Holder {
    anything: Symbol,
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq, Clone, Copy)]
struct Node(i32);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Hop {
    from: Node,
    to: Node,
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Pair<T>(T, T);

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Wide {
    count: u32,
    total: i64,
    small: u8,
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
struct Route {
    stops: Vec<i32>,
    span: (i32, i32),
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
enum Shape {
    Point,
    Circle(i32),
    Rect {
        w: i32,
        h: i32,
    },
    #[clingo(name = "sq")]
    Square(i32),
}

#[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
enum Color {
    DarkRed,
    Named(#[clingo(constant)] String),
}

// ---------------------------------------------------------------------------
// Structs.

#[test]
fn a_named_struct_becomes_a_function_named_in_snake_case() {
    let value = TaskAssignment { worker: 1, task: 2 };
    assert_eq!(text_of(&value), "task_assignment(1,2)");
    assert_eq!(
        TaskAssignment::from_symbol(term("task_assignment(1,2)")).unwrap(),
        value
    );
}

#[test]
fn acronyms_split_as_heck_does() {
    assert_eq!(text_of(&HTTPCheck(7)), "http_check(7)");
    assert_eq!(<HTTPCheck as Predicate>::NAME, "http_check");
}

#[test]
fn the_name_attribute_overrides_the_predicate_name() {
    assert_eq!(text_of(&Link(1, 2)), "edge(1,2)");
    assert_eq!(Link::from_symbol(term("edge(1,2)")).unwrap(), Link(1, 2));
    assert_eq!(<Link as Predicate>::NAME, "edge");
    conversion_error(Link::from_symbol(term("link(1,2)")));
}

#[test]
fn a_tuple_struct_takes_its_fields_in_order() {
    let symbol = Link(3, 4).to_symbol().unwrap();
    assert_eq!(
        symbol.arguments().unwrap(),
        &[Symbol::number(3), Symbol::number(4)]
    );
}

#[test]
fn a_struct_without_fields_is_a_constant() {
    assert_eq!(text_of(&Done), "done");
    assert_eq!(Done::from_symbol(term("done")).unwrap(), Done);
    assert_eq!(text_of(&Empty {}), "empty");
    assert_eq!(Empty::from_symbol(term("empty")).unwrap(), Empty {});
    conversion_error(Done::from_symbol(term("done(1)")));
}

#[test]
fn string_and_constant_fields_differ() {
    let fact = Asserted {
        object: "comp13".into(),
        property: "encrypted".into(),
        value: 0,
    };
    assert_eq!(text_of(&fact), "asserted(comp13,encrypted,0)");
    assert_eq!(
        Asserted::from_symbol(term("asserted(comp13,encrypted,0)")).unwrap(),
        fact
    );

    let violation = Violation::from_symbol(term(r#"violation("floor3_enc",comp13)"#)).unwrap();
    assert_eq!(
        violation,
        Violation {
            rule: "floor3_enc".into(),
            culprit: "comp13".into(),
        }
    );

    assert_eq!(text_of(&Labelled("comp13".into())), r#"labelled("comp13")"#);
    assert_eq!(text_of(&Named("comp13".into())), "named(comp13)");
}

#[test]
fn a_string_field_rejects_a_constant_and_a_constant_field_rejects_a_string() {
    conversion_error(Labelled::from_symbol(term("labelled(comp13)")));
    conversion_error(Named::from_symbol(term(r#"named("comp13")"#)));
    // A constant field takes a positive constant only.
    conversion_error(Named::from_symbol(term("named(-comp13)")));
    conversion_error(Named::from_symbol(term("named(f(1))")));
    conversion_error(Named::from_symbol(term("named(())")));
    conversion_error(Named::from_symbol(term("named(1)")));
    conversion_error(Labelled::from_symbol(term("labelled(1)")));
}

#[test]
fn string_fields_keep_their_content() {
    let content = "a\"b\\c\nd";
    let symbol = Labelled(content.into()).to_symbol().unwrap();
    assert_eq!(symbol.arguments().unwrap()[0].as_string(), Some(content));
    // clingo 5.8.2 prints the escapes back.
    assert_eq!(symbol.to_string(), r#"labelled("a\"b\\c\nd")"#);
    assert_eq!(Labelled::from_symbol(symbol).unwrap().0, content);
}

#[test]
fn symbol_fields_pass_through() {
    for text in ["1", r#""x""#, "c", "-q", "(1,2)", "#sup", "#inf", "f(g(1))"] {
        let holder = Holder {
            anything: term(text),
        };
        assert_eq!(text_of(&holder), format!("holder({text})"));
        assert_eq!(
            Holder::from_symbol(term(&format!("holder({text})"))).unwrap(),
            holder
        );
    }
}

#[test]
fn nested_types_become_nested_terms() {
    let hop = Hop {
        from: Node(1),
        to: Node(2),
    };
    assert_eq!(text_of(&hop), "hop(node(1),node(2))");
    assert_eq!(Hop::from_symbol(term("hop(node(1),node(2))")).unwrap(), hop);
    conversion_error(Hop::from_symbol(term("hop(1,2)")));
    conversion_error(Hop::from_symbol(term("hop(node(1),vertex(2))")));
}

#[test]
fn from_symbol_checks_name_arity_and_sign() {
    assert!(TaskAssignment::from_symbol(term("task_assignment(1,2)")).is_ok());
    for text in [
        "assignment(1,2)",
        "task_assignment(1)",
        "task_assignment(1,2,3)",
        "-task_assignment(1,2)",
        "(1,2)",
        "task_assignment",
        "7",
        r#""task_assignment""#,
        "#sup",
    ] {
        conversion_error(TaskAssignment::from_symbol(term(text)));
    }
}

#[test]
fn a_failed_field_conversion_names_the_field_and_the_symbol() {
    let err = conversion_error(Wide::from_symbol(term("wide(1,2,oops)")));
    let message = err.to_string();
    assert!(message.contains("small"), "{message}");
    assert!(message.contains("Wide"), "{message}");
    assert!(message.contains("oops"), "{message}");
}

#[test]
fn deriving_from_symbol_implements_predicate() {
    assert_eq!(<Violation as Predicate>::NAME, "violation");
    assert_eq!(<Violation as Predicate>::ARITY, 2);
    assert_eq!(<Asserted as Predicate>::NAME, "asserted");
    assert_eq!(<Asserted as Predicate>::ARITY, 3);
    assert_eq!(<Done as Predicate>::ARITY, 0);
    assert_eq!(<Link as Predicate>::ARITY, 2);
}

#[test]
fn generic_structs_are_supported() {
    assert_eq!(text_of(&Pair(1, 2)), "pair(1,2)");
    assert_eq!(text_of(&Pair(Node(1), Node(2))), "pair(node(1),node(2))");
    assert_eq!(
        Pair::<i32>::from_symbol(term("pair(1,2)")).unwrap(),
        Pair(1, 2)
    );
    assert_eq!(<Pair<i32> as Predicate>::NAME, "pair");
    assert_eq!(<Pair<Node> as Predicate>::ARITY, 2);
}

#[test]
fn collection_fields_are_tuples() {
    let route = Route {
        stops: vec![1, 2, 3],
        span: (1, 3),
    };
    assert_eq!(text_of(&route), "route((1,2,3),(1,3))");
    assert_eq!(
        Route::from_symbol(term("route((1,2,3),(1,3))")).unwrap(),
        route
    );
}

#[test]
fn to_symbol_errors_keep_their_kind() {
    let err = Labelled("a\0b".into()).to_symbol().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    let err = Named("a\0b".into()).to_symbol().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Nul);
    // An empty constant would be the empty tuple.
    conversion_error(Named(String::new()).to_symbol());
}

// ---------------------------------------------------------------------------
// Enums.

#[test]
fn enum_variants_are_constants_or_functions() {
    let cases = [
        (Shape::Point, "point"),
        (Shape::Circle(3), "circle(3)"),
        (Shape::Rect { w: 2, h: 3 }, "rect(2,3)"),
        (Shape::Square(4), "sq(4)"),
    ];
    for (shape, text) in cases {
        assert_eq!(text_of(&shape), text);
        assert_eq!(Shape::from_symbol(term(text)).unwrap(), shape);
    }
}

#[test]
fn enum_variant_names_are_snake_case() {
    assert_eq!(text_of(&Color::DarkRed), "dark_red");
    assert_eq!(
        Color::from_symbol(term("dark_red")).unwrap(),
        Color::DarkRed
    );
    assert_eq!(text_of(&Color::Named("teal".into())), "named(teal)");
    assert_eq!(
        Color::from_symbol(term("named(teal)")).unwrap(),
        Color::Named("teal".into())
    );
}

#[test]
fn an_enum_rejects_a_symbol_matching_no_variant() {
    for text in [
        "circle(1,2)",
        "circle",
        "point(1)",
        "square(4)",
        "triangle",
        "-point",
        "1",
    ] {
        conversion_error(Shape::from_symbol(term(text)));
    }
}

// ---------------------------------------------------------------------------
// Standard types.

#[test]
fn integers_convert_with_range_checks() {
    assert_eq!(text_of(&7_i32), "7");
    assert_eq!(text_of(&i32::MIN), "-2147483648");
    assert_eq!(i32::from_symbol(Symbol::number(-5)).unwrap(), -5);

    assert_eq!(text_of(&7_u8), "7");
    assert_eq!(text_of(&-7_i8), "-7");
    assert_eq!(text_of(&7_u16), "7");
    assert_eq!(text_of(&-7_i16), "-7");
    assert_eq!(text_of(&7_usize), "7");
    assert_eq!(text_of(&-7_isize), "-7");
    assert_eq!(text_of(&2_147_483_647_u32), "2147483647");
    assert_eq!(text_of(&-2_147_483_648_i64), "-2147483648");
    assert_eq!(text_of(&7_u64), "7");
    assert_eq!(text_of(&7_i128), "7");
    assert_eq!(text_of(&7_u128), "7");

    conversion_error(u32::MAX.to_symbol());
    conversion_error(2_147_483_648_u32.to_symbol());
    conversion_error(i64::MAX.to_symbol());
    conversion_error((-2_147_483_649_i64).to_symbol());
    conversion_error(u64::MAX.to_symbol());
    conversion_error(usize::MAX.to_symbol());
    conversion_error(i128::MIN.to_symbol());

    assert_eq!(u8::from_symbol(Symbol::number(255)).unwrap(), 255);
    conversion_error(u8::from_symbol(Symbol::number(256)));
    conversion_error(u8::from_symbol(Symbol::number(-1)));
    conversion_error(u32::from_symbol(Symbol::number(-1)));
    conversion_error(i8::from_symbol(Symbol::number(128)));
    assert_eq!(i8::from_symbol(Symbol::number(-128)).unwrap(), -128);
    assert_eq!(
        i64::from_symbol(Symbol::number(i32::MIN)).unwrap(),
        i64::from(i32::MIN)
    );
    assert_eq!(usize::from_symbol(Symbol::number(3)).unwrap(), 3);
    conversion_error(usize::from_symbol(Symbol::number(-3)));

    // Only numbers convert to integers.
    conversion_error(i32::from_symbol(term("a")));
    conversion_error(i32::from_symbol(term(r#""1""#)));
    conversion_error(i32::from_symbol(Symbol::supremum()));
}

#[test]
fn integer_fields_of_other_types_are_checked() {
    let wide = Wide {
        count: 3,
        total: -4,
        small: 5,
    };
    assert_eq!(text_of(&wide), "wide(3,-4,5)");
    assert_eq!(Wide::from_symbol(term("wide(3,-4,5)")).unwrap(), wide);

    conversion_error(
        Wide {
            count: u32::MAX,
            total: 0,
            small: 0,
        }
        .to_symbol(),
    );
    conversion_error(
        Wide {
            count: 0,
            total: i64::MIN,
            small: 0,
        }
        .to_symbol(),
    );
    conversion_error(Wide::from_symbol(term("wide(-1,0,0)")));
    conversion_error(Wide::from_symbol(term("wide(0,0,300)")));
}

#[test]
fn symbols_convert_to_themselves() {
    for text in [
        "1",
        r#""x""#,
        "c",
        "-q",
        "(1,2)",
        "#sup",
        "#inf",
        "p(1,f(x))",
    ] {
        let symbol = term(text);
        assert_eq!(symbol.to_symbol().unwrap(), symbol);
        assert_eq!(Symbol::from_symbol(symbol).unwrap(), symbol);
    }
}

#[test]
fn tuples_become_clingo_tuples() {
    assert_eq!(text_of(&()), "()");
    assert_eq!(text_of(&(1,)), "(1,)");
    assert_eq!(text_of(&(1, 2)), "(1,2)");
    assert_eq!(text_of(&(1, Node(2), (3, 4))), "(1,node(2),(3,4))");
    assert_eq!(
        text_of(&(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)),
        "(1,2,3,4,5,6,7,8,9,10,11,12)"
    );

    <()>::from_symbol(term("()")).unwrap();
    assert_eq!(<(i32,)>::from_symbol(term("(1,)")).unwrap(), (1,));
    assert_eq!(<(i32, i32)>::from_symbol(term("(1,2)")).unwrap(), (1, 2));
    assert_eq!(
        <(i32, Node)>::from_symbol(term("(1,node(2))")).unwrap(),
        (1, Node(2))
    );
    conversion_error(<(i32, i32)>::from_symbol(term("(1,2,3)")));
    conversion_error(<(i32, i32)>::from_symbol(term("p(1,2)")));
    conversion_error(<(i32, i32)>::from_symbol(term("-(1,2)")));
    conversion_error(<(i32, i32)>::from_symbol(term("(1,a)")));
    conversion_error(<()>::from_symbol(term("p")));
}

#[test]
fn vectors_become_clingo_tuples() {
    assert_eq!(text_of(&vec![1, 2, 3]), "(1,2,3)");
    assert_eq!(text_of(&vec![1]), "(1,)");
    assert_eq!(text_of(&Vec::<i32>::new()), "()");
    assert_eq!(text_of(&[Node(1), Node(2)][..]), "(node(1),node(2))");

    assert_eq!(
        Vec::<i32>::from_symbol(term("(1,2,3)")).unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        Vec::<i32>::from_symbol(term("()")).unwrap(),
        Vec::<i32>::new()
    );
    assert_eq!(Vec::<i32>::from_symbol(term("(1,)")).unwrap(), vec![1]);
    conversion_error(Vec::<i32>::from_symbol(term("p(1,2)")));
    conversion_error(Vec::<i32>::from_symbol(term("1")));
    conversion_error(Vec::<i32>::from_symbol(term("(1,a)")));
}

#[test]
fn references_convert_like_their_target() {
    let node = Node(1);
    let reference = &node;
    assert_eq!(reference.to_symbol().unwrap(), node.to_symbol().unwrap());
    assert_eq!(text_of(&&&node), "node(1)");
    assert_eq!(text_of(&vec![&node, &node]), "(node(1),node(1))");
}

#[test]
fn a_negative_function_symbol_is_never_a_derived_struct() {
    let negated = term("-node(1)");
    assert_eq!(negated.sign(), Some(Sign::Negative));
    conversion_error(Node::from_symbol(negated));
}

mod through_the_prelude {
    #![forbid(unsafe_code)]

    use clingox::prelude::*;

    #[derive(ToSymbol, FromSymbol, Debug, PartialEq)]
    struct Visit {
        #[clingo(constant)]
        place: String,
        day: u8,
    }

    #[test]
    fn the_traits_and_derives_are_in_the_prelude() {
        let visit = Visit {
            place: "rome".into(),
            day: 3,
        };
        let symbol = visit.to_symbol().unwrap();
        assert_eq!(symbol.to_string(), "visit(rome,3)");
        assert_eq!(Visit::from_symbol(symbol).unwrap(), visit);
        assert_eq!(<Visit as Predicate>::NAME, "visit");
    }
}
