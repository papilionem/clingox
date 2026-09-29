//! `clingo_main`: the run context, its callbacks and the call itself
//! (DESIGN S18).
//!
//! The context lives on the stack of [`run`], which blocks until clingo and
//! every thread it started are done, so the callbacks may borrow from the
//! caller. Nothing here is thread-local: the logger can run on a solver thread,
//! so the panic slot belongs to the context.

use std::cell::RefCell;
use std::ffi::{CString, OsStr, c_char, c_int, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

use clingox_sys as ffi;

use super::capture::message_code;
use super::control::{Arena, ControlHandle};
use super::exit_options::{self, ShortKind};
use super::interrupt::SolveSync;
use super::model::printer_model;
use super::process::{Containment, RunFlag, flush_c_stdio, flush_rust_stdout};
use super::script::{ScriptFrame, enter_run, freeze};
use super::trampoline::{LogSink, PanicSlot, Slot, guard, logger};
use super::{ClingoErrorState, ErrorState, borrowed_str, c_str, call, check_version, raw_slice};
use crate::Model;
use crate::application::{DefaultPrinter, Flag, Options};
use crate::control::ScopedControl;
use crate::error::{Error, ErrorKind, Message, MessageCode, Result};

/// The logger of an application: it borrows from the caller for `'a`, and the
/// mutex serialises calls, because clingo may log from several threads.
pub(crate) type AppLogger<'a> = Box<dyn FnMut(MessageCode, &str) + Send + 'a>;

/// The application's `main` callback. The control it receives has a brand
/// `'r` that is universally quantified, so nothing borrowed from the control
/// can leave the call (DESIGN S19).
pub(crate) type AppMain<'a> =
    Box<dyn for<'r> FnOnce(&mut ScopedControl<'r>, &[&str]) -> Result<()> + 'a>;

/// The application's model printer. It runs on whichever thread found the
/// model, possibly while `main` runs, hence `Send + Sync`; the printer it is
/// lent has a lifetime `'p` that only exists for the call.
pub(crate) type AppPrinter<'a> =
    Box<dyn for<'p> Fn(&Model, &mut DefaultPrinter<'p>) -> Result<()> + Send + Sync + 'a>;

/// The application's `register_options` callback, run once while clingo
/// builds its command line.
pub(crate) type AppRegister<'a> = Box<dyn FnOnce(&mut Options<'a>) -> Result<()> + 'a>;

/// The application's `validate_options` callback, run once after parsing.
pub(crate) type AppValidate<'a> = Box<dyn FnOnce() -> Result<()> + 'a>;

/// The callback of one option that takes a value.
pub(crate) type AppParse<'o> = Box<dyn FnMut(&str) -> Result<()> + 'o>;

/// What the caller configured, as plain Rust values.
pub(crate) struct Settings<'a> {
    pub(crate) main: Option<AppMain<'a>>,
    pub(crate) register_options: Option<AppRegister<'a>>,
    pub(crate) validate_options: Option<AppValidate<'a>>,
    pub(crate) program_name: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) message_limit: Option<u32>,
    pub(crate) logger: Option<AppLogger<'a>>,
    pub(crate) print_model: Option<AppPrinter<'a>>,
}

/// The `main` callback, in a cell `RunContext` can share.
///
/// The closure is not `Send`. It is taken and run only by the `main`
/// trampoline after that has checked that it is on the thread that called
/// `run` (clingo invokes `main` on the thread that called `clingo_main`), and
/// dropped by `run` on that same thread, so it never moves between threads.
struct MainSlot<'a>(Mutex<Option<SameThread<AppMain<'a>>>>);

/// A value that is only touched on the thread that called `run`.
struct SameThread<T>(T);

// SAFETY: see `MainSlot`: the value is only used on the thread that created
// it, which the trampoline checks before it touches the slot.
unsafe impl<T> Send for SameThread<T> {}

/// What the data pointer of the application callbacks points to.
///
/// The `CString`s are read by clingo after `clingo_main` copied their
/// addresses (the name is used again for `--help`), so they live here, as long
/// as the run (F9).
struct RunContext<'a> {
    program_name: Option<CString>,
    version: Option<CString>,
    message_limit: Option<u32>,
    logger: Option<Mutex<AppLogger<'a>>>,
    /// The model printer, called by whichever thread found the model.
    printer: Option<AppPrinter<'a>>,
    /// Set when the printer failed (an error, a panic, or a failing
    /// `print`): the printer is not called again for this run.
    printer_failed: AtomicBool,
    /// How the printer stops the search when it fails: the interrupt state of
    /// the control `main` received, set by the `main` trampoline.
    interrupt: Arc<Mutex<Option<Arc<SolveSync>>>>,
    /// The first panic of the logger or of `main`, resumed after clingo
    /// returns.
    panic: PanicSlot,
    main: MainSlot<'a>,
    /// The thread that called `run`, where `main` must run.
    thread: ThreadId,
    /// The error a callback returned (`main`, an option's `parse`,
    /// `register_options`, `validate_options`), first writer wins, returned by
    /// `run` in place of the exit code.
    main_error: Slot<Error>,
    /// What the borrowed control's observers and propagators go to.
    arena: Arc<Arena>,
    /// The `register_options` callback, taken by its trampoline.
    register: Mutex<Option<SameThread<AppRegister<'a>>>>,
    /// The `validate_options` callback, taken by its trampoline.
    validate: Mutex<Option<SameThread<AppValidate<'a>>>>,
    /// The options this run registered, alive until `clingo_main` has returned.
    options: Mutex<SameThread<Registered<'a>>>,
    /// The command-line arguments, for the check that runs once the
    /// application's own one-letter aliases are known.
    arguments: Vec<String>,
    /// Whether the arguments select `--mode=clasp`.
    clasp_mode: bool,
}

impl RunContext<'_> {
    /// A context with nothing configured, for the thread that calls this.
    fn empty() -> Self {
        RunContext {
            program_name: None,
            version: None,
            message_limit: None,
            logger: None,
            printer: None,
            printer_failed: AtomicBool::new(false),
            interrupt: Arc::default(),
            panic: PanicSlot::default(),
            main: MainSlot(Mutex::new(None)),
            thread: thread::current().id(),
            main_error: Slot::default(),
            arena: Arc::default(),
            register: Mutex::new(None),
            validate: Mutex::new(None),
            options: Mutex::new(SameThread(Registered::default())),
            arguments: Vec::new(),
            clasp_mode: false,
        }
    }

    /// Drops every callback that is still held, on the calling thread. `run`
    /// does it after clingo returned and before a stored panic is resumed.
    fn release(&self) {
        // Taken out of the locks first, so a destructor cannot run under one.
        let register = lock(&self.register).take();
        let validate = lock(&self.validate).take();
        let main = lock(&self.main.0).take();
        let options = std::mem::take(&mut lock(&self.options).0);
        drop((register, validate, main, options));
    }
}

/// Locks `mutex`, ignoring poisoning: no lock here is held across user code.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The state behind the options one run registered.
///
/// A parse closure's address is handed to clingo, so it lives in a box that is
/// only ever turned into a pointer, never moved as a `Box` (a `Vec` slot
/// would move when the vector grows). A flag's cell is shared, so its address
/// is stable too.
#[derive(Default)]
struct Registered<'a> {
    parse: Vec<NonNull<ParseEntry<'a>>>,
    flags: Vec<FlagEntry>,
    /// The one-letter aliases of the options clingo accepted, for the check
    /// of grouped short options (`exit_options`).
    aliases: Vec<(char, ShortKind)>,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        for entry in self.parse.drain(..) {
            // SAFETY: each pointer came from `Box::into_raw` in `add` and is
            // freed only here, once, after clingo returned.
            drop(unsafe { Box::from_raw(entry.as_ptr()) });
        }
    }
}

/// What the data pointer of an option's `parse` callback points to.
struct ParseEntry<'a> {
    parse: RefCell<AppParse<'a>>,
    /// The run's context, which outlives every callback.
    context: *mut RunContext<'a>,
}

/// A flag option: clasp writes `cell`; the value is copied into `flag` at the
/// start of the validate step.
struct FlagEntry {
    cell: Arc<AtomicBool>,
    flag: Flag,
}

impl fmt::Debug for RunContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunContext")
            .field("program_name", &self.program_name)
            .field("version", &self.version)
            .field("message_limit", &self.message_limit)
            .field("logger", &self.logger.is_some())
            .field("printer", &self.printer.is_some())
            .field("printer_failed", &self.printer_failed)
            .field("panic", &self.panic)
            .field("main", &self.main.0.lock().is_ok_and(|m| m.is_some()))
            .field("main_error", &self.main_error)
            .field("register", &lock(&self.register).is_some())
            .field("validate", &lock(&self.validate).is_some())
            .finish_non_exhaustive()
    }
}

impl LogSink for RunContext<'_> {
    fn message(&self, code: c_int, text: String) {
        let Some(logger) = &self.logger else {
            return;
        };
        // The same text and code as `ControlBuilder::logger` receives.
        let message = Message::from_clingo(message_code(code), &text);
        // The panic is caught by the trampoline while the guard is held, so the
        // mutex is never poisoned by the user's code; the guard tolerates it.
        let mut logger = logger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        logger(message.code(), message.text());
    }

    fn panic_slot(&self) -> &PanicSlot {
        &self.panic
    }
}

/// The `program_name` callback. It never returns NULL, which would crash
/// clingo (F9).
///
/// # Safety
///
/// `data` must point to a live `RunContext`.
unsafe extern "C" fn program_name(data: *mut c_void) -> *const c_char {
    // SAFETY: the caller passes the context registered with `clingo_main`,
    // which outlives the call; only shared access is taken.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    context
        .program_name
        .as_deref()
        .unwrap_or(c"clingo")
        .as_ptr()
}

/// The `version` callback. It never returns NULL (F9).
///
/// # Safety
///
/// `data` must point to a live `RunContext`.
unsafe extern "C" fn version(data: *mut c_void) -> *const c_char {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    context.version.as_deref().unwrap_or(c"").as_ptr()
}

/// The `message_limit` callback; it runs no user code, so it cannot panic.
///
/// # Safety
///
/// `data` must point to a live `RunContext`.
unsafe extern "C" fn message_limit(data: *mut c_void) -> u32 {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    context.message_limit.unwrap_or(crate::raw::MESSAGE_LIMIT)
}

/// The `main` callback: builds the borrowed control, runs the user's closure
/// on it and reports a failure to clingo, which prints it and exits with 65.
///
/// The control lives on this frame and is dropped before returning, inside
/// the panic guard, so an open search is closed before clingo tears the
/// control down.
///
/// # Safety
///
/// `data` must point to a live `RunContext`; `control` must be the control
/// clingo passes to `main`; `files` must be null or point to `size` C strings
/// valid for the call (clingo.h:4300).
unsafe extern "C" fn main_callback(
    control: *mut ffi::clingo_control_t,
    files: *const *const c_char,
    size: usize,
    data: *mut c_void,
) -> bool {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    let Some(pointer) = NonNull::new(control) else {
        fail(c"clingo passed no control to main");
        return false;
    };
    if !admit(context, c"the main callback") {
        return false;
    }
    let Some(SameThread(main)) = lock(&context.main.0).take() else {
        fail(c"the main callback was called twice");
        return false;
    };
    // SAFETY: clingo passes `size` strings (or a null array for none) valid for
    // this call; the borrow ends with it.
    let pointers: &[*const c_char] = unsafe { raw_slice(files, size) };
    let outcome = guard(&context.panic, || {
        // Each string came from a `&str` argument of `run`, so this cannot
        // fail; a failure is an error, not undefined behaviour.
        let files = pointers
            .iter()
            // SAFETY: each pointer is null or a NUL-terminated string valid for
            // this call (the caller's contract).
            .map(|&file| unsafe { borrowed_str(file) })
            .collect::<Result<Vec<&str>>>()?;
        // SAFETY: `pointer` is the live control clingo passed to `main`, and it
        // stays live until this function returns, which is after the closure
        // (and the handle it drops) is done, by return or by unwinding.
        unsafe {
            with_borrowed_control(pointer, &context.arena, &context.interrupt, |control| {
                main(control, &files)
            })
        }
    });
    report(
        context,
        outcome,
        c"the application's main callback panicked",
    )
}

/// Turns what a callback did into clingo's answer: `true` for success, else
/// `false` after setting clingo's error state. An error is kept for `run` to
/// return (the first one wins); a panic is already in the context's slot.
fn report(
    context: &RunContext<'_>,
    outcome: Option<Result<()>>,
    panicked: &std::ffi::CStr,
) -> bool {
    match outcome {
        Some(Ok(())) => true,
        Some(Err(err)) => {
            let text = err.to_string().replace('\0', "\u{fffd}");
            context.main_error.store(err);
            if let Ok(text) = CString::new(text) {
                ClingoErrorState::set(ffi::clingo_error_runtime, &text);
            } else {
                fail(c"the callback failed");
            }
            false
        }
        None => {
            fail(panicked);
            false
        }
    }
}

/// The checks every callback that runs user code makes first: the thread
/// that called `run`, and no earlier panic. Sets clingo's error and returns
/// `false` when one fails.
fn admit(context: &RunContext<'_>, what: &std::ffi::CStr) -> bool {
    if thread::current().id() != context.thread {
        let text = format!("clingo called {} on another thread", what.to_string_lossy());
        fail(&CString::new(text).unwrap_or_default());
        return false;
    }
    if context.panic.is_set() {
        fail(c"an earlier callback panicked");
        return false;
    }
    true
}

/// The `register_options` callback: lends the option registry to the user's
/// closure. It runs once per run, whatever number of times clingo builds its
/// options (the closure is taken).
///
/// # Safety
///
/// `data` must point to a live `RunContext`; `options` must be the options
/// object clingo passes, valid during this call.
unsafe extern "C" fn register_callback(
    options: *mut ffi::clingo_options_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    let Some(pointer) = NonNull::new(options) else {
        fail(c"clingo passed no options to register_options");
        return false;
    };
    if !admit(context, c"the register_options callback") {
        return false;
    }
    let Some(SameThread(register)) = lock(&context.register).take() else {
        // Already run: clingo asked again, and the options are registered.
        return true;
    };
    let outcome = guard(&context.panic, || {
        let handle = OptionsHandle {
            pointer,
            context: std::ptr::from_ref(context).cast_mut(),
            _lifetime: PhantomData,
        };
        register(&mut Options::from_handle(handle))?;
        // The application's one-letter aliases are known now and clingo has
        // not parsed yet: a short-option group such as `-mo text` can hide an
        // option that ends the process, so the arguments are checked once
        // more and the run stops here, before anything is parsed.
        let aliases = lock(&context.options).0.aliases.clone();
        let texts: Vec<&str> = context.arguments.iter().map(String::as_str).collect();
        if let Some(why) = exit_options::ends_process_with(&texts, &aliases) {
            return Err(Error::new(ErrorKind::InvalidInput, why));
        }
        // Past the last refusal: clingo goes on to parse and, in clasp mode,
        // to open the input (U51).
        if context.clasp_mode {
            CLASP_MODE_STARTED.store(true, Ordering::Release);
        }
        Ok(())
    });
    report(context, outcome, c"the register_options callback panicked")
}

/// The `parse` callback of an option that takes a value.
///
/// # Safety
///
/// `data` must be the `ParseEntry` registered with this callback, live for
/// the run; `value` must be a NUL-terminated string valid for the call.
unsafe extern "C" fn parse_callback(value: *const c_char, data: *mut c_void) -> bool {
    // SAFETY: `data` is the entry `OptionsHandle::add` registered; entries are
    // freed only after clingo returned. Only shared access is taken.
    let entry = unsafe { &*data.cast::<ParseEntry<'_>>().cast_const() };
    // SAFETY: the entry's context pointer is the run's context, alive for the
    // whole `clingo_main` call.
    let context = unsafe { &*entry.context.cast_const() };
    if !admit(context, c"an option's parse callback") {
        return false;
    }
    let outcome = guard(&context.panic, || {
        // SAFETY: clingo passes a NUL-terminated string valid for this call.
        let value = unsafe { borrowed_str(value) }?;
        let mut parse = entry.parse.try_borrow_mut().map_err(|_| {
            Error::new(
                ErrorKind::Logic,
                "an option's parse callback was entered while it was running",
            )
        })?;
        (*parse)(value)
    });
    report(context, outcome, c"an option's parse callback panicked")
}

/// The `validate_options` callback. It first copies each flag's result into
/// the caller's `Flag`, then runs the user's closure, if any.
///
/// # Safety
///
/// `data` must point to a live `RunContext`.
unsafe extern "C" fn validate_callback(data: *mut c_void) -> bool {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    if !admit(context, c"the validate_options callback") {
        return false;
    }
    for entry in &lock(&context.options).0.flags {
        entry.flag.store(entry.cell.load(Ordering::Acquire));
    }
    let validate = lock(&context.validate).take();
    let outcome = guard(&context.panic, || match validate {
        Some(SameThread(validate)) => validate(),
        None => Ok(()),
    });
    report(context, outcome, c"the validate_options callback panicked")
}

/// Clingo's own model printer, lent to the user's printer for one call.
///
/// `print` flushes Rust's standard output before and C stdio after the
/// default printer, so text the closure wrote with `println!` and clingo's
/// own text stay in program order on a pipe or a file (F13).
pub(crate) struct PrinterHandle<'p> {
    function: unsafe extern "C" fn(*mut c_void) -> bool,
    data: *mut c_void,
    /// Borrowed for one call, and `!Send + !Sync`: clasp holds the C `stdout`
    /// lock while the default printer runs, so another thread that called it
    /// would wait for a lock the printing thread holds while it waits.
    _lifetime: PhantomData<(&'p mut (), *mut u8)>,
}

impl PrinterHandle<'_> {
    /// Calls clingo's default model printer once.
    pub(crate) fn print(&mut self) -> Result<()> {
        flush_rust_stdout();
        // SAFETY: `function` and `data` are the pair clingo passed to the
        // printer callback, valid until that callback returns, and this handle
        // does not outlive it (its lifetime is a borrow of the call).
        let result = call(|| unsafe { (self.function)(self.data) });
        flush_c_stdio();
        result
    }
}

/// Records that the printer failed and stops the search, as `main`'s control
/// allows. The callback still answers `true` to clingo.
fn printer_failed(context: &RunContext<'_>) {
    context.printer_failed.store(true, Ordering::Release);
    if let Some(sync) = lock(&context.interrupt).clone() {
        // `false` when no search is running or the build cannot interrupt a
        // blocking search: the printer stays silent for the remaining models.
        let _stopped = sync.interrupt();
    }
}

/// The model printer callback: lends the model and the default printer to the
/// user's closure. It may run on any solver thread. It never answers `false`
/// (a failing closure is recorded and answered with `true`, after which the
/// closure and the default printer are skipped and the search is interrupted),
/// and no panic crosses it.
///
/// # Safety
///
/// `data` must point to a live `RunContext`; `model` must be the live model
/// clingo passes, and `printer` and `printer_data` the default printer for
/// it, valid for the call.
unsafe extern "C" fn printer_callback(
    model: *const ffi::clingo_model_t,
    printer: ffi::clingo_default_model_printer_t,
    printer_data: *mut c_void,
    data: *mut c_void,
) -> bool {
    // SAFETY: as for `program_name`.
    let context = unsafe { &*data.cast::<RunContext<'_>>().cast_const() };
    let (Some(function), Some(user)) = (printer, context.printer.as_ref()) else {
        return true;
    };
    if model.is_null() || context.printer_failed.load(Ordering::Acquire) || context.panic.is_set() {
        return true;
    }
    // Pending C output (from `main`, say) goes before anything the closure
    // writes.
    flush_c_stdio();
    let outcome = guard(&context.panic, || {
        // SAFETY: clingo passes a live model that stays valid for this call.
        let model = unsafe { printer_model(model) };
        let mut default = DefaultPrinter::from_handle(PrinterHandle {
            function,
            data: printer_data,
            _lifetime: PhantomData,
        });
        user(model, &mut default)
    });
    flush_rust_stdout();
    flush_c_stdio();
    match outcome {
        Some(Ok(())) => {}
        Some(Err(err)) => {
            context.main_error.store(err);
            printer_failed(context);
        }
        None => printer_failed(context),
    }
    true
}

/// The option registry, valid during `register_options` (DESIGN S3).
pub(crate) struct OptionsHandle<'o> {
    pointer: NonNull<ffi::clingo_options_t>,
    /// Invariant in `'o`: the closures stored through it live as long as the
    /// context does.
    context: *mut RunContext<'o>,
    _lifetime: PhantomData<*mut &'o ()>,
}

impl<'o> OptionsHandle<'o> {
    /// Registers an option that takes a value (`clingo_options_add`).
    pub(crate) fn add(
        &mut self,
        group: &str,
        option: &str,
        description: &str,
        multi: bool,
        argument: Option<&str>,
        parse: AppParse<'o>,
    ) -> Result<()> {
        let group = c_str(group)?;
        let option = c_str(option)?;
        let description = c_str(&escape_percent(description))?;
        let argument = argument.map(c_str).transpose()?;
        let entry = NonNull::from(Box::leak(Box::new(ParseEntry {
            parse: RefCell::new(parse),
            context: self.context,
        })));
        // SAFETY: `pointer` is the options object of the running registration;
        // the strings are NUL-terminated and copied before the call returns
        // (clingo_app.cc:94-120); `entry` stays alive until `Registered` is
        // dropped, after clingo returned, and `parse_callback` expects it.
        let added = call(|| unsafe {
            ffi::clingo_options_add(
                self.pointer.as_ptr(),
                group.as_ptr(),
                option.as_ptr(),
                description.as_ptr(),
                Some(parse_callback),
                entry.as_ptr().cast(),
                multi,
                argument.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
            )
        });
        // SAFETY: the context is alive for the whole registration.
        let context = unsafe { &*self.context };
        let mut registered = lock(&context.options);
        // Kept even when clingo refused the option, and freed with the rest.
        registered.0.parse.push(entry);
        if added.is_ok()
            && let Some(alias) = exit_options::alias_of(option.to_str().unwrap_or_default())
        {
            registered.0.aliases.push((alias, ShortKind::Required));
        }
        added
    }

    /// Registers a flag (`clingo_options_add_flag`); its result reaches `flag`
    /// at the start of the validate step.
    pub(crate) fn add_flag(
        &mut self,
        group: &str,
        option: &str,
        description: &str,
        flag: &Flag,
    ) -> Result<()> {
        let group = c_str(group)?;
        let option = c_str(option)?;
        let description = c_str(&escape_percent(description))?;
        let cell = Arc::new(AtomicBool::new(flag.get()));
        // SAFETY: as for `add`; clasp keeps the target until `clingo_main`
        // returns and writes it on this thread during parsing. The run holds
        // `cell` until then, and nobody else reads it before validate.
        let added = call(|| unsafe {
            ffi::clingo_options_add_flag(
                self.pointer.as_ptr(),
                group.as_ptr(),
                option.as_ptr(),
                description.as_ptr(),
                cell.as_ptr(),
            )
        });
        // SAFETY: the context is alive for the whole registration.
        let context = unsafe { &*self.context };
        if added.is_ok() {
            let mut registered = lock(&context.options);
            registered.0.flags.push(FlagEntry {
                cell,
                flag: flag.clone(),
            });
            if let Some(alias) = exit_options::alias_of(option.to_str().unwrap_or_default()) {
                registered.0.aliases.push((alias, ShortKind::Flag));
            }
        }
        added
    }
}

/// clingo's help formatter treats `%` as an escape: it swallows the `%` and
/// the next character, so a description is shown literally only with each `%`
/// doubled (U43).
fn escape_percent(text: &str) -> String {
    text.replace('%', "%%")
}

/// Lends the control clingo passes to a `main` callback, for the length of `f`.
///
/// The value stays on this frame and is lent as `&mut` to a closure that is
/// higher-ranked over the brand, so the closure cannot move it out, keep it,
/// or keep anything borrowed from it. It is dropped before this returns, by
/// return or by unwinding, so an open search is closed before clingo tears the
/// control down. Shared by the application's `main` and a script's `main`
/// (`raw::script`), which receive the same control (F17).
///
/// # Safety
///
/// `pointer` must be the live control clingo passed to a `main` callback, and
/// it must stay live until this function has returned.
pub(super) unsafe fn with_borrowed_control<R>(
    pointer: NonNull<ffi::clingo_control_t>,
    arena: &Arc<Arena>,
    interrupt: &Mutex<Option<Arc<SolveSync>>>,
    f: impl for<'r> FnOnce(&mut ScopedControl<'r>) -> R,
) -> R {
    // SAFETY: the caller's contract; the handle is dropped below, before this
    // function returns.
    let handle = unsafe { ControlHandle::borrowed(pointer, Arc::clone(arena)) };
    // A failing printer stops the search through this (see `printer_callback`).
    *lock(interrupt) = Some(handle.interrupt_state());
    let mut control = ScopedControl::from_borrowed(handle);
    let result = f(&mut control);
    drop(control);
    result
}

/// Sets clingo's error state to a runtime error with `message`.
fn fail(message: &std::ffi::CStr) {
    ClingoErrorState::set(ffi::clingo_error_runtime, message);
}

type NameFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type LimitFn = unsafe extern "C" fn(*mut c_void) -> u32;
type MainFn = unsafe extern "C" fn(
    *mut ffi::clingo_control_t,
    *const *const c_char,
    usize,
    *mut c_void,
) -> bool;
type LoggerFn = unsafe extern "C" fn(c_int, *const c_char, *mut c_void);
type RegisterFn = unsafe extern "C" fn(*mut ffi::clingo_options_t, *mut c_void) -> bool;
type ValidateFn = unsafe extern "C" fn(*mut c_void) -> bool;
type PrinterFn = unsafe extern "C" fn(
    *const ffi::clingo_model_t,
    ffi::clingo_default_model_printer_t,
    *mut c_void,
    *mut c_void,
) -> bool;

/// Whether `argument` is an abbreviation of clasp's hidden `--fast-exit`, at
/// least `--fa`. clingo accepts every unambiguous prefix, and the option ends
/// the process with `_exit` after the run (F6). A single dash is never one
/// (`-f` is `--file`), and neither is `--f`, which clingo rejects as
/// ambiguous.
fn is_fast_exit(argument: &str) -> bool {
    let Some(rest) = argument.strip_prefix("--") else {
        return false;
    };
    let name = rest.split_once('=').map_or(rest, |(name, _)| name);
    name.len() >= 2 && "fast-exit".starts_with(name)
}

/// Converts the arguments for clingo, in order: not UTF-8 is `InvalidInput`, a
/// NUL byte is `Nul`, a `--fast-exit` prefix is `InvalidInput`, and finally
/// any combination that would end the process (`exit_options`) is
/// `InvalidInput`.
fn arguments_c(arguments: &[&OsStr]) -> Result<Vec<CString>> {
    let converted = arguments
        .iter()
        .map(|argument| {
            let text = argument.to_str().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("the argument {} is not valid UTF-8", argument.display()),
                )
            })?;
            let converted = c_str(text)?;
            if is_fast_exit(text) {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "the argument {text:?} would end the process: clingo's --fast-exit \
                         leaves with _exit after the run"
                    ),
                ));
            }
            Ok((text, converted))
        })
        .collect::<Result<Vec<_>>>()?;
    let texts: Vec<&str> = converted.iter().map(|(text, _)| *text).collect();
    if let Some(why) = exit_options::ends_process(&texts) {
        return Err(Error::new(ErrorKind::InvalidInput, why));
    }
    Ok(converted.into_iter().map(|(_, c)| c).collect())
}

/// Whether a run with `--mode=clasp` has started in this process. clasp opens
/// its input once per process, so a later clasp-mode run would read the first
/// run's stream and answer for the wrong file (U51).
static CLASP_MODE_STARTED: AtomicBool = AtomicBool::new(false);

/// The refusal of a second clasp-mode run.
fn clasp_mode_used() -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        "a run with --mode=clasp already started in this process: clasp opens its input once \
         per process, so a second one would read the first run's file and can answer for the \
         wrong problem (U51)",
    )
}

/// The refusal of a second run.
fn already_running() -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        "an application run is already in progress (a nested or concurrent run): clingo keeps \
         process-wide state for the run, so only one can be active in the process",
    )
}

/// Runs clingo's command line with `settings` and `arguments`, in the order of
/// operations of `Application::run`.
pub(crate) fn run(settings: Settings<'_>, arguments: &[&OsStr]) -> Result<i32> {
    // 1. Everything that can fail without side effects.
    let name_c = settings.program_name.as_deref().map(c_str).transpose()?;
    let version_c = settings.version.as_deref().map(c_str).transpose()?;
    let arguments = arguments_c(arguments)?;
    // 2. The linked library must be one the bindings match.
    check_version()?;
    // The printer's model reads crash clingo without a `main` (U40), so the
    // combination is refused before anything runs.
    if settings.print_model.is_some() && settings.main.is_none() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "print_model needs a main callback: clingo crashes when the model printer reads a \
             model and the application has no main (U40)",
        ));
    }
    // 3, 4. One run at a time in the process, refused rather than waited for.
    let flag = RunFlag::try_acquire().ok_or_else(already_running)?;
    let clasp_mode = {
        let texts: Vec<&str> = arguments.iter().filter_map(|a| a.to_str().ok()).collect();
        exit_options::selects_clasp_mode(&texts)
    };
    if clasp_mode && CLASP_MODE_STARTED.load(Ordering::Acquire) {
        return Err(clasp_mode_used());
    }
    // 5. Flush and save the signal dispositions.
    let mut containment = Containment::enter(flag)?;

    // The validate callback is also needed when only options are registered:
    // it is what hands a flag's result to the caller (see `validate_callback`).
    let with_options = settings.register_options.is_some();
    let with_validate = with_options || settings.validate_options.is_some();
    let context = RunContext {
        program_name: name_c,
        version: version_c,
        message_limit: settings.message_limit,
        logger: settings.logger.map(Mutex::new),
        printer: settings.print_model,
        main: MainSlot(Mutex::new(settings.main.map(SameThread))),
        register: Mutex::new(settings.register_options.map(SameThread)),
        validate: Mutex::new(settings.validate_options.map(SameThread)),
        arguments: arguments
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
        clasp_mode,
        ..RunContext::empty()
    };
    // A NULL callback makes clingo use its own default, so only what the caller
    // set is installed; the callbacks left unset stay NULL.
    let mut application = ffi::clingo_application_t {
        program_name: context
            .program_name
            .is_some()
            .then_some(program_name as NameFn),
        version: context.version.is_some().then_some(version as NameFn),
        message_limit: context
            .message_limit
            .is_some()
            .then_some(message_limit as LimitFn),
        main: context
            .main
            .0
            .lock()
            .is_ok_and(|main| main.is_some())
            .then_some(main_callback as MainFn),
        logger: context
            .logger
            .is_some()
            .then_some(logger::<RunContext<'_>> as LoggerFn),
        printer: context
            .printer
            .is_some()
            .then_some(printer_callback as PrinterFn),
        register_options: with_options.then_some(register_callback as RegisterFn),
        validate_options: with_validate.then_some(validate_callback as ValidateFn),
    };
    let argv: Vec<*const c_char> = arguments.iter().map(|a| a.as_ptr()).collect();
    let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
    // A script's `execute`, `callable`, `call` and `main` run inside the call:
    // their failures go to this frame, and the script `main` finds the run's
    // arena through the guard (`raw::script`).
    let scripts = ScriptFrame::open();
    let run_guard = enter_run(Arc::clone(&context.arena), Arc::clone(&context.interrupt));
    // 6. The one call. No script may be registered from here on; the freeze is
    // set here and not earlier, so a run refused by an argument check leaves
    // the registry open.
    freeze();
    // With `register_options` the flag is set after its re-check passes, so a
    // run refused there does not block a later clasp-mode run.
    if clasp_mode && !with_options {
        CLASP_MODE_STARTED.store(true, Ordering::Release);
    }
    // SAFETY: `application` is a valid struct whose callbacks are NULL or the
    // trampolines above, each of which expects `data`, a pointer to `context`.
    // `context`, `arguments` (behind `argv`) and `application` outlive the
    // call, which returns only after clingo and all its threads are done with
    // them. `argv` holds `argv.len()` NUL-terminated strings (clingo.h:4276).
    let code = unsafe { ffi::clingo_main(&raw mut application, argv.as_ptr(), argv.len(), data) };
    // 7. Signals back first, before any user `Drop` below can run: clasp's
    // handler is still installed until here, and a signal that reaches it
    // after clingo cleared its singleton crashes the process (U38).
    containment.restore_signals();
    drop(run_guard);
    let scripted = scripts.finish();
    // The observers and propagators the borrowed control kept alive go now,
    // after clingo's teardown and before anything can unwind.
    context.arena.drain();
    // The option callbacks go on this thread too, before a panic is resumed.
    context.release();
    // Buffers flushed and flag released, before anything else can run, in
    // particular before a panic resumes.
    let restored = containment.finish();
    // 8. A stored panic is resumed (the logger's wins over `main`'s), then a
    // failed restore is reported (a restore that failed cannot be undone; the
    // exit code is kept in the message), then the error `main` returned, else
    // the exit code.
    context.panic.resume();
    if let Some(payload) = scripted.panic {
        std::panic::resume_unwind(payload);
    }
    restored.map_err(|e| e.context(format!("clingo returned exit code {code}")))?;
    match context.main_error.take().or(scripted.error) {
        Some(err) => Err(err),
        None => Ok(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_prefixes_of_fast_exit_of_two_letters_or_more_are_filtered() {
        for hit in [
            "--fast-exit",
            "--fast-e",
            "--fast",
            "--fa",
            "--fast-exit=no",
            "--fa=1",
        ] {
            assert!(is_fast_exit(hit), "{hit}");
        }
        for miss in [
            "--f",
            "-f",
            "-fa",
            "fast-exit",
            "--no-fast-exit",
            "--fast-exits",
            "--time-limit=1",
            "--",
            "--=",
            "",
            "f.lp",
        ] {
            assert!(!is_fast_exit(miss), "{miss}");
        }
    }

    #[test]
    fn arguments_are_checked_in_order() {
        let ok = arguments_c(&[OsStr::new("a.lp"), OsStr::new("0")]).unwrap();
        assert_eq!(ok, [c"a.lp".to_owned(), c"0".to_owned()]);
        let nul = arguments_c(&[OsStr::new("a\0b")]).unwrap_err();
        assert_eq!(nul.kind(), ErrorKind::Nul);
        let filtered = arguments_c(&[OsStr::new("a.lp"), OsStr::new("--fa")]).unwrap_err();
        assert_eq!(filtered.kind(), ErrorKind::InvalidInput);
        assert!(filtered.to_string().contains("--fa"));
    }

    #[cfg(unix)]
    #[test]
    fn an_argument_that_is_not_utf8_is_refused() {
        use std::os::unix::ffi::OsStrExt;
        let bad = OsStr::from_bytes(b"\xff.lp");
        let err = arguments_c(&[bad]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("UTF-8"));
    }

    #[test]
    fn the_name_and_version_callbacks_never_return_null() {
        let context = RunContext::empty();
        let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
        // SAFETY: `data` points to a live context for both calls.
        let (name, text, limit) =
            unsafe { (program_name(data), version(data), message_limit(data)) };
        // SAFETY: both are non-null NUL-terminated strings from static data.
        let (name, text) = unsafe {
            (
                std::ffi::CStr::from_ptr(name),
                std::ffi::CStr::from_ptr(text),
            )
        };
        assert_eq!(name, c"clingo");
        assert_eq!(text, c"");
        assert_eq!(limit, 20);
    }

    /// The default printer of the tests below: counts its calls in `data`.
    unsafe extern "C" fn counting_printer(data: *mut c_void) -> bool {
        // SAFETY: `data` points to a live `AtomicUsize` in every test.
        unsafe { &*data.cast::<std::sync::atomic::AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// Calls the printer trampoline like clingo does, with a model pointer that
    /// is never read (`Model` has no bytes) and the counting default printer.
    fn print_one(context: &RunContext<'_>, default_calls: &std::sync::atomic::AtomicUsize) -> bool {
        let data = std::ptr::from_ref(context).cast_mut().cast::<c_void>();
        let counter = std::ptr::from_ref(default_calls)
            .cast_mut()
            .cast::<c_void>();
        // SAFETY: `data` is a live context; the model pointer is aligned and
        // non-null and only ever turned into a zero-sized reference; the
        // printer and its data are valid for the call.
        unsafe {
            printer_callback(
                NonNull::<ffi::clingo_model_t>::dangling().as_ptr(),
                Some(counting_printer),
                counter,
                data,
            )
        }
    }

    /// Whatever the closure did, clingo is told `true`, and
    /// after a failure neither the closure nor the default printer runs again.
    #[test]
    #[cfg_attr(miri, ignore = "the printer flushes C stdio and calls clingo")]
    fn a_failing_printer_is_answered_with_true_and_never_called_again() {
        use std::sync::atomic::AtomicUsize;

        for panics in [false, true] {
            let closure_calls = AtomicUsize::new(0);
            let default_calls = AtomicUsize::new(0);
            let context = RunContext {
                printer: Some(Box::new(|_model, printer| {
                    // The second model fails; the first and the rest would
                    // print.
                    let call = closure_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    printer.print()?;
                    assert!(!(panics && call == 2), "the printer panicked on purpose");
                    if call == 2 {
                        return Err(Error::new(ErrorKind::Logic, "printer failed"));
                    }
                    Ok(())
                })),
                ..RunContext::empty()
            };
            assert!(print_one(&context, &default_calls), "first model");
            assert!(!context.printer_failed.load(Ordering::SeqCst));
            for _ in 0..3 {
                assert!(print_one(&context, &default_calls), "after the failure");
            }
            assert!(context.printer_failed.load(Ordering::SeqCst));
            assert_eq!(closure_calls.load(Ordering::SeqCst), 2, "panics: {panics}");
            assert_eq!(default_calls.load(Ordering::SeqCst), 2, "panics: {panics}");
            assert_eq!(context.panic.is_set(), panics);
            assert_eq!(context.main_error.is_set(), !panics);
        }
    }

    #[test]
    fn the_context_logger_delivers_trimmed_text_and_keeps_the_first_panic() {
        let seen = Mutex::new(Vec::new());
        let context = RunContext {
            logger: Some(Mutex::new(Box::new(|code, text: &str| {
                assert!(text != "boom", "the logger panicked on purpose");
                seen.lock().unwrap().push((code, text.to_owned()));
            }))),
            ..RunContext::empty()
        };
        let data = std::ptr::from_ref(&context).cast_mut().cast::<c_void>();
        let send = |text: &std::ffi::CStr| {
            // SAFETY: `data` points to a live context and `text` is a valid C
            // string for the call.
            unsafe {
                logger::<RunContext<'_>>(
                    ffi::clingo_warning_atom_undefined.cast_signed(),
                    text.as_ptr(),
                    data,
                );
            }
        };
        send(c"x.lp:1:6-7: info: atom does not occur\n  b\n");
        send(c"boom");
        send(c"after");
        assert!(context.panic.is_set());
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                MessageCode::AtomUndefined,
                "x.lp:1:6-7: info: atom does not occur\n  b".to_owned()
            )]
        );
    }
}
