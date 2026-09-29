//! The model printer of an [`Application`](super::Application): clingo's own
//! printer, lent to the closure of
//! [`print_model`](super::Application::print_model).

use std::fmt;

use crate::error::Result;
use crate::raw::PrinterHandle;

/// Clingo's own model printer, lent to the closure of
/// [`Application::print_model`](super::Application::print_model) for the length
/// of one call.
///
/// [`print`](DefaultPrinter::print) writes the model the way clingo would.
/// A closure that does not call it prints nothing for that model, so it can
/// replace clingo's output or wrap it.
///
/// It is only ever lent as `&mut`, for a lifetime shorter than the call, so it
/// cannot be kept. It is neither `Send` nor `Sync`, and that matters: clasp
/// holds the C `stdout` lock for the whole print, so a thread that called the
/// default printer while the printing thread waited for it would deadlock. It
/// is not `Clone` or `Copy`, and cannot be created.
pub struct DefaultPrinter<'p>(PrinterHandle<'p>);

impl<'p> DefaultPrinter<'p> {
    pub(crate) fn from_handle(handle: PrinterHandle<'p>) -> Self {
        DefaultPrinter(handle)
    }
}

impl DefaultPrinter<'_> {
    /// Prints the current model the way clingo would.
    ///
    /// Rust's standard output is flushed before and C stdio after the call, so
    /// text the closure wrote with `println!` before or after stays in order
    /// with clingo's text on a pipe or a file. The C stdio flush exists on
    /// Unix targets only; elsewhere the order is not promised. Calling it twice prints the
    /// model twice, and not calling it prints nothing for this model.
    ///
    /// # Errors
    ///
    /// clingo's error, if the default printer fails (only if writing the text
    /// throws). Return it from the closure, which fails the run with it.
    pub fn print(&mut self) -> Result<()> {
        self.0.print()
    }
}

impl fmt::Debug for DefaultPrinter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DefaultPrinter { .. }")
    }
}
