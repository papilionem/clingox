// The owned kind keeps its name: `Control` is `ScopedControl<'static>`, so
// existing signatures, struct fields and return types compile unchanged, and a
// function that takes `&mut ScopedControl<'_>` accepts it.
use clingox::{Control, ScopedControl};

struct Holder {
    control: Control,
}

fn make(program: &str) -> Control {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl
}

fn make_scoped(program: &str) -> ScopedControl<'static> {
    make(program)
}

fn generic(ctl: &mut ScopedControl<'_>) -> bool {
    ctl.is_conflicting()
}

fn main() {
    let mut holder = Holder {
        control: make_scoped("a."),
    };
    assert!(!generic(&mut holder.control));
}
