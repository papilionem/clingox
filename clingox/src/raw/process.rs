//! Process-wide guards for `clingo_main` (DESIGN S18).
//!
//! `clingo_main` is a whole program inside a library call. It keeps a
//! process-global singleton, installs signal handlers and never removes them,
//! and writes through C stdio. Nothing here calls clingo. The pieces are:
//! - [`RunFlag`]: at most one run at a time, taken without blocking;
//! - [`Containment`]: flushes the output buffers around the run and puts the
//!   signal dispositions back, on every path that returns.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, ErrorKind};

/// Whether a run is in progress on any thread.
static RUN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The right to run clingo's application, held for the whole run.
///
/// It is taken with a single non-blocking attempt, so a second run (nested,
/// concurrent, or started from a callback on a solver thread) is refused at
/// once instead of waiting for the run that would have to end first. Dropping
/// it releases the flag, on every exit path including an unwinding panic.
#[derive(Debug)]
pub(crate) struct RunFlag(());

impl RunFlag {
    /// Takes the flag, or returns `None` if a run is in progress.
    pub(crate) fn try_acquire() -> Option<RunFlag> {
        RUN_ACTIVE
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| RunFlag(()))
    }
}

impl Drop for RunFlag {
    fn drop(&mut self) {
        RUN_ACTIVE.store(false, Ordering::Release);
    }
}

/// Flushes Rust's standard output and error, then every C stdio stream.
///
/// clingo writes through C stdio, which has its own buffer; Rust's `stdout()`
/// has another. Flushing both at the boundaries keeps their output in order
/// and makes text that was pending before the run survive a `_exit` path.
pub(crate) fn flush_all() {
    flush_rust_stdout();
    // A failed flush of a closed pipe has nowhere to be reported.
    let _ = std::io::stderr().flush();
    flush_c_stdio();
}

/// Flushes Rust's standard output only.
pub(crate) fn flush_rust_stdout() {
    // A failed flush of a closed pipe has nowhere to be reported.
    let _ = std::io::stdout().flush();
}

/// Flushes every C stdio stream (`fflush(NULL)`), where clingo's output waits.
pub(crate) fn flush_c_stdio() {
    #[cfg(unix)]
    {
        // SAFETY: fflush(NULL) flushes all open output streams and takes no
        // other argument; it is safe to call at any time.
        unsafe { libc::fflush(std::ptr::null_mut()) };
    }
}

/// The signals whose dispositions clasp's application changes (F4, F18).
#[cfg(unix)]
const SIGNALS: [libc::c_int; 9] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGQUIT,
    libc::SIGHUP,
    libc::SIGXCPU,
    libc::SIGXFSZ,
    libc::SIGALRM,
];

/// The dispositions of [`SIGNALS`] as they were when the run began.
#[cfg(unix)]
struct SavedSignals([Option<libc::sigaction>; SIGNALS.len()]);

#[cfg(unix)]
impl SavedSignals {
    /// Reads the dispositions. `Err` names the signal `sigaction` refused.
    fn save() -> Result<Self, libc::c_int> {
        let mut saved = [None; SIGNALS.len()];
        for (slot, &signal) in saved.iter_mut().zip(&SIGNALS) {
            // SAFETY: `sigaction` is a plain C struct for which all-zero bytes
            // are a valid value; it is overwritten below.
            let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
            // SAFETY: a NULL new action only reads the current one into the
            // valid out-pointer `old`; `signal` is a valid signal number.
            if unsafe { libc::sigaction(signal, std::ptr::null(), &raw mut old) } != 0 {
                return Err(signal);
            }
            *slot = Some(old);
        }
        Ok(SavedSignals(saved))
    }

    /// Writes the saved dispositions back with the nine signals blocked on this
    /// thread, so a signal aimed at this thread waits until it is done. This
    /// narrows the window after `clingo_main` cleared its singleton but cannot
    /// close it: the kernel may deliver a process-directed signal to another
    /// thread, which then runs clasp's handler with a null instance (U38).
    ///
    /// Returns the first signal `sigaction` refused to restore. POSIX lets
    /// `sigaction` fail only for an invalid signal number or for SIGKILL and
    /// SIGSTOP (`EINVAL`), and a disposition that was read back is valid, so
    /// on a conforming system this cannot happen for these nine signals. The
    /// failure is reported anyway rather than assumed away: a restore that
    /// failed cannot be undone, and the caller should know that clasp's
    /// handler is still installed.
    fn restore(&self) -> Option<libc::c_int> {
        let mut failed = None;
        // SAFETY: `sigset_t` is a plain C struct; `sigemptyset` initialises it
        // before any use, and the signal numbers are valid.
        let mut block: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: as above, `block` is a valid out-pointer.
        unsafe { libc::sigemptyset(&raw mut block) };
        for &signal in &SIGNALS {
            // SAFETY: `block` is an initialised set and `signal` is valid.
            unsafe { libc::sigaddset(&raw mut block, signal) };
        }
        // SAFETY: as for `block`; `previous` receives the calling thread's mask.
        let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: both sets are valid; blocking signals on this thread is
        // always allowed.
        let masked =
            unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const block, &raw mut previous) }
                == 0;
        for (saved, &signal) in self.0.iter().zip(&SIGNALS) {
            if let Some(action) = saved {
                // SAFETY: `action` was read by `sigaction` for this signal, so
                // it is a valid disposition (a handler, SIG_DFL or SIG_IGN);
                // a NULL old-action pointer is allowed.
                if unsafe { libc::sigaction(signal, action, std::ptr::null_mut()) } != 0 {
                    failed.get_or_insert(signal);
                }
            }
        }
        if masked {
            // SAFETY: `previous` was filled by the successful call above.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &raw const previous, std::ptr::null_mut());
            }
        }
        failed
    }
}

/// The state around one run: the flushed buffers, the saved signal
/// dispositions and the run flag. [`Containment::finish`] undoes all three in
/// the order of steps 9 to 11 of `Application::run` and reports a failed
/// restore; dropping it does the same when an unwind skips `finish`.
///
/// The dispositions written back are those saved at entry, so a disposition
/// another thread changed while the run was in progress is overwritten.
pub(crate) struct Containment {
    #[cfg(unix)]
    signals: Option<SavedSignals>,
    /// The signal that could not be restored, kept for `finish`.
    failed: Option<i32>,
    /// Released last, after the buffers and signals are back to normal.
    _flag: RunFlag,
}

impl Containment {
    /// Flushes the output buffers and saves the signal dispositions. If the
    /// dispositions cannot be read nothing has changed yet, and the flag is
    /// released on return.
    #[cfg_attr(
        not(unix),
        expect(
            clippy::unnecessary_wraps,
            reason = "only the Unix path can fail to read the signal dispositions; \
                      the signature is shared with it"
        )
    )]
    pub(crate) fn enter(flag: RunFlag) -> Result<Self, Error> {
        flush_all();
        #[cfg(unix)]
        let signals = SavedSignals::save().map_err(|signal| {
            Error::new(
                ErrorKind::Unknown,
                format!("could not read the disposition of signal {signal}; nothing was run"),
            )
        })?;
        Ok(Containment {
            #[cfg(unix)]
            signals: Some(signals),
            failed: None,
            _flag: flag,
        })
    }

    /// Restores the signals, flushes, and releases the flag. Returns the
    /// signal that could not be restored, if any (see `SavedSignals::restore`).
    pub(crate) fn finish(mut self) -> Result<(), Error> {
        self.restore_signals();
        let failed = self.failed;
        // Dropping `self` flushes once more (cheap) and releases the flag.
        match failed {
            None => Ok(()),
            Some(signal) => Err(Error::new(
                ErrorKind::Unknown,
                format!("could not restore the disposition of signal {signal} after the run"),
            )),
        }
    }

    /// Puts the signal dispositions back now. `run` calls it the moment
    /// `clingo_main` returns, so no user `Drop` runs with clasp's handler
    /// installed; later calls do nothing. A failure is kept for `finish`.
    pub(crate) fn restore_signals(&mut self) {
        let failed = self.restore();
        if self.failed.is_none() {
            self.failed = failed;
        }
    }

    #[cfg_attr(
        not(unix),
        expect(
            clippy::unused_self,
            reason = "there are no signal dispositions to put back off Unix"
        )
    )]
    fn restore(&mut self) -> Option<i32> {
        #[cfg(unix)]
        {
            self.signals.take().and_then(|saved| saved.restore())
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

impl Drop for Containment {
    fn drop(&mut self) {
        // On the unwind path `finish` did not run; a failure here has nowhere
        // to go, and cannot happen for valid signals (see `restore`).
        let failed = self.restore();
        debug_assert!(failed.is_none(), "restoring signal {failed:?} failed");
        flush_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_taken_once_and_released_on_drop() {
        // One test owns the flag; the others in this crate never take it.
        let first = RunFlag::try_acquire().expect("free at first");
        assert!(RunFlag::try_acquire().is_none(), "a second attempt fails");
        drop(first);
        let again = RunFlag::try_acquire();
        assert!(again.is_some(), "released by the drop");
    }
}
