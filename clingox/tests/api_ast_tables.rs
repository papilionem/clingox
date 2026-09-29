//! The complete `AstType` (46) and `Attribute` (45) tables against the
//! pyclingo 5.8.2 oracle: a swapped pair such as
//! `Pool` and `Interval`, or `Left` and `Right`, must not survive.
//!
//! The corpus below covers every node type clingo 5.8.2 can parse. It is
//! walked completely, through `ast_type`, `has_attribute`,
//! `attribute_type` and the typed accessors. For every type the expected
//! table fixes the smallest `Display` text of any node of that type; for
//! every (type, attribute) pair that occurs it fixes the smallest rendering
//! of the attribute's value. Both are `min` over the whole corpus, so the
//! result does not depend on visiting order. The expected text was produced
//! by the same walk in pyclingo (`clingo.ast`, attribute names mapped from
//! `snake_case` to the variant names).
//!
//! Two attributes, `Coefficient` and `Variable`, are declared by the header
//! but used by no node constructor (`control.cc:1432-1487`); they must be
//! absent everywhere, which the table check covers by never listing them.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::collections::BTreeMap;

use clingox::ast::{self, Ast, Attribute, AttributeType};

const CORPUS: &str = r#"#const n = 3.
#program p(x,y).
a(X,Y) :- b(X), not c(Y), X < Y, X = 1..n, Z = (1;2;3), -X != |Y|, X ** 2 = Y \ 2 ? 3 & 4 ^ 5.
#show p/1.
#show -q/2.
#show t : p(X).
#defined d/2.
#external e(1) : b(1).
#external f. [true]
#project g/1.
#project h(X) : k(X).
#edge (1,2) : e.
#heuristic hh(X) : k(X). [1@2, sign]
:~ p(X), q(Y). [X+Y@1,X,Y]
{ a; b : c } = 1 :- d.
1 { a(X) : b(X) } 2 :- c.
a ; b :- c.
a(X) : b(X) ; c :- d.
:- #sum { X,Y : p(X,Y) ; 3 : q } > 2, #count { 1 : a } < 3, #sum+ { 1: a } >= 1, #min{ 1:a } = 2, #max{ 1:a } != 2.
:- 1 < #count{ X : p(X) } < 3.
:- &diff { x - y } <= 3.
&sum { 1 : a } >= 3 :- b.
#theory t {
  term { + : 1, binary, left; - : 2, unary };
  &dom/0 : term, {<=,=}, term, any;
  &sum/1 : term, {=}, term, body
}.
#script (python)
def f(): pass
#end.
p(@f(1), "s", #inf, #sup, f(1,2), (1,2), ()).
#true.
:- #false.
a :- not not b, X != Y.
a :- b : c, d.
%* block *%
% line
#count { 1 : a : x; 2 : b : y, z } >= 1 :- d.
#sum { 1,2 : a : x } = 3 :- d.
&sum { [1,2] , {3,4}, (5,6) , f(1) , a + b : x } >= 2 :- e.
"#;

/// Every `Attribute`, in the enum's own declaration order.
const ATTRIBUTES: [Attribute; 45] = [
    Attribute::Argument,
    Attribute::Arguments,
    Attribute::Arity,
    Attribute::Atom,
    Attribute::Atoms,
    Attribute::AtomType,
    Attribute::Bias,
    Attribute::Body,
    Attribute::Code,
    Attribute::Coefficient,
    Attribute::Comparison,
    Attribute::Condition,
    Attribute::Elements,
    Attribute::External,
    Attribute::ExternalType,
    Attribute::Function,
    Attribute::Guard,
    Attribute::Guards,
    Attribute::Head,
    Attribute::IsDefault,
    Attribute::Left,
    Attribute::LeftGuard,
    Attribute::Literal,
    Attribute::Location,
    Attribute::Modifier,
    Attribute::Name,
    Attribute::NodeU,
    Attribute::NodeV,
    Attribute::OperatorName,
    Attribute::OperatorType,
    Attribute::Operators,
    Attribute::Parameters,
    Attribute::Positive,
    Attribute::Priority,
    Attribute::Right,
    Attribute::RightGuard,
    Attribute::SequenceType,
    Attribute::Sign,
    Attribute::Symbol,
    Attribute::Term,
    Attribute::Terms,
    Attribute::Value,
    Attribute::Variable,
    Attribute::Weight,
    Attribute::CommentType,
];

/// One line per (type, attribute) pair and per type, the format of
/// `EXPECTED`: `Type<TAB>Attribute<TAB>value`, with an empty attribute for
/// the type's own smallest `Display` text.
const EXPECTED: &str = r#"Aggregate		1 <= { a(X): b(X) } <= 2
Aggregate	Elements	[a | b: c]
Aggregate	LeftGuard	 <= 1
Aggregate	Location	<location>
Aggregate	RightGuard	 <= 2
BinaryOperation		(((Y\\2)?(3&4))^5)
BinaryOperation	Left	((Y\\2)?(3&4))
BinaryOperation	Location	<location>
BinaryOperation	OperatorType	0
BinaryOperation	Right	(3&4)
BodyAggregate		1 < #count { X: p(X) } < 3
BodyAggregate	Elements	[1: a]
BodyAggregate	Function	0
BodyAggregate	LeftGuard	 != 2
BodyAggregate	Location	<location>
BodyAggregate	RightGuard	 < 3
BodyAggregateElement		1: a
BodyAggregateElement	Condition	[a]
BodyAggregateElement	Terms	[1]
BooleanConstant		#false
BooleanConstant	Value	0
Comment		% line
Comment	CommentType	0
Comment	Location	<location>
Comment	Value	% line
Comparison		(X**2) = (((Y\\2)?(3&4))^5)
Comparison	Guards	[ != Y]
Comparison	Term	(X**2)
ConditionalLiteral		a
ConditionalLiteral	Condition	[]
ConditionalLiteral	Literal	a
ConditionalLiteral	Location	<location>
Defined		#defined d/2.
Defined	Arity	2
Defined	Location	<location>
Defined	Name	d
Defined	Positive	1
Definition		#const n = 3.
Definition	IsDefault	1
Definition	Location	<location>
Definition	Name	n
Definition	Value	3
Disjunction		a(X): b(X); c
Disjunction	Elements	[a | b]
Disjunction	Location	<location>
Edge		#edge (1,2) : e.
Edge	Body	[e]
Edge	Location	<location>
Edge	NodeU	1
Edge	NodeV	2
External		#external e(1) : b(1). [false]
External	Atom	e(1)
External	Body	[]
External	ExternalType	false
External	Location	<location>
Function		()
Function	Arguments	[1 | 2]
Function	External	0
Function	Location	<location>
Function	Name	
Guard		 != 2
Guard	Comparison	0
Guard	Term	(((Y\\2)?(3&4))^5)
HeadAggregate		1 <= #count { 1: a: x; 2: b: y, z }
HeadAggregate	Elements	[1,2: a: x]
HeadAggregate	Function	0
HeadAggregate	LeftGuard	 <= 1
HeadAggregate	Location	<location>
HeadAggregate	RightGuard	none
HeadAggregateElement		1,2: a: x
HeadAggregateElement	Condition	a: x
HeadAggregateElement	Terms	[1 | 2]
Heuristic		#heuristic hh(X) : k(X). [1@2,sign]
Heuristic	Atom	hh(X)
Heuristic	Bias	1
Heuristic	Body	[k(X)]
Heuristic	Location	<location>
Heuristic	Modifier	sign
Heuristic	Priority	2
Id		x
Id	Location	<location>
Id	Name	x
Interval		(1..n)
Interval	Left	1
Interval	Location	<location>
Interval	Right	n
Literal		#false
Literal	Atom	#false
Literal	Location	<location>
Literal	Sign	0
Minimize		:~ p(X); q(Y). [(X+Y)@1,X,Y]
Minimize	Body	[p(X) | q(Y)]
Minimize	Location	<location>
Minimize	Priority	1
Minimize	Terms	[X | Y]
Minimize	Weight	(X+Y)
Pool		(1;2;3)
Pool	Arguments	[1 | 2 | 3]
Pool	Location	<location>
Program		#program base.
Program	Location	<location>
Program	Name	base
Program	Parameters	[]
ProjectAtom		#project h(X) : k(X).
ProjectAtom	Atom	h(X)
ProjectAtom	Body	[k(X)]
ProjectAtom	Location	<location>
ProjectSignature		#project g/1.
ProjectSignature	Arity	1
ProjectSignature	Location	<location>
ProjectSignature	Name	g
ProjectSignature	Positive	1
Rule		#false :- #false.
Rule	Body	[#false]
Rule	Head	#false
Rule	Location	<location>
Script		#script (python)\ndef f(): pass\n#end.
Script	Code	\ndef f(): pass\n
Script	Location	<location>
Script	Name	python
ShowSignature		#show -q/2.
ShowSignature	Arity	1
ShowSignature	Location	<location>
ShowSignature	Name	p
ShowSignature	Positive	0
ShowTerm		#show t : p(X).
ShowTerm	Body	[p(X)]
ShowTerm	Location	<location>
ShowTerm	Term	t
SymbolicAtom		a
SymbolicAtom	Symbol	a
SymbolicTerm		"s"
SymbolicTerm	Location	<location>
SymbolicTerm	Symbol	"s"
TheoryAtom		&diff { (x - y) } <= 3
TheoryAtom	Elements	[(x - y)]
TheoryAtom	Guard	<= 3
TheoryAtom	Location	<location>
TheoryAtom	Term	diff
TheoryAtomDefinition		&dom/0: term, { <=, = }, term, any
TheoryAtomDefinition	Arity	0
TheoryAtomDefinition	AtomType	1
TheoryAtomDefinition	Guard	{ <=, = }, term
TheoryAtomDefinition	Location	<location>
TheoryAtomDefinition	Name	dom
TheoryAtomDefinition	Term	term
TheoryAtomElement		(x - y)
TheoryAtomElement	Condition	[]
TheoryAtomElement	Terms	[(x - y)]
TheoryDefinition		#theory t {\n  term {\n    + : 1, binary, left;\n    - : 2, unary\n  };\n  &dom/0: term, { <=, = }, term, any;\n  &sum/1: term, { = }, term, body\n}.
TheoryDefinition	Atoms	[&dom/0: term, { <=, = }, term, any | &sum/1: term, { = }, term, body]
TheoryDefinition	Location	<location>
TheoryDefinition	Name	t
TheoryDefinition	Terms	[term {\n  + : 1, binary, left;\n- : 2, unary\n}]
TheoryFunction		f(1)
TheoryFunction	Arguments	[1]
TheoryFunction	Location	<location>
TheoryFunction	Name	f
TheoryGuard		<= 3
TheoryGuard	OperatorName	<=
TheoryGuard	Term	2
TheoryGuardDefinition		{ <=, = }, term
TheoryGuardDefinition	Operators	[<= | =]
TheoryGuardDefinition	Term	term
TheoryOperatorDefinition		+ : 1, binary, left
TheoryOperatorDefinition	Location	<location>
TheoryOperatorDefinition	Name	+
TheoryOperatorDefinition	OperatorType	0
TheoryOperatorDefinition	Priority	1
TheorySequence		(5,6)
TheorySequence	Location	<location>
TheorySequence	SequenceType	0
TheorySequence	Terms	[1 | 2]
TheoryTermDefinition		term {\n  + : 1, binary, left;\n- : 2, unary\n}
TheoryTermDefinition	Location	<location>
TheoryTermDefinition	Name	term
TheoryTermDefinition	Operators	[+ : 1, binary, left | - : 2, unary]
TheoryUnparsedTerm		(a + b)
TheoryUnparsedTerm	Elements	[a | + b]
TheoryUnparsedTerm	Location	<location>
TheoryUnparsedTermElement		+ b
TheoryUnparsedTermElement	Operators	[+]
TheoryUnparsedTermElement	Term	a
UnaryOperation		-X
UnaryOperation	Argument	X
UnaryOperation	Location	<location>
UnaryOperation	OperatorType	0
Variable		X
Variable	Location	<location>
Variable	Name	X
"#;

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn upd(map: &mut BTreeMap<String, String>, key: String, value: String) {
    map.entry(key)
        .and_modify(|old| {
            if value < *old {
                old.clone_from(&value);
            }
        })
        .or_insert(value);
}

fn walk(node: &Ast, map: &mut BTreeMap<String, String>) {
    let ty = format!("{:?}", node.ast_type());
    upd(map, format!("{ty}\t"), escape(&node.to_string()));
    for attribute in ATTRIBUTES {
        if !node.has_attribute(attribute) {
            continue;
        }
        let kind = node.attribute_type(attribute).unwrap();
        let value = match kind {
            AttributeType::Number => node.number(attribute).unwrap().to_string(),
            AttributeType::Symbol => escape(&node.symbol(attribute).unwrap().to_string()),
            AttributeType::Location => {
                node.span(attribute).unwrap();
                "<location>".to_owned()
            }
            AttributeType::String => escape(&node.string(attribute).unwrap()),
            AttributeType::Ast => {
                let child = node.ast(attribute).unwrap();
                walk(&child, map);
                escape(&child.to_string())
            }
            AttributeType::OptionalAst => match node.optional_ast(attribute).unwrap() {
                Some(child) => {
                    walk(&child, map);
                    escape(&child.to_string())
                }
                None => "none".to_owned(),
            },
            AttributeType::StringArray => {
                let items: Vec<String> = (0..node.string_array_len(attribute).unwrap())
                    .map(|i| escape(&node.string_at(attribute, i).unwrap()))
                    .collect();
                format!("[{}]", items.join(" | "))
            }
            AttributeType::AstArray => {
                let items: Vec<String> = (0..node.ast_array_len(attribute).unwrap())
                    .map(|i| {
                        let child = node.ast_at(attribute, i).unwrap();
                        walk(&child, map);
                        escape(&child.to_string())
                    })
                    .collect();
                format!("[{}]", items.join(" | "))
            }
            other => panic!("an attribute type this test does not know: {other:?}"),
        };
        upd(map, format!("{ty}\t{attribute:?}"), value);
    }
}

#[test]
fn the_attribute_list_names_45_distinct_attributes() {
    let mut names: Vec<String> = ATTRIBUTES.iter().map(|a| format!("{a:?}")).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 45);
}

#[test]
fn every_type_and_attribute_of_the_corpus_reads_back_as_the_oracle_says() {
    let mut map = BTreeMap::new();
    ast::parse_string(CORPUS, |node| {
        walk(&node, &mut map);
        Ok(())
    })
    .unwrap();

    let actual: Vec<String> = map.iter().map(|(k, v)| format!("{k}\t{v}")).collect();
    let mut expected: Vec<String> = EXPECTED
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    expected.sort();
    assert_eq!(actual, expected);

    let types: std::collections::BTreeSet<&str> =
        map.keys().filter_map(|k| k.strip_suffix('\t')).collect();
    assert_eq!(
        types.len(),
        46,
        "the corpus covers every AstType: {types:?}"
    );
    let used: std::collections::BTreeSet<&str> = map
        .keys()
        .filter_map(|k| k.split('\t').nth(1))
        .filter(|a| !a.is_empty())
        .collect();
    assert_eq!(used.len(), 43, "43 attributes occur in some node: {used:?}");
    assert!(!used.contains("Coefficient") && !used.contains("Variable"));
}
