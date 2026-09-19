use std::{
    ffi::{CStr, CString},
    fmt,
    ptr::{self, NonNull},
    rc::Rc,
};

use hegel_c::{
    HEGEL_STATE_MACHINE_DONE, HegelContext, HegelFailure, HegelPool, HegelRun, HegelRunResult,
    HegelSettings, HegelStateMachine, HegelTestCase, hegel_event, hegel_failure_free,
    hegel_failure_origin, hegel_failure_reproduction_blob, hegel_generate_boolean,
    hegel_generate_bytes, hegel_generate_bytes_result_free, hegel_generate_bytes_result_t,
    hegel_generate_integer, hegel_generate_integer_big, hegel_label_from_name, hegel_mark_complete,
    hegel_new_pool, hegel_new_state_machine, hegel_next_test_case, hegel_note, hegel_pool_add,
    hegel_pool_free, hegel_pool_generate, hegel_run_free, hegel_run_result, hegel_run_result_error,
    hegel_run_result_failure, hegel_run_result_failure_count, hegel_run_result_free,
    hegel_run_result_status, hegel_run_start, hegel_run_status_t, hegel_settings_free,
    hegel_settings_new, hegel_settings_new_for_profile, hegel_settings_set_database,
    hegel_settings_set_database_key, hegel_settings_set_phases, hegel_settings_set_print_blob,
    hegel_settings_set_seed, hegel_settings_set_test_cases, hegel_settings_set_verbosity,
    hegel_start_span, hegel_state_machine_free, hegel_state_machine_next_group,
    hegel_state_machine_next_rule, hegel_state_machine_rule_rejected,
    hegel_state_machine_should_check_invariant, hegel_status_t, hegel_stop_span, hegel_target,
    hegel_test_case_free, hegel_test_case_from_blob,
};

pub mod integer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop;

#[derive(Debug)]
pub enum Error {
    Stop(Stop),
    Assume,
    Engine(String),
    Nul(std::ffi::NulError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stop(_) => f.write_str("Hegel exhausted the test-case choice budget"),
            Self::Assume => f.write_str("Hegel rejected the test-case assumption"),
            Self::Engine(message) => f.write_str(message),
            Self::Nul(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::ffi::NulError> for Error {
    fn from(value: std::ffi::NulError) -> Self {
        Self::Nul(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

struct ContextInner(NonNull<HegelContext>);

impl Drop for ContextInner {
    fn drop(&mut self) {
        // SAFETY: this is the sole owner of a pointer returned by hegel_context_new.
        unsafe {
            let _ = hegel_c::hegel_context_free(self.0.as_ptr());
        }
    }
}

#[derive(Clone)]
pub struct Context(Rc<ContextInner>);

impl Context {
    pub fn new() -> Self {
        let raw = hegel_c::hegel_context_new();
        Self(Rc::new(ContextInner(
            NonNull::new(raw).expect("hegel_context_new promises a non-null pointer"),
        )))
    }

    fn raw(&self) -> *mut HegelContext {
        self.0.0.as_ptr()
    }

    fn check(&self, result: hegel_c::hegel_result_t) -> Result<()> {
        use hegel_c::hegel_result_t::*;
        match result {
            HEGEL_OK => Ok(()),
            HEGEL_E_STOP_TEST => Err(Error::Stop(Stop)),
            HEGEL_E_ASSUME => Err(Error::Assume),
            _ => {
                // SAFETY: the context is alive and owns the returned string.
                let message = unsafe {
                    let ptr = hegel_c::hegel_context_last_error(self.raw());
                    if ptr.is_null() {
                        format!("libhegel error: {result:?}")
                    } else {
                        CStr::from_ptr(ptr).to_string_lossy().into_owned()
                    }
                };
                Err(Error::Engine(message))
            }
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Settings {
    ctx: Context,
    raw: NonNull<HegelSettings>,
}

impl Settings {
    pub fn new(ctx: &Context) -> Result<Self> {
        Self::new_inner(ctx, None)
    }

    pub fn for_profile(ctx: &Context, profile: &str) -> Result<Self> {
        Self::new_inner(ctx, Some(profile))
    }

    fn new_inner(ctx: &Context, profile: Option<&str>) -> Result<Self> {
        let mut raw = ptr::null_mut();
        let rc = if let Some(profile) = profile {
            let profile = CString::new(profile)?;
            // SAFETY: all pointers remain valid for the duration of the call.
            unsafe { hegel_settings_new_for_profile(ctx.raw(), profile.as_ptr(), &mut raw) }
        } else {
            // SAFETY: out pointer is valid.
            unsafe { hegel_settings_new(ctx.raw(), &mut raw) }
        };
        ctx.check(rc)?;
        Ok(Self {
            ctx: ctx.clone(),
            raw: NonNull::new(raw)
                .ok_or_else(|| Error::Engine("libhegel returned a null settings handle".into()))?,
        })
    }

    fn raw(&self) -> *mut HegelSettings {
        self.raw.as_ptr()
    }

    pub fn set_test_cases(&mut self, count: u64) -> Result<&mut Self> {
        let rc = unsafe { hegel_settings_set_test_cases(self.ctx.raw(), self.raw(), count) };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_seed(&mut self, seed: Option<u64>) -> Result<&mut Self> {
        let rc = unsafe {
            hegel_settings_set_seed(
                self.ctx.raw(),
                self.raw(),
                seed.unwrap_or(0),
                seed.is_some(),
            )
        };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_database(&mut self, path: Option<&str>) -> Result<&mut Self> {
        let path = path.map(CString::new).transpose()?;
        let rc = unsafe {
            hegel_settings_set_database(
                self.ctx.raw(),
                self.raw(),
                path.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
            )
        };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_database_key(&mut self, key: &str) -> Result<&mut Self> {
        let key = CString::new(key)?;
        let rc =
            unsafe { hegel_settings_set_database_key(self.ctx.raw(), self.raw(), key.as_ptr()) };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_phases(&mut self, phases: u32) -> Result<&mut Self> {
        let rc = unsafe { hegel_settings_set_phases(self.ctx.raw(), self.raw(), phases) };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_verbosity(&mut self, verbosity: u32) -> Result<&mut Self> {
        let rc = unsafe { hegel_settings_set_verbosity(self.ctx.raw(), self.raw(), verbosity) };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn set_print_blob(&mut self, yes: bool) -> Result<&mut Self> {
        let rc = unsafe { hegel_settings_set_print_blob(self.ctx.raw(), self.raw(), yes) };
        self.ctx.check(rc)?;
        Ok(self)
    }

    pub fn start_run(&self) -> Result<Run> {
        let mut raw = ptr::null_mut();
        let rc =
            unsafe { hegel_run_start(self.ctx.raw(), self.raw(), None, ptr::null_mut(), &mut raw) };
        self.ctx.check(rc)?;
        Ok(Run {
            ctx: self.ctx.clone(),
            raw: NonNull::new(raw)
                .ok_or_else(|| Error::Engine("libhegel returned a null run handle".into()))?,
        })
    }

    pub fn test_case_from_blob(&self, blob: &str) -> Result<TestCase> {
        let blob = CString::new(blob)?;
        let mut raw = ptr::null_mut();
        let rc = unsafe {
            hegel_test_case_from_blob(
                self.ctx.raw(),
                self.raw(),
                blob.as_ptr(),
                None,
                ptr::null_mut(),
                &mut raw,
            )
        };
        self.ctx.check(rc)?;
        TestCase::from_raw(&self.ctx, raw)
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_settings_free(self.ctx.raw(), self.raw());
        }
    }
}

pub struct Run {
    ctx: Context,
    raw: NonNull<HegelRun>,
}

impl Run {
    pub fn next_test_case(&mut self) -> Result<Option<TestCase>> {
        let mut raw = ptr::null_mut();
        let rc = unsafe { hegel_next_test_case(self.ctx.raw(), self.raw.as_ptr(), &mut raw) };
        self.ctx.check(rc)?;
        if raw.is_null() {
            Ok(None)
        } else {
            Ok(Some(TestCase::from_raw(&self.ctx, raw)?))
        }
    }

    pub fn result(&self) -> Result<RunResult> {
        let mut raw = ptr::null_mut();
        let rc = unsafe { hegel_run_result(self.ctx.raw(), self.raw.as_ptr(), &mut raw) };
        self.ctx.check(rc)?;
        Ok(RunResult {
            ctx: self.ctx.clone(),
            raw: NonNull::new(raw).ok_or_else(|| {
                Error::Engine("libhegel returned a null run-result handle".into())
            })?,
        })
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_run_free(self.ctx.raw(), self.raw.as_ptr());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Valid,
    Invalid,
    Overrun,
    Interesting,
}

impl Status {
    fn ffi(self) -> u32 {
        match self {
            Self::Valid => hegel_status_t::HEGEL_STATUS_VALID as u32,
            Self::Invalid => hegel_status_t::HEGEL_STATUS_INVALID as u32,
            Self::Overrun => hegel_status_t::HEGEL_STATUS_OVERRUN as u32,
            Self::Interesting => hegel_status_t::HEGEL_STATUS_INTERESTING as u32,
        }
    }
}

pub struct TestCase {
    ctx: Context,
    raw: NonNull<HegelTestCase>,
    complete: bool,
}

impl TestCase {
    fn from_raw(ctx: &Context, raw: *mut HegelTestCase) -> Result<Self> {
        Ok(Self {
            ctx: ctx.clone(),
            raw: NonNull::new(raw)
                .ok_or_else(|| Error::Engine("libhegel returned a null test-case handle".into()))?,
            complete: false,
        })
    }

    fn raw(&self) -> *mut HegelTestCase {
        self.raw.as_ptr()
    }

    pub fn integer_big(&self, min: &[u8], max: &[u8]) -> Result<Vec<u8>> {
        let cap = min.len().max(max.len());
        let mut out = vec![0; cap];
        let mut len = 0;
        let rc = unsafe {
            hegel_generate_integer_big(
                self.ctx.raw(),
                self.raw(),
                min.as_ptr(),
                min.len(),
                max.as_ptr(),
                max.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut len,
            )
        };
        self.ctx.check(rc)?;
        // libhegel writes the value's minimal encoding and sign-fills the rest of
        // the buffer, so the whole buffer reads back as the drawn value. Keeping
        // all `cap` bytes (rather than truncating to `len`) is what makes the
        // fixed-width 33-byte decoders correct for negative values.
        let _ = len;
        Ok(out)
    }

    pub fn integer(&self, min: i64, max: i64) -> Result<i64> {
        let mut out = 0;
        let rc = unsafe { hegel_generate_integer(self.ctx.raw(), self.raw(), min, max, &mut out) };
        self.ctx.check(rc)?;
        Ok(out)
    }

    pub fn boolean(&self, probability: f64) -> Result<bool> {
        let mut out = false;
        let rc = unsafe {
            hegel_generate_boolean(
                self.ctx.raw(),
                self.raw(),
                probability,
                false,
                false,
                &mut out,
            )
        };
        self.ctx.check(rc)?;
        Ok(out)
    }

    pub fn bytes(&self, min: u64, max: u64) -> Result<Vec<u8>> {
        let mut generated = hegel_generate_bytes_result_t {
            data: ptr::null_mut(),
            len: 0,
        };
        let rc =
            unsafe { hegel_generate_bytes(self.ctx.raw(), self.raw(), min, max, &mut generated) };
        self.ctx.check(rc)?;
        let bytes = unsafe { std::slice::from_raw_parts(generated.data, generated.len).to_vec() };
        unsafe {
            let _ = hegel_generate_bytes_result_free(self.ctx.raw(), &mut generated);
        }
        Ok(bytes)
    }

    pub fn start_span(&self, label: &str) -> Result<()> {
        let label = CString::new(label)?;
        let mut hash = 0;
        let rc = unsafe { hegel_label_from_name(self.ctx.raw(), label.as_ptr(), &mut hash) };
        self.ctx.check(rc)?;
        let rc = unsafe { hegel_start_span(self.ctx.raw(), self.raw(), hash) };
        self.ctx.check(rc)
    }

    pub fn stop_span(&self, discard: bool) -> Result<()> {
        let rc = unsafe { hegel_stop_span(self.ctx.raw(), self.raw(), discard) };
        self.ctx.check(rc)
    }

    pub fn note(&self, text: &str) -> Result<()> {
        let rc = unsafe { hegel_note(self.ctx.raw(), self.raw(), text.as_ptr(), text.len()) };
        self.ctx.check(rc)
    }

    /// Report a score for the engine to maximise. Scores under distinct labels
    /// form independent objectives, so callers that track unrelated quantities
    /// must not share one.
    pub fn target(&self, label: &str, value: f64) -> Result<()> {
        let label = CString::new(label)?;
        let rc = unsafe { hegel_target(self.ctx.raw(), self.raw(), value, label.as_ptr()) };
        self.ctx.check(rc)
    }

    pub fn event(&self, label: &str) -> Result<()> {
        let label = CString::new(label)?;
        let rc = unsafe { hegel_event(self.ctx.raw(), self.raw(), label.as_ptr()) };
        self.ctx.check(rc)
    }

    pub fn new_pool(&self) -> Result<Pool> {
        let mut raw = ptr::null_mut();
        let rc = unsafe { hegel_new_pool(self.ctx.raw(), self.raw(), &mut raw) };
        self.ctx.check(rc)?;
        Ok(Pool {
            ctx: self.ctx.clone(),
            raw: NonNull::new(raw)
                .ok_or_else(|| Error::Engine("libhegel returned a null pool handle".into()))?,
        })
    }

    pub fn new_state_machine(
        &self,
        rule_names: &[&str],
        rule_groups: &[i64],
        invariant_names: &[&str],
        invariant_always_check: &[bool],
        step_count: i64,
    ) -> Result<StateMachine> {
        if rule_names.len() != rule_groups.len() {
            return Err(Error::Engine(
                "state-machine rule names and groups have different lengths".into(),
            ));
        }
        if invariant_names.len() != invariant_always_check.len() {
            return Err(Error::Engine(
                "state-machine invariant names and flags have different lengths".into(),
            ));
        }
        let rules = rule_names
            .iter()
            .map(|name| CString::new(*name))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rule_ptrs = rules.iter().map(|name| name.as_ptr()).collect::<Vec<_>>();
        let invariants = invariant_names
            .iter()
            .map(|name| CString::new(*name))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let invariant_ptrs = invariants
            .iter()
            .map(|name| name.as_ptr())
            .collect::<Vec<_>>();
        let mut raw = ptr::null_mut();
        let mut concurrency = 0;
        let rc = unsafe {
            hegel_new_state_machine(
                self.ctx.raw(),
                self.raw(),
                rule_ptrs.as_ptr(),
                rule_groups.as_ptr(),
                rule_ptrs.len(),
                invariant_ptrs.as_ptr(),
                invariant_always_check.as_ptr(),
                invariant_ptrs.len(),
                1,
                1,
                step_count,
                &mut raw,
                &mut concurrency,
            )
        };
        self.ctx.check(rc)?;
        Ok(StateMachine {
            ctx: self.ctx.clone(),
            raw: NonNull::new(raw).ok_or_else(|| {
                Error::Engine("libhegel returned a null state-machine handle".into())
            })?,
            concurrency,
        })
    }

    pub fn mark_complete(&mut self, status: Status, origin: Option<&str>) -> Result<()> {
        let origin = origin.map(CString::new).transpose()?;
        let rc = unsafe {
            hegel_mark_complete(
                self.ctx.raw(),
                self.raw(),
                status.ffi(),
                origin.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
            )
        };
        self.ctx.check(rc)?;
        self.complete = true;
        Ok(())
    }
}

impl Drop for TestCase {
    fn drop(&mut self) {
        if !self.complete {
            let _ = self.mark_complete(Status::Valid, None);
        }
        unsafe {
            let _ = hegel_test_case_free(self.ctx.raw(), self.raw());
        }
    }
}

pub struct Pool {
    ctx: Context,
    raw: NonNull<HegelPool>,
}

impl Pool {
    pub fn add(&self, tc: &TestCase) -> Result<i64> {
        let mut id = 0;
        let rc = unsafe { hegel_pool_add(self.ctx.raw(), tc.raw(), self.raw.as_ptr(), &mut id) };
        self.ctx.check(rc)?;
        Ok(id)
    }

    pub fn generate(&self, tc: &TestCase, consume: bool) -> Result<i64> {
        let mut id = 0;
        let rc = unsafe {
            hegel_pool_generate(
                self.ctx.raw(),
                tc.raw(),
                self.raw.as_ptr(),
                consume,
                &mut id,
            )
        };
        self.ctx.check(rc)?;
        Ok(id)
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_pool_free(self.ctx.raw(), self.raw.as_ptr());
        }
    }
}

pub struct StateMachine {
    ctx: Context,
    raw: NonNull<HegelStateMachine>,
    concurrency: i64,
}

impl StateMachine {
    pub fn concurrency(&self) -> i64 {
        self.concurrency
    }

    pub fn next_group(&self, tc: &TestCase) -> Result<Option<i64>> {
        let mut group = HEGEL_STATE_MACHINE_DONE;
        let rc = unsafe {
            hegel_state_machine_next_group(self.ctx.raw(), tc.raw(), self.raw.as_ptr(), &mut group)
        };
        self.ctx.check(rc)?;
        Ok((group != HEGEL_STATE_MACHINE_DONE).then_some(group))
    }

    pub fn next_rule(&self, tc: &TestCase, worker: i64) -> Result<Option<usize>> {
        let mut rule = HEGEL_STATE_MACHINE_DONE;
        let rc = unsafe {
            hegel_state_machine_next_rule(
                self.ctx.raw(),
                tc.raw(),
                self.raw.as_ptr(),
                worker,
                &mut rule,
            )
        };
        self.ctx.check(rc)?;
        if rule == HEGEL_STATE_MACHINE_DONE {
            Ok(None)
        } else {
            usize::try_from(rule)
                .map(Some)
                .map_err(|_| Error::Engine(format!("invalid state-machine rule index {rule}")))
        }
    }

    pub fn rule_rejected(&self, tc: &TestCase, worker: i64) -> Result<()> {
        let rc = unsafe {
            hegel_state_machine_rule_rejected(self.ctx.raw(), tc.raw(), self.raw.as_ptr(), worker)
        };
        self.ctx.check(rc)
    }

    pub fn should_check_invariant(&self, tc: &TestCase, index: usize) -> Result<bool> {
        let mut should_check = false;
        let index = i64::try_from(index)
            .map_err(|_| Error::Engine("invariant index does not fit i64".into()))?;
        let rc = unsafe {
            hegel_state_machine_should_check_invariant(
                self.ctx.raw(),
                tc.raw(),
                self.raw.as_ptr(),
                index,
                &mut should_check,
            )
        };
        self.ctx.check(rc)?;
        Ok(should_check)
    }
}

impl Drop for StateMachine {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_state_machine_free(self.ctx.raw(), self.raw.as_ptr());
        }
    }
}

pub struct RunResult {
    ctx: Context,
    raw: NonNull<HegelRunResult>,
}

impl RunResult {
    pub fn status(&self) -> Result<hegel_run_status_t> {
        let mut status = hegel_run_status_t::HEGEL_RUN_STATUS_PASSED;
        let rc = unsafe { hegel_run_result_status(self.ctx.raw(), self.raw.as_ptr(), &mut status) };
        self.ctx.check(rc)?;
        Ok(status)
    }

    pub fn error(&self) -> Result<Option<String>> {
        let mut raw = ptr::null();
        let rc = unsafe { hegel_run_result_error(self.ctx.raw(), self.raw.as_ptr(), &mut raw) };
        self.ctx.check(rc)?;
        if raw.is_null() {
            Ok(None)
        } else {
            Ok(Some(
                unsafe { CStr::from_ptr(raw) }
                    .to_string_lossy()
                    .into_owned(),
            ))
        }
    }

    pub fn failure_count(&self) -> Result<usize> {
        let mut count = 0;
        let rc = unsafe {
            hegel_run_result_failure_count(self.ctx.raw(), self.raw.as_ptr(), &mut count)
        };
        self.ctx.check(rc)?;
        Ok(count)
    }

    pub fn failure(&self, index: usize) -> Result<Failure> {
        let mut raw = ptr::null_mut();
        let rc =
            unsafe { hegel_run_result_failure(self.ctx.raw(), self.raw.as_ptr(), index, &mut raw) };
        self.ctx.check(rc)?;
        Ok(Failure {
            ctx: self.ctx.clone(),
            raw: NonNull::new(raw)
                .ok_or_else(|| Error::Engine("libhegel returned a null failure handle".into()))?,
        })
    }
}

impl Drop for RunResult {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_run_result_free(self.ctx.raw(), self.raw.as_ptr());
        }
    }
}

pub struct Failure {
    ctx: Context,
    raw: NonNull<HegelFailure>,
}

impl Failure {
    pub fn origin(&self) -> Result<String> {
        self.string_value(hegel_failure_origin)
            .map(|value| value.unwrap_or_default())
    }

    pub fn reproduction_blob(&self) -> Result<Option<String>> {
        self.string_value(hegel_failure_reproduction_blob)
    }

    fn string_value(
        &self,
        getter: unsafe extern "C" fn(
            *mut HegelContext,
            *const HegelFailure,
            *mut *const std::ffi::c_char,
        ) -> hegel_c::hegel_result_t,
    ) -> Result<Option<String>> {
        let mut raw = ptr::null();
        let rc = unsafe { getter(self.ctx.raw(), self.raw.as_ptr(), &mut raw) };
        self.ctx.check(rc)?;
        if raw.is_null() {
            Ok(None)
        } else {
            Ok(Some(
                unsafe { CStr::from_ptr(raw) }
                    .to_string_lossy()
                    .into_owned(),
            ))
        }
    }
}

impl Drop for Failure {
    fn drop(&mut self) {
        unsafe {
            let _ = hegel_failure_free(self.ctx.raw(), self.raw.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_shrinks_a_failing_integer_to_the_boundary() {
        let ctx = Context::new();
        let mut settings = Settings::new(&ctx).unwrap();
        settings
            .set_database(Some(""))
            .unwrap()
            .set_seed(Some(1))
            .unwrap()
            .set_test_cases(50)
            .unwrap();
        let mut run = settings.start_run().unwrap();
        while let Some(mut tc) = run.next_test_case().unwrap() {
            match tc.integer(0, 100) {
                Ok(value) if value > 10 => tc
                    .mark_complete(Status::Interesting, Some("value > 10"))
                    .unwrap(),
                Ok(_) => tc.mark_complete(Status::Valid, None).unwrap(),
                Err(Error::Stop(_)) => tc.mark_complete(Status::Overrun, None).unwrap(),
                Err(error) => panic!("unexpected draw error: {error}"),
            }
        }
        let result = run.result().unwrap();
        assert_eq!(result.failure_count().unwrap(), 1);

        // Replaying the blob is the only way a frontend can observe the shrunk
        // example, so assert the engine reached the smallest failing value
        // rather than merely that it failed somewhere.
        let blob = result
            .failure(0)
            .unwrap()
            .reproduction_blob()
            .unwrap()
            .expect("a failure carries a reproduction blob");
        let tc = settings.test_case_from_blob(&blob).unwrap();
        assert_eq!(tc.integer(0, 100).unwrap(), 11);
    }
}
