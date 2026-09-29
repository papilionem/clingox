// `Application::main` takes `FnOnce + 'a`: the closure may borrow the caller's
// locals mutably and the caller reads them after `run` (the `'a` rule,
// unchanged by the brand).
use clingox::application::Application;

fn main() {
    let mut seen: Vec<String> = Vec::new();
    let result = Application::new()
        .main(|ctl, files| {
            ctl.add_base("a.")?;
            seen.extend(files.iter().map(|f| (*f).to_owned()));
            seen.push("done".to_owned());
            Ok(())
        })
        .run(["--outf=3"]);
    assert!(result.is_ok());
    assert_eq!(seen.last().map(String::as_str), Some("done"));
}
