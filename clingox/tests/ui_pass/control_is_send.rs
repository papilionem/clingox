// A Control may move to another thread (DESIGN S12): clingo keeps no
// thread-affine state that a moved control depends on (item 3).
fn main() {
    let mut ctl = clingox::Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    let ctl = std::thread::spawn(move || {
        let mut ctl = ctl;
        ctl.ground(&[clingox::Part::base()]).unwrap();
        ctl
    })
    .join()
    .unwrap();
    std::thread::spawn(move || drop(ctl)).join().unwrap();
}
