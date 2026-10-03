// An entry of the writable statistics borrows it for the length of the `&self`
// borrow of `root` (DESIGN S5), and the statistics are lent only for the
// `on_statistics` call, so the entry cannot be stored anywhere that lives
// longer. A handler that stores an entry is rejected twice, once here and once
// because an entry is not `Send` (`stats_entry_is_not_send.rs`), and the second
// error would hide this one, so the store is written as a function.
// Expected: E0621, explicit lifetime required in the type of `step`.
use clingox::{MutableStatistics, StatsEntry};

fn keep<'a>(step: &MutableStatistics<'_>, slot: &mut Option<StatsEntry<'a>>) {
    *slot = Some(step.root());
}

fn main() {
    let _ = keep;
}
