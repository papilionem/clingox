// A plain function is a valid `main` callback; it takes `ScopedControl<'_>`.
use clingox::application::Application;
use clingox::{Result, ScopedControl};

fn callback(ctl: &mut ScopedControl<'_>, _files: &[&str]) -> Result<()> {
    ctl.add_base("a.")
}

fn main() {
    let _ = Application::new().main(callback).run(["--outf=3"]);
}
