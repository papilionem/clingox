# Configure the solver

clingo has one setting for every choice the solver makes: how many models to
enumerate, how to optimise, which heuristic to use, how many threads to run.
This page shows how to read and change them, and how to find out which
settings exist. The settings form a tree, and `Control::configuration` gives a
view of it.

## Set an option before the first solve

The simplest way is on the command line clingo itself accepts, at creation:

```rust
use clingox::Control;

let mut ctl = Control::with_args(["--opt-mode=optN", "--models=0"])?;
let conf = ctl.configuration();
assert_eq!(conf.get("solve.opt_mode")?.as_deref(), Some("optN"));
assert_eq!(conf.get("solve.models")?.as_deref(), Some("0"));
# Ok::<(), clingox::Error>(())
```

`Control::builder()` has shortcuts for the two options that most programs set,
`.threads(n)` and `.seed(n)`. Everything else goes through the configuration.

## Read and write by path

A path is clingo's own key syntax: names separated by dots. A number selects an
element of an array, so `solver.0.seed` is the seed of the first solver thread.
`get` returns the value as text, and `set` takes text:

```rust
use clingox::{Control, Part};

let mut ctl = Control::new()?;
ctl.add_base("{a;b}.")?;
ctl.ground(&[Part::base()])?;

let mut conf = ctl.configuration();
assert_eq!(conf.get("solve.models")?.as_deref(), Some("-1"));
conf.set("solve.models", "0")?;

// The change takes effect from the next solve call.
let (_, models) = ctl.solve_all()?;
assert_eq!(models.len(), 4);
# Ok::<(), clingox::Error>(())
```

The view borrows the control mutably, so the control cannot be used while the
view is alive. Let it go out of scope (or end its last use) before you ground
or solve, as above. The value of an option is always text, in the syntax of the
command line: `"0"`, `"optN"`, `"berkmin"`.

`get` returns `None` for an entry that has no value: a map or array such as
`solve`, or an option clingo leaves unassigned, such as
`tester.solver.heuristic`.

## Handle a bad path or value

An unknown path, a value clingo rejects, and an attempt to `set` a map or array
are `ErrorKind::Runtime` errors whose message names the path. They do not
poison the control:

```rust
use clingox::{Control, ErrorKind};

let mut ctl = Control::new()?;
let mut conf = ctl.configuration();
let err = conf.set("solve.models", "abc").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Runtime);
let err = conf.get("solve.nosuch").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Runtime);

// The option kept its value, and the control still works.
assert_eq!(conf.get("solve.models")?.as_deref(), Some("-1"));
# Ok::<(), clingox::Error>(())
```

A rejected value leaves the option as it was, with one exception clingo does
not allow clingox to repair: the options under `tester` start unassigned, and
the first `set` of any of them, accepted or not, makes clingo create that
whole configuration with every option at its default.

## Find out what exists

`keys` lists the names under a map, in clingo's order, and the empty path is
the root. `has_key` asks about one name, which may itself be dotted, without
listing:

```rust
use clingox::Control;

let mut ctl = Control::new()?;
let conf = ctl.configuration();

let top = conf.keys("")?;
assert!(top.iter().any(|key| key == "solve"));
assert!(conf.keys("solve")?.iter().any(|key| key == "models"));

assert!(conf.has_key("solve", "models")?);
assert!(!conf.has_key("solve", "nosuch")?);
assert!(conf.has_key("", "solver.seed")?);
# Ok::<(), clingox::Error>(())
```

`description` gives the help text clingo prints with `--help`, exactly as it
has it. A `%A` in it stands for the option's argument:

```rust
use clingox::Control;

let mut ctl = Control::new()?;
let conf = ctl.configuration();
assert_eq!(conf.description("solve")?, "Solve Options");
assert_eq!(
    conf.description("solver.seed")?,
    "Set random number generator's seed to %A",
);
# Ok::<(), clingox::Error>(())
```

## Arrays: one entry per solver thread

The `solver` entry is an array with one map of options per solver thread. By
default there is one. `len` gives the size and `element` the path of an
element:

```rust
use clingox::{Control, ErrorKind};

let mut ctl = Control::new()?;
let mut conf = ctl.configuration();

assert_eq!(conf.len("solver")?, 1);
assert_eq!(conf.element("solver", 0)?, "solver.0");

// `element` stays below `len`; `set` is how an array grows.
let err = conf.element("solver", 1).unwrap_err();
assert_eq!(err.kind(), ErrorKind::InvalidInput);
conf.set("solver.1.seed", "9")?;
assert_eq!(conf.len("solver")?, 2);
assert_eq!(conf.get("solver.1.seed")?.as_deref(), Some("9"));
# Ok::<(), clingox::Error>(())
```

Asking `len` of an entry that is not an array, or `has_key` of one that is not
a map, is `ErrorKind::InvalidInput`. The one entry that is both an array and a
map is `solver`: it answers to `solver.seed`, which is the first element's
option, as well as to `solver.0.seed`.

Setting options for more than one solver thread only has an effect when the
program runs more than one thread. Thread options such as `--parallel-mode`
exist only in builds with threads, so a default WebAssembly build rejects them
(see [Platform support](../reference/platforms.md)).

## Walk the whole tree

`Configuration::root` gives the root entry, and `children` lists what is below
an entry. An entry holds clingo's own key for one place in the tree, so a step
of a walk costs one or two calls into clingo and no path is built or resolved.
`kind` says what an entry is: `ConfigKind::Value` is read with `value`, `Map`
and `Array` are walked with `children`, and `ArrayMap` (only `solver`) is
walked as an array, so each option is visited once. `children` hands over each
child with a `PathSegment`, the name or index it has under its parent, which
`Display` writes the way a path does. This prints every option with its value:

```rust
use clingox::{ConfigEntry, ConfigKind, Control};

fn walk(entry: ConfigEntry<'_>, path: &mut Vec<String>, lines: &mut Vec<String>) -> clingox::Result<()> {
    match entry.kind()? {
        ConfigKind::Value => {
            let value = entry.value()?.unwrap_or_default();
            lines.push(format!("{} = {value}", path.join(".")));
        }
        ConfigKind::Array | ConfigKind::ArrayMap | ConfigKind::Map => {
            for child in entry.children()? {
                let (segment, child) = child?;
                path.push(segment.to_string());
                walk(child, path, lines)?;
                path.pop();
            }
        }
        // `ConfigKind` is non-exhaustive: a later clingo may add a kind.
        _ => {}
    }
    Ok(())
}

let mut ctl = Control::new()?;
let conf = ctl.configuration();
let mut lines = Vec::new();
walk(conf.root()?, &mut Vec::new(), &mut lines)?;
assert!(lines.contains(&"solve.models = -1".to_owned()));
# Ok::<(), clingox::Error>(())
```

Bind the view before taking the root: `ctl.configuration().root()?` does not
compile when the entry is kept, because the view is a temporary. An entry
borrows the view, so `set` cannot run while one is alive. To change what a walk
found, collect the paths (the `path.join(".")` above) and set them afterwards.

An entry is also the way to read one option many times: `conf.entry("solve.models")?`
resolves the path once, and `value` then reads it without resolving it again.
`entry` on an entry takes a path relative to it, so
`conf.entry("solve")?.entry("models")?` reads the same option. A single read is
simpler by path (`conf.get("solve.models")?`); entries pay off for walks and for
reads that repeat.

`ConfigEntry::len` is the size of an array and an error for anything else, as
`Configuration::len` is. For `solver`, `keys` lists the options of its first
element, while `children` lists the elements.

The same walk by path, with `kind`, `get`, `keys`, `len` and `element`, still
works and gives the same list. It resolves every path from the root, which is
what makes it two to three times slower:

```rust
use clingox::{ConfigKind, Configuration, Control};

fn walk(conf: &Configuration<'_>, path: &str, lines: &mut Vec<String>) -> clingox::Result<()> {
    match conf.kind(path)? {
        ConfigKind::Value => {
            let value = conf.get(path)?.unwrap_or_default();
            lines.push(format!("{path} = {value}"));
        }
        ConfigKind::Array | ConfigKind::ArrayMap => {
            for index in 0..conf.len(path)? {
                walk(conf, &conf.element(path, index)?, lines)?;
            }
        }
        ConfigKind::Map => {
            for key in conf.keys(path)? {
                let child = if path.is_empty() { key } else { format!("{path}.{key}") };
                walk(conf, &child, lines)?;
            }
        }
        // `ConfigKind` is non-exhaustive: a later clingo may add a kind.
        _ => {}
    }
    Ok(())
}

let mut ctl = Control::new()?;
let conf = ctl.configuration();
let mut lines = Vec::new();
walk(&conf, "", &mut lines)?;
assert!(lines.contains(&"solve.models = -1".to_owned()));
# Ok::<(), clingox::Error>(())
```

## Related pages

- [Errors and panics](../concepts/errors-and-panics.md) explains which errors
  poison a control; none of the configuration errors do.
- [Solving step by step](../tutorial/solving-step-by-step.md) uses
  `solve.models` to control enumeration.
- [Coming from pyclingo](../reference/coming-from-pyclingo.md) maps
  `ctl.configuration.solve.models = "0"` to `set("solve.models", "0")`.
