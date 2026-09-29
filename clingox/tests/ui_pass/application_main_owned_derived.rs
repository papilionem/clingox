// An owned control made inside the callback is `Control`: it may leave
// the callback, and the borrowed views of *it* behave as they always did.
use clingox::Control;
use clingox::application::Application;

fn main() {
    let mut kept: Option<Control> = None;
    Application::new()
        .main(|_ctl, _files| {
            kept = Some(Control::new()?);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let mut ctl = kept.expect("the callback stored a control");
    ctl.add_base("a.").unwrap();
    let stats = ctl.statistics().unwrap();
    let _ = stats.keys("");
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let _ = handle.next_model();
}
