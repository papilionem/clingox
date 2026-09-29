//! The error type and the messages clingo logs (DESIGN S2, RULES 11.1).

use std::fmt;

/// The result type of clingox, with [`Error`] as the default error.
///
/// It is never glob-exported, so it cannot shadow a two-parameter `Result`
/// in user code.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error from clingo or from clingox's own checks.
///
/// Match on [`Error::kind`]. The [`Display`](fmt::Display) form is one line
/// that names the operation and its subject, followed by clingo's message.
/// Errors from parsing carry every message clingo logged during the call, with
/// its position, in [`Error::messages`].
///
/// # Examples
///
/// ```
/// use clingox::{Control, ErrorKind};
///
/// let mut ctl = Control::new()?;
/// let err = ctl.add_base("a :- b c.").unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::Parse);
/// let location = err.messages()[0].location().unwrap();
/// assert_eq!((location.line(), location.column()), (1, 8));
/// # Ok::<(), clingox::Error>(())
/// ```
pub struct Error {
    kind: ErrorKind,
    /// The operation and its subject, such as "parsing program `base`".
    context: Option<String>,
    /// clingo's message, or clingox's own.
    message: String,
    messages: Vec<Message>,
    /// A user's own error, from [`Error::callback`]. An `Arc`, not a `Box`,
    /// so [`Error::repeatable_copy`] can share it instead of
    /// dropping it: a boxed `dyn std::error::Error` cannot be cloned without
    /// knowing its concrete type, but an `Arc` of one can, cheaply, without
    /// needing to.
    source: Option<std::sync::Arc<dyn std::error::Error + Send + Sync + 'static>>,
    /// Whether the error poisons the control whatever its kind (DESIGN S3).
    poison: Poison,
}

/// How an error poisons the control beyond what its kind decides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Poison {
    /// As the kind decides (`Control::note`).
    #[default]
    ByKind,
    /// Always, with the error itself as the cause, such as a failed close or
    /// a failed restore of the model limit.
    Always,
    /// Always, with another error as the cause: one that failed while this
    /// one was being reported, such as a failed close after a failed `get`.
    Cause(String),
    /// Never, whatever the kind: a propagator's `propagate`, `undo`, `check`
    /// or `decide` returned this error. `Control::
    /// note`'s by-kind default would otherwise poison it for `Parse`/
    /// `Logic`/`BadAlloc`/`Unknown` (S3's provisional default, before this
    /// was measured); a further `solve` on the same control completes
    /// normally after one of these (checked directly), so it
    /// must not poison whatever kind the error carries. Only `init`'s own
    /// error still poisons unconditionally ([`Poison::Always`]).
    Never,
}

impl Error {
    /// Makes an error of any `kind`, with `message` as its one-line text.
    ///
    /// clingox's own code uses this for every error it builds internally (from
    /// clingo's own codes, or from clingox's checks such as [`ErrorKind::Nul`]
    /// or [`ErrorKind::InvalidInput`]). It is also the way a
    /// [`GroundProgramObserver`](crate::observer::GroundProgramObserver) or a
    /// [`Control::ground_with`](crate::Control::ground_with) callback raises an
    /// error of a specific kind rather than wrapping a foreign error type with
    /// [`Error::callback`]: unlike `callback`, this does not set a `source()`,
    /// since there is no other error being wrapped.
    ///
    /// **[`ErrorKind::Poisoned`] and [`ErrorKind::GroundingLimit`] are reserved
    /// for clingox's own internals**. Nothing stops building one with either
    /// kind here, but doing so gives an ordinary `Error` with that `kind()`,
    /// not the effect the name suggests: only clingox's own internal poisoning
    /// logic, checking the kind of an error a call actually *returned*, ever
    /// poisons a control, so a hand-built `ErrorKind::Poisoned` value does not
    /// poison anything just by existing; likewise, a hand-built
    /// `ErrorKind::GroundingLimit` value does not interact with
    /// [`Control::ground_with_limit`](crate::Control::ground_with_limit)'s own
    /// guard at all, which decides on its own counts, never on an error's kind.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Error, ErrorKind};
    ///
    /// let err = Error::new(ErrorKind::InvalidInput, "the limit must be positive");
    /// assert_eq!(err.kind(), ErrorKind::InvalidInput);
    /// assert_eq!(err.to_string(), "the limit must be positive");
    /// ```
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            context: None,
            message: message.into(),
            messages: Vec::new(),
            source: None,
            poison: Poison::ByKind,
        }
    }

    /// Wraps an error raised by user code, such as a model closure, as an
    /// error of kind [`ErrorKind::Callback`].
    ///
    /// The user's error is kept whole: [`source`](std::error::Error::source)
    /// returns it, and `downcast_ref` recovers its type. The one-line
    /// [`Display`](fmt::Display) form names only the operation, as
    /// `the callback failed`, preceded by the operation clingox adds, such as
    /// ``grounding `base`: the callback failed``. It does not repeat the
    /// user's text, so a reporter that walks the chain of sources, as `anyhow`
    /// does, prints that text once. [`Debug`](fmt::Debug) shows the source.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Error, ErrorKind};
    ///
    /// let err = Error::callback(std::fmt::Error);
    /// assert_eq!(err.kind(), ErrorKind::Callback);
    /// assert_eq!(err.to_string(), "the callback failed");
    /// let source = std::error::Error::source(&err).unwrap();
    /// assert!(source.downcast_ref::<std::fmt::Error>().is_some());
    /// ```
    pub fn callback<E>(error: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut err = Error::new(ErrorKind::Callback, "the callback failed");
        err.source = Some(std::sync::Arc::new(error));
        err
    }

    /// Makes an error of kind [`ErrorKind::Conversion`] with `message`, for a
    /// hand-written [`ToSymbol`](crate::ToSymbol) or
    /// [`FromSymbol`](crate::FromSymbol) that meets a value it cannot convert.
    ///
    /// The [`Display`](fmt::Display) form is `message` on one line: its lines
    /// are joined by `; ` and a trailing period is dropped (RULES 11.1). The
    /// error has no source and no messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use clingox::{Error, ErrorKind};
    ///
    /// let err = Error::conversion("expected a colour, found `p(1)`");
    /// assert_eq!(err.kind(), ErrorKind::Conversion);
    /// assert_eq!(err.to_string(), "expected a colour, found `p(1)`");
    /// ```
    pub fn conversion(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Conversion, message)
    }

    /// Names the operation that failed; an existing context stays in front.
    pub(crate) fn context(mut self, context: impl Into<String>) -> Self {
        let context = context.into();
        self.context = Some(match self.context.take() {
            Some(inner) => format!("{context}: {inner}"),
            None => context,
        });
        self
    }

    pub(crate) fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub(crate) fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Puts `logged`, the messages clingo logged during the failed call, in
    /// front of the messages the error already carries.
    pub(crate) fn with_logged_messages(mut self, mut logged: Vec<Message>) -> Self {
        logged.append(&mut self.messages);
        self.messages = logged;
        self
    }

    /// Makes the error poison the control whatever its kind, with itself as
    /// the cause.
    pub(crate) fn poisoning(mut self) -> Self {
        if self.poison == Poison::ByKind {
            self.poison = Poison::Always;
        }
        self
    }

    /// Makes the error never poison the control, whatever its kind: see
    /// [`Poison::Never`].
    pub(crate) fn excused(mut self) -> Self {
        if self.poison == Poison::ByKind {
            self.poison = Poison::Never;
        }
        self
    }

    /// Makes the error poison the control whatever its kind, with `cause` as
    /// the cause: an error that happened while this one was on its way out.
    pub(crate) fn poisoned_by(mut self, cause: &Error) -> Self {
        self.poison = Poison::Cause(cause.to_string());
        self
    }

    /// How the error poisons the control beyond its kind.
    pub(crate) fn poison(&self) -> &Poison {
        &self.poison
    }

    /// A freshly owned copy of this error, for reporting it to more than one
    /// caller (a solve-event handler's own error must be reported on every
    /// later call on the same search, not only the first one to observe it).
    /// `Error` is not `Clone` in general, since a caller who only holds a
    /// `&Error` (most of the crate's own internals) has no reason to copy one;
    /// this exists for the one place that must. Every field copies cheaply:
    /// `source` is an `Arc`, not a `Box` (see its own doc comment), so this
    /// shares the same source object rather than dropping or duplicating it.
    pub(crate) fn repeatable_copy(&self) -> Error {
        Error {
            kind: self.kind,
            context: self.context.clone(),
            message: self.message.clone(),
            messages: self.messages.clone(),
            source: self.source.clone(),
            poison: self.poison.clone(),
        }
    }

    /// The message clingo or clingox gave, without context.
    pub(crate) fn raw_message(&self) -> &str {
        &self.message
    }

    /// What kind of error this is.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The messages clingo logged during the failed call, in order.
    ///
    /// For a parse error they say what is wrong and where. They are empty when
    /// clingo logged nothing.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The text the one-line `Display` shows after the context: the first error
    /// clingo logged, which says more than its generic "parsing failed", or
    /// clingo's message otherwise. A callback error names only the operation,
    /// whatever clingo logged meanwhile: its cause is the source.
    fn detail(&self) -> &str {
        if self.kind == ErrorKind::Callback {
            return &self.message;
        }
        self.messages
            .iter()
            .find(|m| m.is_error())
            .map_or(self.message.as_str(), Message::text)
    }
}

/// Writes `text` on one line, without a trailing period (RULES 11.1).
fn write_one_line(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    let mut first = true;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !first {
            f.write_str("; ")?;
        }
        first = false;
        f.write_str(line)?;
    }
    Ok(())
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(context) = &self.context {
            write!(f, "{context}: ")?;
        }
        let detail = self.detail().trim_end();
        write_one_line(f, detail.strip_suffix('.').unwrap_or(detail))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("context", &self.context)
            .field("message", &self.message)
            .field("messages", &self.messages)
            .field("source", &self.source)
            .field("poison", &self.poison)
            .finish()
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// The kinds of [`Error`].
///
/// The first five mirror clingo's error codes. The others come from clingox's
/// own checks. An interrupt or a timeout is never an error: it is a search
/// result ([`SolveResult::is_interrupted`](crate::SolveResult::is_interrupted),
/// [`Outcome::Unknown`](crate::Outcome::Unknown)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A program or term has a syntax error. The messages say where.
    Parse,
    /// clingo reported a runtime error, such as an invalid option.
    Runtime,
    /// clingo reported wrong use of its API.
    Logic,
    /// clingo could not allocate memory.
    BadAlloc,
    /// clingo failed without saying why, or with an error code clingox does not
    /// know.
    Unknown,
    /// A user callback failed.
    Callback,
    /// A value could not be converted.
    Conversion,
    /// clingo returned text that is not valid UTF-8.
    Utf8,
    /// A string passed to clingo contains a NUL byte.
    Nul,
    /// The [`Control`](crate::Control) was poisoned by an earlier error and can
    /// only be dropped.
    Poisoned,
    /// The linked clingo is outside the versions clingox accepts: 5.8.1 or
    /// newer within 5.8.
    Version,
    /// The operation needs something this build of clingo lacks, such as
    /// threads on the default WebAssembly target. It never poisons the
    /// control: clingox detects it before calling clingo.
    Unsupported,
    /// clingox rejected the input before calling clingo, such as a part name
    /// reserved for [`Control::add_facts`](crate::Control::add_facts) or a
    /// thread count outside 1 to 64. It never poisons the control. A NUL
    /// byte in a string is [`ErrorKind::Nul`] instead.
    InvalidInput,
    /// A registered [`GroundingLimit`](crate::observer::GroundingLimit) was
    /// exceeded during grounding (DESIGN S15), through
    /// [`LimitedObserver`](crate::observer::LimitedObserver) or
    /// [`Control::ground_with_limit`](crate::Control::ground_with_limit).
    /// This is entirely clingox's own guard, not a clingo error, but it
    /// poisons the control exactly as any other error that stops grounding
    /// partway does: clingo keeps the truncated program it
    /// already ground and would answer from it silently.
    GroundingLimit,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorKind::Parse => "parse error",
            ErrorKind::Runtime => "runtime error",
            ErrorKind::Logic => "logic error",
            ErrorKind::BadAlloc => "out of memory",
            ErrorKind::Unknown => "unknown error",
            ErrorKind::Callback => "callback error",
            ErrorKind::Conversion => "conversion error",
            ErrorKind::Utf8 => "invalid UTF-8",
            ErrorKind::Nul => "NUL byte in a string",
            ErrorKind::Poisoned => "poisoned control",
            ErrorKind::Version => "clingo version mismatch",
            ErrorKind::Unsupported => "unsupported operation",
            ErrorKind::InvalidInput => "invalid input",
            ErrorKind::GroundingLimit => "grounding-size limit exceeded",
        })
    }
}

/// A message clingo logged: a warning, an informational note, or the details
/// of an error.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Message {
    code: MessageCode,
    text: String,
    location: Option<Location>,
}

impl Message {
    pub(crate) fn from_clingo(code: MessageCode, text: &str) -> Self {
        let text = text.trim_end().to_owned();
        let location = Location::parse_prefix(&text);
        Message {
            code,
            text,
            location,
        }
    }

    /// Whether clingo reports an error with this message rather than a warning.
    pub(crate) fn is_error(&self) -> bool {
        self.code == MessageCode::RuntimeError
    }

    /// The kind of message.
    pub fn code(&self) -> MessageCode {
        self.code
    }

    /// The message as clingo wrote it, position prefix included, for example
    /// `<block>:1:8-9: error: syntax error, unexpected <IDENTIFIER>`.
    ///
    /// Trailing whitespace is removed. clingo ends most messages with a
    /// newline, which pyclingo passes on unchanged; lines inside the message
    /// are kept as they are.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where in the input the message points, if clingo gave a position.
    pub fn location(&self) -> Option<&Location> {
        self.location.as_ref()
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A position in a program, parsed from the prefix clingo writes at the start
/// of a message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    file: String,
    line: u32,
    column: u32,
}

impl Location {
    /// The file name, or `<block>` for program text passed to
    /// [`Control::add`](crate::Control::add).
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The line, starting at 1.
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The first column of the span, starting at 1.
    pub fn column(&self) -> u32 {
        self.column
    }

    /// Reads the position clingo writes before `: ` at the start of a message.
    ///
    /// gringo prints a location as `file:line:column`, followed by the end of
    /// the span as `-column`, `-line:column` or `-file:line:column` when it
    /// differs (libgringo/gringo/locatable.hh). File names may contain `:` and
    /// `-`, so the start is found as the shortest prefix that ends in
    /// `:line:column` and leaves a valid end, or nothing.
    fn parse_prefix(text: &str) -> Option<Location> {
        let (position, _) = text.split_once(": ")?;
        let mut cut = 0;
        loop {
            let (start, end) = match position[cut..].find('-') {
                Some(dash) => (&position[..cut + dash], Some(&position[cut + dash + 1..])),
                None => (position, None),
            };
            if let Some(location) = Self::parse_start(start)
                && end.is_none_or(Self::is_end)
            {
                return Some(location);
            }
            cut = start.len() + 1;
            end?;
        }
    }

    /// Parses `file:line:column`.
    fn parse_start(start: &str) -> Option<Location> {
        let mut parts = start.rsplitn(3, ':');
        let column = parts.next()?.parse().ok()?;
        let line = parts.next()?.parse().ok()?;
        let file = parts.next()?;
        (!file.is_empty()).then(|| Location {
            file: file.to_owned(),
            line,
            column,
        })
    }

    /// Accepts `column`, `line:column` or `file:line:column`.
    fn is_end(end: &str) -> bool {
        let mut parts = end.rsplitn(3, ':');
        let numbers = parts
            .by_ref()
            .take(2)
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        numbers && parts.next().is_none_or(|file| !file.is_empty())
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// The kind of a [`Message`], from clingo's warning codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessageCode {
    /// An undefined arithmetic operation or weight of an aggregate.
    OperationUndefined,
    /// An error; clingo raises the corresponding error after logging it.
    RuntimeError,
    /// An atom that occurs in no rule head.
    AtomUndefined,
    /// The same file was included more than once.
    FileIncluded,
    /// A CSP variable with an unbounded domain.
    VariableUnbounded,
    /// A global variable in the tuple of an aggregate element.
    GlobalVariable,
    /// Any other message.
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(text: &str) -> Option<(String, u32, u32)> {
        Location::parse_prefix(text).map(|l| (l.file, l.line, l.column))
    }

    #[test]
    fn locations_are_read_in_every_form_gringo_prints() {
        let expected = Some(("<block>".to_owned(), 1, 8));
        assert_eq!(location("<block>:1:8-9: error: syntax error"), expected);
        assert_eq!(location("<block>:1:8: error: x"), expected);
        assert_eq!(location("<block>:1:8-2:3: error: x"), expected);
        assert_eq!(location("<block>:1:8-other.lp:2:3: error: x"), expected);
    }

    #[test]
    fn file_names_may_contain_colons_and_dashes() {
        assert_eq!(
            location("C:\\a-b\\x-1.lp:12:3-5: info: y"),
            Some(("C:\\a-b\\x-1.lp".to_owned(), 12, 3))
        );
        assert_eq!(
            location("my-file:2:1-3:4: info: y"),
            Some(("my-file".to_owned(), 2, 1))
        );
    }

    #[test]
    fn messages_without_a_position_have_no_location() {
        assert_eq!(location("error: something"), None);
        assert_eq!(location("<block>:x:1: error"), None);
        assert_eq!(location("no separator"), None);
        assert_eq!(location(":1:2: error"), None);
    }

    #[test]
    fn display_is_one_line_without_a_trailing_period() {
        let err =
            Error::new(ErrorKind::Runtime, "first line.\n  second line.\n").context("doing x");
        assert_eq!(err.to_string(), "doing x: first line.; second line");
    }

    #[test]
    fn context_nests_outside_in() {
        let err = Error::new(ErrorKind::Runtime, "m")
            .context("inner")
            .context("outer");
        assert_eq!(err.to_string(), "outer: inner: m");
    }

    #[test]
    fn unsupported_reads_as_an_unsupported_operation() {
        assert_eq!(ErrorKind::Unsupported.to_string(), "unsupported operation");
    }

    #[test]
    fn poisoning_and_excused_only_override_the_default() {
        let poisoned = Error::new(ErrorKind::Logic, "m").poisoning();
        assert_eq!(*poisoned.poison(), Poison::Always);

        let excused = Error::new(ErrorKind::Logic, "m").excused();
        assert_eq!(
            *excused.poison(),
            Poison::Never,
            "an excused error never poisons"
        );

        // Neither overrides a poison state that is already more specific
        // than the `ByKind` default (mirrors `poisoning`'s own existing
        // conditional, kept symmetric for `excused`).
        let already_always = Error::new(ErrorKind::Logic, "m").poisoning().excused();
        assert_eq!(*already_always.poison(), Poison::Always);

        let cause = Error::new(ErrorKind::Logic, "m")
            .poisoned_by(&Error::new(ErrorKind::Runtime, "closing"));
        let still_cause = cause.excused();
        assert!(matches!(still_cause.poison(), Poison::Cause(_)));
    }

    #[test]
    fn errors_are_send_and_sync() {
        fn traits<T: std::error::Error + Send + Sync + 'static>() {}
        traits::<Error>();
    }
}
