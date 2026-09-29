//! Configuring a control before it is created: arguments, the user logger,
//! the message limit and the typed shortcuts for common options.

use std::ffi::CString;
use std::fmt;

use crate::control::{Control, ScopedControl};
use crate::error::{Error, ErrorKind, MessageCode, Result};
use crate::raw::{self, ControlHandle, UserLogger};

/// Builds a [`Control`] with arguments, a logger and a message limit.
///
/// [`Control::builder`] starts one. `Control::builder().build()` is
/// [`Control::new`], and `Control::builder().args(a).build()` is
/// [`Control::with_args`].
///
/// # Examples
///
/// ```
/// use std::sync::{Arc, Mutex};
///
/// use clingox::{Control, MessageCode, Part};
///
/// let seen = Arc::new(Mutex::new(Vec::new()));
/// let sink = Arc::clone(&seen);
/// let mut ctl = Control::builder()
///     .args(["--models=0"])
///     .seed(7)
///     .logger(move |code: MessageCode, text: &str| {
///         sink.lock().unwrap().push((code, text.to_owned()));
///     })
///     .build()?;
/// ctl.add_base("a :- b.")?;
/// ctl.ground(&[Part::base()])?;
/// assert_eq!(seen.lock().unwrap()[0].0, MessageCode::AtomUndefined);
/// # Ok::<(), clingox::Error>(())
/// ```
#[must_use]
#[derive(Default)]
pub struct ControlBuilder {
    args: Vec<String>,
    logger: Option<UserLogger>,
    /// `None` is clingo's default of 20; `Default` cannot express it.
    message_limit: Option<u32>,
    threads: Option<u32>,
    seed: Option<u32>,
}

/// The most solver threads clingo accepts (`--parallel-mode`).
const MAX_THREADS: u32 = 64;

impl ScopedControl<'static> {
    /// Starts a [`ControlBuilder`], for a control with a logger, a message
    /// limit or typed options.
    ///
    /// # Examples
    ///
    /// ```
    /// let ctl = clingox::Control::builder().args(["--models=0"]).build()?;
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn builder() -> ControlBuilder {
        ControlBuilder::default()
    }
}

impl ControlBuilder {
    /// Appends command-line options, as the `clingo` program takes them.
    ///
    /// Calling it again appends to the list. Only grounding and solving options
    /// are accepted, as for [`Control::with_args`]; they are checked by
    /// [`build`](Self::build).
    pub fn args<I, S>(mut self, args: I) -> ControlBuilder
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// Sets the logger that receives clingo's messages, with their code and
    /// text.
    ///
    /// The text is [`Message::text`](crate::Message::text): clingo's message
    /// with its position prefix and without trailing whitespace. With a logger
    /// set, messages are no longer sent to the `log` crate. Either way they
    /// are still captured during each call and attached to its error
    /// ([`Error::messages`]).
    ///
    /// The control owns the logger and drops it with itself. clingo may call
    /// it from any thread, including solver threads (DESIGN S10), hence
    /// `Send + 'static`; clingox never calls it twice at once. A panic in the
    /// logger is caught: the rest of the call proceeds without calling the
    /// logger again, and the panic resumes on the caller's thread when the
    /// call returns. It does not poison the control.
    pub fn logger<F>(mut self, logger: F) -> ControlBuilder
    where
        F: FnMut(MessageCode, &str) + Send + 'static,
    {
        self.logger = Some(Box::new(logger));
        self
    }

    /// Sets clingo's message limit; the default is 20.
    ///
    /// clingo counts the warnings and informational messages it passes on and
    /// drops those over the limit. It still passes errors on with a limit of
    /// 0; the limit only makes it give up ("too many messages") when errors
    /// exceed it. The limit applies to everything clingox receives, so it also
    /// bounds the messages captured into an [`Error`].
    pub fn message_limit(mut self, limit: u32) -> ControlBuilder {
        self.message_limit = Some(limit);
        self
    }

    /// Solves with `threads` solver threads, as `--parallel-mode=<threads>`.
    ///
    /// clingo accepts 1 to 64, and [`build`](Self::build) checks that range
    /// itself before anything else. Parallel solving finds models in an order
    /// that can change between runs (DESIGN S16). A build of clingo without
    /// threads (the default WebAssembly target) has one thread: `threads(1)`
    /// passes nothing there, and any other value makes
    /// [`build`](Self::build) fail.
    pub fn threads(mut self, threads: u32) -> ControlBuilder {
        self.threads = Some(threads);
        self
    }

    /// Sets the solver's random seed, as `--seed=<seed>`; the default is 1.
    pub fn seed(mut self, seed: u32) -> ControlBuilder {
        self.seed = Some(seed);
        self
    }

    /// Creates the control.
    ///
    /// The arguments are passed to clingo in order, followed by
    /// `--parallel-mode` and `--seed` when [`threads`](Self::threads) and
    /// [`seed`](Self::seed) were used.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] for a [`threads`](Self::threads) count
    ///   outside 1 to 64, on every build;
    /// - [`ErrorKind::Logic`] for an unknown option, a bad value, or an option
    ///   given twice, such as `seed` together with `--seed` in the arguments;
    /// - [`ErrorKind::Parse`] for a `-c` (`--const`) definition that is not
    ///   valid, such as `-c a` or `-c a=b c`; the messages say where;
    /// - [`ErrorKind::Runtime`] for a `--configuration` that is neither a
    ///   preset nor a file clingo can read;
    /// - [`ErrorKind::Unsupported`] for more than one thread on a build of
    ///   clingo without threads;
    /// - [`ErrorKind::Nul`] if an argument contains a NUL byte;
    /// - [`ErrorKind::BadAlloc`] and [`ErrorKind::Version`] as for
    ///   [`Control::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Control, ErrorKind};
    ///
    /// let ctl = Control::builder().seed(42).message_limit(5).build()?;
    /// let err = Control::builder().args(["--seed=1"]).seed(2).build().unwrap_err();
    /// assert_eq!(err.kind(), ErrorKind::Logic);
    /// # Ok::<(), clingox::Error>(())
    /// ```
    pub fn build(self) -> Result<Control> {
        let context = "creating a control";
        // Checked first, so that the same count is refused the same way on
        // every build, with or without threads.
        if let Some(threads) = self.threads
            && !(1..=MAX_THREADS).contains(&threads)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "{threads} solver threads are outside clingo's range of 1 to {MAX_THREADS}"
                ),
            )
            .context(context));
        }
        let mut args = self
            .args
            .iter()
            .map(|a| raw::c_str(a))
            .collect::<Result<Vec<CString>>>()
            .map_err(|e| e.context(context))?;
        let mut push = |option: String| {
            raw::c_str(&option)
                .map(|option| args.push(option))
                .map_err(|e| e.context(context))
        };
        if let Some(threads) = self.threads {
            if raw::HAS_THREADS {
                push(format!("--parallel-mode={threads}"))?;
            } else if threads != 1 {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("{threads} solver threads need a build of clingo with threads"),
                )
                .context(context));
            }
        }
        if let Some(seed) = self.seed {
            push(format!("--seed={seed}"))?;
        }
        let limit = self.message_limit.unwrap_or(raw::MESSAGE_LIMIT);
        let handle = ControlHandle::new(&args, self.logger, limit).map_err(|e| {
            // clingo reports a `-c` definition that does not parse as a plain
            // runtime error, like a configuration file that cannot be read.
            // Only the first logs an error with a position, so that tells
            // them apart.
            let parse = e.kind() == ErrorKind::Runtime
                && e.messages()
                    .iter()
                    .any(|m| m.is_error() && m.location().is_some());
            let e = if parse {
                e.with_kind(ErrorKind::Parse)
            } else {
                e
            };
            e.context(context)
        })?;
        Ok(Control::from_handle(handle))
    }
}

impl fmt::Debug for ControlBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlBuilder")
            .field("args", &self.args)
            .field("logger", &self.logger.is_some())
            .field(
                "message_limit",
                &self.message_limit.unwrap_or(raw::MESSAGE_LIMIT),
            )
            .field("threads", &self.threads)
            .field("seed", &self.seed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_shows_the_arguments_and_whether_a_logger_is_set() {
        let text = format!("{:?}", Control::builder().args(["--models=0"]).threads(2));
        assert!(text.contains("--models=0"), "{text}");
        assert!(text.contains("logger: false"), "{text}");
        assert!(text.contains("message_limit: 20"), "{text}");
        let text = format!("{:?}", Control::builder().logger(|_, _| {}));
        assert!(text.contains("logger: true"), "{text}");
    }
}
