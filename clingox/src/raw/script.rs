//! Custom scripting languages: `clingo_register_script`, its trampolines, the
//! registry and the freeze rule (DESIGN S18, S8's documented exception).
//!
//! clingo's script registry is a plain vector that grounding reads without a
//! lock, `call`, `callable` and `main` of a script are called without a null
//! check, and a `free` callback runs during static destruction (U41). So a
//! script is registered only before the process has created a control or run
//! an application ([`freeze`]), under a mutex, once per name, with every
//! member of `clingo_script_t` set except `free`, and the script is leaked on
//! purpose.
//!
//! The trampolines carry the process-global script as their data, which cannot
//! name a control, so an error or a panic goes into a thread-local slot that
//! the API call which can run script code opens a [`ScriptFrame`] for. That is
//! the one thread-local in clingox's callback machinery; it rests on a
//! measurement: clingo runs script callbacks only on the thread that called
//! `clingo_control_add`, `_load`, `_ground` or `clingo_main`, never on a
//! solver thread.

use std::any::Any;
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard};

use clingox_sys as ffi;

use super::application::with_borrowed_control;
use super::control::{Arena, ControlHandle};
use super::interrupt::SolveSync;
use super::symbol::static_str;
use super::trampoline::fail;
use super::{
    ClingoErrorState, ErrorState, borrowed_str_lossy, c_str, call, check_version, raw_slice,
};
use crate::Symbol;
use crate::ast::Span;
use crate::error::{Error, ErrorKind};
use crate::script::Script;

type PanicPayload = Box<dyn Any + Send>;

// ---------------------------------------------------------------------------
// The registry and the freeze
// ---------------------------------------------------------------------------

/// What the registration mutex protects.
struct Registry {
    /// Set once a control was created (or tried) or an application started.
    /// Never reset: clingo's own "has executed a block" flag is not either.
    frozen: bool,
    /// The names registered through this module.
    names: Vec<String>,
    /// The leaked script and version string of every accepted registration.
    /// Never read and never dropped: clingo's own registry vector is destroyed
    /// at exit, so without this static owner the allocations would become
    /// unreachable and `LeakSanitizer` would report them.
    kept: Vec<Kept>,
}

/// The two allocations of one accepted registration, as raw pointers.
struct Kept {
    #[expect(dead_code, reason = "held only so the allocation stays reachable")]
    script: *mut c_void,
    #[expect(dead_code, reason = "held only so the allocation stays reachable")]
    version: *mut c_char,
}

// SAFETY: the pointers are never dereferenced through this struct, only kept
// so the allocations stay reachable; the script behind one is `Send + Sync`.
unsafe impl Send for Kept {}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    frozen: false,
    names: Vec::new(),
    kept: Vec::new(),
});

fn lock(registry: &Mutex<Registry>) -> MutexGuard<'_, Registry> {
    // The only user code that runs under this lock is a refused script's
    // `Drop`, and that runs after the guard is gone (see `register_in`), so a
    // poisoned lock holds intact data.
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Forbids registering scripts from now on, in this process.
///
/// Called immediately before the first `clingo_control_new` or `clingo_main`
/// of every path, including one that then fails, and taken under the mutex
/// registration holds: when it returns, no registration is in flight and none
/// will start.
pub(crate) fn freeze() {
    lock(&REGISTRY).frozen = true;
}

/// Registers `script` under `name` (clingo.h:4322).
///
/// Refused, in this order: a NUL byte in either string (`Nul`), a process that
/// has frozen the registry and a name registered before (`InvalidInput`). A
/// refused script is dropped at once; an accepted one is never dropped.
pub(crate) fn register(name: &str, version: &str, script: Box<dyn Script>) -> Result<(), Error> {
    register_in(&REGISTRY, name, version, script, &|name, script, data| {
        // SAFETY: `name` is NUL-terminated and `script` a valid struct, both
        // for the call; clingo copies the struct and the name. `data` points
        // to a leaked `Box<dyn Script>` that stays valid for the rest of the
        // process, and `script.version` to a leaked string (clingo.h:4322).
        call(|| unsafe { ffi::clingo_register_script(name.as_ptr(), script, data) })
    })
}

fn register_in(
    registry: &Mutex<Registry>,
    name: &str,
    version: &str,
    script: Box<dyn Script>,
    register: &dyn Fn(&CStr, &ffi::clingo_script_t, *mut c_void) -> Result<(), Error>,
) -> Result<(), Error> {
    let name_c = c_str(name)?;
    let version_c = c_str(version)?;
    let mut registry = lock(registry);
    if registry.frozen {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "the scripting language `{name}` cannot be registered: scripts must be registered \
                 before the process creates a Control or runs an application, because clingo's \
                 script registry is not synchronised with grounding"
            ),
        ));
    }
    if registry.names.iter().any(|known| known == name) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "the scripting language `{name}` is already registered (clingo would run every \
                 block of it twice)"
            ),
        ));
    }
    // Both allocations are leaked on purpose: clingo reads the version for the
    // life of the process, and calls the script until it exits.
    let version_ptr = version_c.into_raw();
    let data = Box::into_raw(Box::new(script)).cast::<c_void>();
    let raw = ffi::clingo_script_t {
        execute: Some(execute::<ClingoErrorState> as ExecuteFn),
        call: Some(call_function::<ClingoErrorState> as CallFn),
        callable: Some(callable::<ClingoErrorState> as CallableFn),
        main: Some(main_function::<ClingoErrorState> as MainFn),
        // clingo runs `free` during static destruction, after Rust may be gone
        // (U41), so it is never installed.
        free: None,
        version: version_ptr,
    };
    match register(&name_c, &raw, data) {
        Ok(()) => {
            registry.names.push(name.to_owned());
            registry.kept.push(Kept {
                script: data,
                version: version_ptr,
            });
            Ok(())
        }
        Err(err) => {
            drop(registry);
            // SAFETY: clingo did not take the script, so nothing else refers to
            // the two allocations made above, each made by `into_raw` from the
            // matching type and reclaimed exactly once, here.
            let (version, script) = unsafe {
                (
                    CString::from_raw(version_ptr),
                    Box::from_raw(data.cast::<Box<dyn Script>>()),
                )
            };
            drop(version);
            drop(script);
            Err(err)
        }
    }
}

/// The version text given at registration, or `None` for a name that is not
/// registered (clingo.h:4327).
pub(crate) fn version(name: &str) -> Option<String> {
    check_version().ok()?;
    let name = c_str(name).ok()?;
    // The lookup walks clingo's script list, which a registration on another
    // thread may reallocate; `register` holds this lock across its call, so
    // no registration overlaps the read.
    let _registry = lock(&REGISTRY);
    // SAFETY: `name` is NUL-terminated and outlives the call. The result is
    // null or a string valid for the rest of the process, copied at once
    // (clingo.h:4327).
    unsafe { borrowed_str_lossy(ffi::clingo_script_version(name.as_ptr())) }
}

// ---------------------------------------------------------------------------
// Frames and slots
// ---------------------------------------------------------------------------

/// What a script callback left behind: at most one error and one panic, first
/// writer wins.
#[derive(Default)]
pub(crate) struct Failure {
    pub(crate) error: Option<Error>,
    pub(crate) panic: Option<PanicPayload>,
}

thread_local! {
    /// The slots of the innermost open frame on this thread, or `None` when
    /// no API call that can run script code is in progress.
    static CURRENT: RefCell<Option<Failure>> = const { RefCell::new(None) };

    /// The arena of the run in progress on this thread, for the script `main`
    /// trampoline.
    static RUN: RefCell<Option<RunState>> = const { RefCell::new(None) };
}

/// The slots of one API call that can run script code: `add`, `load`,
/// `ground`, `ground_with`, the program builder's `add` and `run`.
///
/// Opening it saves whatever the enclosing frame on this thread holds and
/// starts empty; [`ScriptFrame::finish`] returns what this call collected and
/// puts the saved content back. So a stale value cannot leak into the next
/// call, and a call made from inside a script callback (a `main` that adds to
/// its control) does not mix its failure with the outer one's.
///
/// Dropping it without `finish`, as an unwinding panic does, restores the
/// saved content and discards what was collected.
pub(crate) struct ScriptFrame {
    /// The slots of the enclosing frame, `None` if there was none.
    saved: Option<Failure>,
    /// Whether the frame is still open, that is `finish` has not run.
    open: bool,
}

impl ScriptFrame {
    pub(crate) fn open() -> ScriptFrame {
        let saved = CURRENT.with(|current| current.replace(Some(Failure::default())));
        ScriptFrame { saved, open: true }
    }

    /// Closes the frame and returns what its call collected.
    pub(crate) fn finish(mut self) -> Failure {
        self.open = false;
        let saved = self.saved.take();
        CURRENT
            .with(|current| current.replace(saved))
            .unwrap_or_default()
    }
}

impl Drop for ScriptFrame {
    fn drop(&mut self) {
        if self.open {
            let saved = self.saved.take();
            CURRENT.with(|current| current.replace(saved));
        }
    }
}

/// Makes `arena` the run in progress on this thread until the guard is dropped,
/// which is where the script `main` trampoline takes it from.
pub(crate) struct RunGuard(Option<RunState>);

/// What the script `main` trampoline needs from the run: its arena and the
/// slot a failing printer interrupts the search through.
#[derive(Clone)]
pub(crate) struct RunState {
    arena: Arc<Arena>,
    interrupt: Arc<Mutex<Option<Arc<SolveSync>>>>,
}

pub(crate) fn enter_run(
    arena: Arc<Arena>,
    interrupt: Arc<Mutex<Option<Arc<SolveSync>>>>,
) -> RunGuard {
    RunGuard(RUN.with(|run| run.replace(Some(RunState { arena, interrupt }))))
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        RUN.with(|run| run.replace(previous));
    }
}

/// Whether the open frame of this thread already holds a failure.
fn failed() -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_some_and(|f| f.error.is_some() || f.panic.is_some())
    })
}

fn record_error(err: Error) {
    CURRENT.with(|current| {
        if let Some(slots) = current.borrow_mut().as_mut() {
            slots.error.get_or_insert(err);
        }
    });
}

fn record_panic(payload: PanicPayload) {
    CURRENT.with(|current| {
        if let Some(slots) = current.borrow_mut().as_mut() {
            slots.panic.get_or_insert(payload);
        }
    });
}

impl ControlHandle {
    /// Closes `frame` and keeps its failure in this handle's slots, where the
    /// safe layer reads it after the call ([`ControlHandle::take_script_error`]).
    pub(crate) fn keep_script_failure(&self, frame: ScriptFrame) {
        let failure = frame.finish();
        if let Some(payload) = failure.panic {
            self.script_panic.store(payload);
        }
        if let Some(err) = failure.error {
            self.script_error.store(err);
        }
    }

    /// The panic a script callback raised during the last call, if any.
    pub(crate) fn take_script_panic(&self) -> Option<PanicPayload> {
        self.script_panic.take()
    }

    /// The error a script callback returned during the last call, if any.
    pub(crate) fn take_script_error(&self) -> Option<Error> {
        self.script_error.take()
    }
}

// ---------------------------------------------------------------------------
// Trampolines
// ---------------------------------------------------------------------------

type ExecuteFn =
    unsafe extern "C" fn(*const ffi::clingo_location_t, *const c_char, *mut c_void) -> bool;
type CallFn = unsafe extern "C" fn(
    *const ffi::clingo_location_t,
    *const c_char,
    *const ffi::clingo_symbol_t,
    usize,
    ffi::clingo_symbol_callback_t,
    *mut c_void,
    *mut c_void,
) -> bool;
type CallableFn = unsafe extern "C" fn(*const c_char, *mut bool, *mut c_void) -> bool;
type MainFn = unsafe extern "C" fn(*mut ffi::clingo_control_t, *mut c_void) -> bool;

/// Runs the user's code for one callback: nothing at all if the frame already
/// holds a failure, else inside `catch_unwind`, storing an error or a panic in
/// the frame and setting clingo's error state so clingo stops (the reported
/// error is the frame's, never clingo's copy). `poison` marks the stored
/// error as poisoning whatever its kind.
fn run_script<S: ErrorState, R>(poison: bool, f: impl FnOnce() -> Result<R, Error>) -> Option<R> {
    if failed() {
        fail::<S>(c"an earlier script callback failed");
        return None;
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Some(value),
        Ok(Err(err)) => {
            // The text is what `clingo_main` prints as `*** ERROR: (clingo): ..`.
            let text = err.to_string().replace('\0', "\u{fffd}");
            record_error(if poison { err.poisoning() } else { err });
            match CString::new(text) {
                Ok(text) => S::set(ffi::clingo_error_unknown, &text),
                Err(_) => fail::<S>(c"a script callback failed"),
            }
            None
        }
        Err(payload) => {
            record_panic(payload);
            fail::<S>(c"a script callback panicked");
            None
        }
    }
}

/// The script behind the callback data.
///
/// # Safety
///
/// `data` must be the pointer given to `clingo_register_script`: a leaked
/// `Box<dyn Script>` that is never freed.
unsafe fn script_of<'a>(data: *mut c_void) -> &'a dyn Script {
    // SAFETY: the caller's contract; the allocation lives for the process and
    // only shared access is taken (`Script: Sync`).
    unsafe { &**data.cast::<Box<dyn Script>>().cast_const() }
}

/// A span from clingo's location, or an empty one for a null pointer.
///
/// # Safety
///
/// `location` must be null or point to a location valid for the call, whose
/// file names are null or interned strings.
unsafe fn span_of(location: *const ffi::clingo_location_t) -> Span {
    // SAFETY: the caller's contract.
    let Some(loc) = (unsafe { location.as_ref() }) else {
        return Span::from_raw_parts("", "", 0, 0, 0, 0);
    };
    // SAFETY: location file names are interned by clingo and valid for the
    // rest of the process (clingo.h:205-209); null is allowed.
    let (begin_file, end_file) = unsafe { (static_str(loc.begin_file), static_str(loc.end_file)) };
    Span::from_raw_parts(
        begin_file,
        end_file,
        loc.begin_line,
        loc.end_line,
        loc.begin_column,
        loc.end_column,
    )
}

/// The `execute` callback (clingo.h:4285).
///
/// # Safety
///
/// `data` as for [`script_of`]; `location` as for [`span_of`]; `code` null or
/// a NUL-terminated string valid for the call.
unsafe extern "C" fn execute<S: ErrorState>(
    location: *const ffi::clingo_location_t,
    code: *const c_char,
    data: *mut c_void,
) -> bool {
    // SAFETY: the caller's contract.
    let (script, span, code) = unsafe {
        (
            script_of(data),
            span_of(location),
            borrowed_str_lossy(code).unwrap_or_default(),
        )
    };
    run_script::<S, _>(true, || script.execute(&span, &code)).is_some()
}

/// The `call` callback: the returned symbols reach clingo in one
/// `symbol_callback` call, none in no call (clingo.h:4296).
///
/// # Safety
///
/// `data` as for [`script_of`], `location` as for [`span_of`]; `name` null or
/// a NUL-terminated string and `arguments` null or `arguments_size` symbols,
/// both valid for the call; `symbol_callback` and its data as clingo passed
/// them for this call.
unsafe extern "C" fn call_function<S: ErrorState>(
    location: *const ffi::clingo_location_t,
    name: *const c_char,
    arguments: *const ffi::clingo_symbol_t,
    arguments_size: usize,
    symbol_callback: ffi::clingo_symbol_callback_t,
    symbol_callback_data: *mut c_void,
    data: *mut c_void,
) -> bool {
    // SAFETY: the caller's contract; `Symbol` is `repr(transparent)` over
    // `clingo_symbol_t`.
    let (script, span, name, args) = unsafe {
        (
            script_of(data),
            span_of(location),
            borrowed_str_lossy(name).unwrap_or_default(),
            raw_slice(arguments.cast::<Symbol>(), arguments_size),
        )
    };
    let Some(values) = run_script::<S, _>(true, || script.call(&span, &name, args)) else {
        return false;
    };
    if values.is_empty() {
        // No symbols is a call without a value.
        return true;
    }
    let Some(symbol_callback) = symbol_callback else {
        fail::<S>(c"clingo passed no symbol callback");
        return false;
    };
    // SAFETY: `symbol_callback` and its data are what clingo passed for this
    // call, and `values` holds `values.len()` symbols (`Symbol` is
    // `repr(transparent)`) that outlive it; clingo copies them (clingo.h:4296).
    unsafe {
        symbol_callback(
            values.as_ptr().cast::<ffi::clingo_symbol_t>(),
            values.len(),
            symbol_callback_data,
        )
    }
}

/// The `callable` callback (clingo.h:4305).
///
/// # Safety
///
/// `data` as for [`script_of`]; `name` null or a NUL-terminated string valid
/// for the call; `result` null or a valid out-pointer.
unsafe extern "C" fn callable<S: ErrorState>(
    name: *const c_char,
    result: *mut bool,
    data: *mut c_void,
) -> bool {
    // SAFETY: the caller's contract.
    let (script, name) = unsafe {
        (
            script_of(data),
            borrowed_str_lossy(name).unwrap_or_default(),
        )
    };
    let Some(answer) = run_script::<S, _>(true, || script.callable(&name)) else {
        return false;
    };
    // SAFETY: `result` is null or a valid out-pointer (the caller's contract).
    if let Some(result) = unsafe { result.as_mut() } {
        *result = answer;
        true
    } else {
        fail::<S>(c"clingo passed no result pointer");
        false
    }
}

/// The `main` callback (clingo.h:4312): runs the script's `main` on the
/// control clingo passes, lent exactly as for an application's `main`.
///
/// # Safety
///
/// `data` as for [`script_of`]; `control` null or the live control clingo
/// passes to `main`, valid for the call.
unsafe extern "C" fn main_function<S: ErrorState>(
    control: *mut ffi::clingo_control_t,
    data: *mut c_void,
) -> bool {
    // SAFETY: the caller's contract.
    let script = unsafe { script_of(data) };
    let Some(pointer) = NonNull::new(control) else {
        fail::<S>(c"clingo passed no control to main");
        return false;
    };
    let Some(run) = RUN.with(|run| run.borrow().clone()) else {
        fail::<S>(c"a script main was called outside a run on this thread");
        return false;
    };
    run_script::<S, _>(false, || {
        // SAFETY: `pointer` is the live control clingo passed to `main`, which
        // stays live until this function returns, after the closure (and with
        // it the handle) is done, by return or by unwinding.
        unsafe {
            with_borrowed_control(pointer, &run.arena, &run.interrupt, |control| {
                script.main(control)
            })
        }
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::c_uint;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    thread_local! {
        static SET: Cell<Option<c_uint>> = const { Cell::new(None) };
    }

    /// An error state that records the code, so the trampolines run under
    /// Miri without clingo.
    enum Recorded {}

    impl ErrorState for Recorded {
        fn set(code: c_uint, _: &CStr) {
            SET.with(|s| s.set(Some(code)));
        }
        fn code() -> std::ffi::c_int {
            0
        }
        fn message() -> Option<String> {
            None
        }
    }

    /// Counts what it was asked, fails or panics on request, and counts drops.
    struct Probe {
        drops: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        fail_with: Option<ErrorKind>,
        panic: bool,
    }

    impl Probe {
        fn new() -> (Probe, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let drops = Arc::new(AtomicUsize::new(0));
            let calls = Arc::new(AtomicUsize::new(0));
            let probe = Probe {
                drops: Arc::clone(&drops),
                calls: Arc::clone(&calls),
                fail_with: None,
                panic: false,
            };
            (probe, drops, calls)
        }

        fn step(&self) -> Result<(), Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic, "the probe panicked on purpose");
            match self.fail_with {
                Some(kind) => Err(Error::new(kind, "probe failed")),
                None => Ok(()),
            }
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Script for Probe {
        fn execute(&self, span: &Span, code: &str) -> crate::Result<()> {
            assert_eq!(span.begin_file(), "f.lp");
            assert_eq!(code, "x = 1");
            self.step()
        }
        fn callable(&self, name: &str) -> crate::Result<bool> {
            self.step()?;
            Ok(name == "f")
        }
        fn call(&self, _: &Span, _: &str, arguments: &[Symbol]) -> crate::Result<Vec<Symbol>> {
            self.step()?;
            Ok(arguments.to_vec())
        }
    }

    /// A recording `symbol_callback`: the data is a `Vec<u64>`.
    unsafe extern "C" fn collect(symbols: *const u64, size: usize, data: *mut c_void) -> bool {
        // SAFETY: the tests pass a `Vec<u64>` as data, and the trampoline
        // passes `size` symbols.
        let (out, symbols) = unsafe { (&mut *data.cast::<Vec<u64>>(), raw_slice(symbols, size)) };
        out.extend_from_slice(symbols);
        true
    }

    fn data_of(script: Probe) -> *mut c_void {
        let boxed: Box<dyn Script> = Box::new(script);
        Box::into_raw(Box::new(boxed)).cast()
    }

    /// Reclaims what `data_of` leaked, so Miri sees no leak.
    fn reclaim(data: *mut c_void) {
        // SAFETY: `data` came from `data_of` and is reclaimed once.
        drop(unsafe { Box::from_raw(data.cast::<Box<dyn Script>>()) });
    }

    fn location() -> ffi::clingo_location_t {
        ffi::clingo_location_t {
            begin_file: c"f.lp".as_ptr(),
            end_file: c"f.lp".as_ptr(),
            begin_line: 1,
            end_line: 1,
            begin_column: 2,
            end_column: 9,
        }
    }

    fn run_execute(data: *mut c_void) -> bool {
        let loc = location();
        SET.with(|s| s.set(None));
        // SAFETY: `data` is a leaked probe, `loc` and the strings outlive the call.
        unsafe { execute::<Recorded>(&raw const loc, c"x = 1".as_ptr(), data) }
    }

    fn run_callable(data: *mut c_void) -> Option<bool> {
        let mut answer = false;
        SET.with(|s| s.set(None));
        // SAFETY: `data` is a leaked probe; `answer` is a valid out-pointer.
        let ok = unsafe { callable::<Recorded>(c"f".as_ptr(), &raw mut answer, data) };
        ok.then_some(answer)
    }

    #[test]
    fn an_error_is_kept_in_the_frame_and_stops_later_callbacks() {
        let (mut probe, _drops, calls) = Probe::new();
        probe.fail_with = Some(ErrorKind::Logic);
        let data = data_of(probe);
        let frame = ScriptFrame::open();
        assert!(!run_execute(data));
        assert_eq!(SET.with(Cell::get), Some(ffi::clingo_error_unknown));
        assert!(!run_execute(data), "a later callback fails at once");
        assert_eq!(run_callable(data), None);
        let failure = frame.finish();
        assert_eq!(failure.error.unwrap().kind(), ErrorKind::Logic);
        assert!(failure.panic.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "user code ran once");
        reclaim(data);
    }

    #[test]
    fn a_panic_is_caught_and_kept() {
        let (mut probe, _drops, calls) = Probe::new();
        probe.panic = true;
        let data = data_of(probe);
        let frame = ScriptFrame::open();
        assert!(!run_execute(data));
        assert!(!run_execute(data));
        let failure = frame.finish();
        let payload = failure.panic.unwrap();
        assert!(payload.downcast_ref::<&str>().is_some() || payload.is::<String>());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        reclaim(data);
    }

    #[test]
    fn every_script_error_poisons_whatever_its_kind() {
        let (mut probe, _drops, _calls) = Probe::new();
        probe.fail_with = Some(ErrorKind::Callback);
        let data = data_of(probe);
        let frame = ScriptFrame::open();
        assert!(!run_execute(data));
        let always_too = frame.finish().error.unwrap();
        assert_eq!(*always_too.poison(), crate::error::Poison::Always);
        let frame = ScriptFrame::open();
        assert_eq!(run_callable(data), None);
        let always = frame.finish().error.unwrap();
        assert_eq!(*always.poison(), crate::error::Poison::Always);
        reclaim(data);
    }

    #[test]
    fn a_stale_slot_does_not_leak_into_the_next_frame() {
        let (mut probe, _drops, calls) = Probe::new();
        probe.fail_with = Some(ErrorKind::Runtime);
        let data = data_of(probe);
        let first = ScriptFrame::open();
        assert!(!run_execute(data));
        // The call that opened the frame never read its slots (it unwound).
        drop(first);
        let second = ScriptFrame::open();
        assert!(second_is_empty());
        assert!(!run_execute(data), "user code runs again in a new frame");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(second.finish().error.is_some());
        reclaim(data);
    }

    fn second_is_empty() -> bool {
        !failed()
    }

    #[test]
    fn nested_frames_do_not_mix() {
        let (mut probe, _drops, _calls) = Probe::new();
        probe.fail_with = Some(ErrorKind::Runtime);
        let data = data_of(probe);
        let outer = ScriptFrame::open();
        assert!(!run_execute(data));
        let inner = ScriptFrame::open();
        assert!(!failed(), "the inner frame starts empty");
        let quiet = inner.finish();
        assert!(quiet.error.is_none());
        assert!(failed(), "the outer failure is back after the inner frame");
        assert_eq!(outer.finish().error.unwrap().kind(), ErrorKind::Runtime);
        assert!(!failed());
        reclaim(data);
    }

    #[test]
    fn without_a_frame_user_code_runs_and_nothing_is_kept() {
        let (mut probe, _drops, calls) = Probe::new();
        probe.fail_with = Some(ErrorKind::Runtime);
        let data = data_of(probe);
        assert!(!run_execute(data));
        assert!(!run_execute(data));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let frame = ScriptFrame::open();
        assert!(frame.finish().error.is_none());
        reclaim(data);
    }

    #[test]
    fn call_passes_the_pool_in_one_callback_and_nothing_for_none() {
        let (probe, _drops, _calls) = Probe::new();
        let data = data_of(probe);
        let args = [7_u64, 9];
        let loc = location();
        let mut out: Vec<u64> = Vec::new();
        let frame = ScriptFrame::open();
        // SAFETY: as in `run_execute`; `collect` takes `out` as its data.
        let ok = unsafe {
            call_function::<Recorded>(
                &raw const loc,
                c"f".as_ptr(),
                args.as_ptr(),
                args.len(),
                Some(collect),
                std::ptr::from_mut(&mut out).cast(),
                data,
            )
        };
        assert!(ok);
        assert_eq!(out, [7, 9]);
        out.clear();
        // SAFETY: as above, with no arguments (a null pointer is allowed).
        let ok = unsafe {
            call_function::<Recorded>(
                &raw const loc,
                c"f".as_ptr(),
                std::ptr::null(),
                0,
                Some(collect),
                std::ptr::from_mut(&mut out).cast(),
                data,
            )
        };
        assert!(ok && out.is_empty(), "no symbols is no callback");
        assert!(frame.finish().error.is_none());
        reclaim(data);
    }

    #[test]
    fn callable_reports_the_answer() {
        let (probe, _drops, _calls) = Probe::new();
        let data = data_of(probe);
        assert_eq!(run_callable(data), Some(true));
        reclaim(data);
    }

    /// A fake `clingo_register_script` that records what it was given. When it
    /// accepts, it plays the process exit and frees the version string; the
    /// tests free the script through `reclaim`.
    fn fake_register(
        seen: &RefCell<Vec<(String, *mut c_void, String)>>,
        outcome: Result<(), Error>,
    ) -> impl Fn(&CStr, &ffi::clingo_script_t, *mut c_void) -> Result<(), Error> + '_ {
        let outcome = RefCell::new(Some(outcome));
        move |name, script, data| {
            // SAFETY: the registry passes a valid string it leaked.
            let version = unsafe { CStr::from_ptr(script.version) }
                .to_string_lossy()
                .into_owned();
            assert!(script.execute.is_some() && script.call.is_some());
            assert!(script.callable.is_some() && script.main.is_some());
            assert!(script.free.is_none(), "free is never installed");
            seen.borrow_mut()
                .push((name.to_string_lossy().into_owned(), data, version));
            let outcome = outcome.borrow_mut().take().unwrap_or(Ok(()));
            if outcome.is_ok() {
                // SAFETY: `version` came from `CString::into_raw` and nothing
                // else refers to it once the test replaces process exit.
                drop(unsafe { CString::from_raw(script.version.cast_mut()) });
            }
            outcome
        }
    }

    fn fresh() -> Mutex<Registry> {
        Mutex::new(Registry {
            frozen: false,
            names: Vec::new(),
            kept: Vec::new(),
        })
    }

    #[test]
    fn registration_installs_every_member_but_free_and_never_drops_an_accepted_script() {
        let registry = fresh();
        let seen = RefCell::new(Vec::new());
        let (probe, drops, _calls) = Probe::new();
        register_in(
            &registry,
            "foo",
            "1.0",
            Box::new(probe),
            &fake_register(&seen, Ok(())),
        )
        .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let (name, data, version) = seen.borrow()[0].clone();
        assert_eq!((name.as_str(), version.as_str()), ("foo", "1.0"));
        assert_eq!(lock(&registry).names, ["foo"]);
        // The test plays the process exit: it reclaims the script (and the
        // version string) so Miri sees no leak.
        reclaim(data);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_refused_script_is_dropped_before_register_returns() {
        let registry = fresh();
        let seen = RefCell::new(Vec::new());
        let (first, first_drops, _c) = Probe::new();
        register_in(
            &registry,
            "foo",
            "1",
            Box::new(first),
            &fake_register(&seen, Ok(())),
        )
        .unwrap();
        let (again, again_drops, _c) = Probe::new();
        let err = register_in(
            &registry,
            "foo",
            "2",
            Box::new(again),
            &fake_register(&seen, Ok(())),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("foo"));
        assert_eq!(again_drops.load(Ordering::SeqCst), 1);
        assert_eq!(first_drops.load(Ordering::SeqCst), 0);
        assert_eq!(seen.borrow().len(), 1, "clingo saw only the first");
        reclaim(seen.borrow()[0].1);
    }

    #[test]
    fn a_frozen_registry_refuses_and_drops() {
        let registry = fresh();
        lock(&registry).frozen = true;
        let seen = RefCell::new(Vec::new());
        let (probe, drops, _c) = Probe::new();
        let err = register_in(
            &registry,
            "foo",
            "1",
            Box::new(probe),
            &fake_register(&seen, Ok(())),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Control"));
        assert!(err.to_string().contains("before"));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn a_nul_byte_is_refused_first_and_registers_nothing() {
        let registry = fresh();
        lock(&registry).frozen = true;
        let seen = RefCell::new(Vec::new());
        for (name, version) in [("a\0b", "1"), ("a", "1\x0002")] {
            let (probe, drops, _c) = Probe::new();
            let err = register_in(
                &registry,
                name,
                version,
                Box::new(probe),
                &fake_register(&seen, Ok(())),
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Nul, "before the freeze check");
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
        assert_eq!(lock(&registry).names, Vec::<String>::new());
    }

    #[test]
    fn a_failing_clingo_registration_reclaims_and_drops_the_script() {
        let registry = fresh();
        let seen = RefCell::new(Vec::new());
        let (probe, drops, _c) = Probe::new();
        let err = register_in(
            &registry,
            "foo",
            "1",
            Box::new(probe),
            &fake_register(&seen, Err(Error::new(ErrorKind::BadAlloc, "no memory"))),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::BadAlloc);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(lock(&registry).names.is_empty(), "the name stays free");
    }

    #[test]
    fn the_freeze_is_sticky() {
        let registry = fresh();
        assert!(!lock(&registry).frozen);
        lock(&registry).frozen = true;
        assert!(lock(&registry).frozen);
    }
}
