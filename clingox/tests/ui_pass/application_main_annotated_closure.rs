// A closure whose parameter is annotated must name the scoped type: the alias
// `Control` is `ScopedControl<'static>`, the owned kind, and would not accept the
// callback's control. The inferred form needs no annotation at all.
use clingox::ScopedControl;
use clingox::application::Application;

fn main() {
    let _ = Application::new()
        .main(|ctl: &mut ScopedControl, _files: &[&str]| ctl.add_base("a."))
        .run(["--outf=3"]);
    let _ = Application::new()
        .main(|ctl: &mut ScopedControl<'_>, _files: &[&str]| ctl.add_base("a."))
        .run(["--outf=3"]);
    let _ = Application::new()
        .main(|ctl, _files| ctl.add_base("a."))
        .run(["--outf=3"]);
}
