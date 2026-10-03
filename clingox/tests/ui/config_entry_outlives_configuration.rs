// `Configuration::root` borrows the view, not the control for the view's full
// lifetime, because the view holds the control mutably. An entry cannot leave
// the block that owns the view (DESIGN S5).
// Expected: E0597, `conf` does not live long enough.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    let root = {
        let conf = ctl.configuration();
        conf.root().unwrap()
    };
    let _kind = root.kind();
}
