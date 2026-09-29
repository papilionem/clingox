// Inside the callback the control is an ordinary `Send` value: a scoped thread
// may drive it, and the scope ends before the callback returns.
use clingox::application::Application;

fn main() {
    let _ = Application::new()
        .main(|ctl, _files| {
            std::thread::scope(|scope| {
                let control = &mut *ctl;
                scope
                    .spawn(move || control.add_base("a."))
                    .join()
                    .expect("the scoped thread does not panic")
            })?;
            ctl.add_base("b.")
        })
        .run(["--outf=3"]);
}
