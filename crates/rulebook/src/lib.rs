#![deny(clippy::float_arithmetic)]

use std::cell::RefCell;
use std::fmt::Debug;

use anyhow::Result;
use scoped_tls::scoped_thread_local;
use serde::{de::DeserializeOwned, Serialize};

use rulebook_interface_types::{Output, TaskResult};

pub use {anyhow, serde, serde_json};

pub use rulebook_interface_types::{PlayerId, RoomInfo};

struct Context {
    input: Box<[u8]>,
    output: Vec<u8>,
    print_state: bool,
}

#[repr(C)]
#[derive(Debug)]
pub struct IoParams {
    pub input_ptr: *mut u8,
    pub input_cap: usize,
    pub output_ptr: *const u8,
    pub output_len: usize,
}

impl IoParams {
    pub fn new(input: &mut [u8], output: &[u8]) -> Self {
        log!(
            "ioparam, input: {:p}-{}, output: {:p}-{}",
            input.as_ptr(),
            input.len(),
            output.as_ptr(),
            output.len()
        );
        IoParams {
            input_ptr: input.as_mut_ptr(),
            input_cap: input.len(),
            output_ptr: output.as_ptr(),
            output_len: output.len(),
        }
    }
}

scoped_thread_local!(static CONTEXT: RefCell<Context>);

extern "C" {
    #[doc(hidden)]
    pub fn rulebook_trigger_io(params: *const IoParams) -> usize;

    #[doc(hidden)]
    pub fn rulebook_log(msg_ptr: *const u8, msg_len: usize);
}

fn perform_io_raw<I, O>(out: Output<O>) -> Result<I>
where
    I: DeserializeOwned + Debug,
    O: Serialize,
{
    CONTEXT.with(|ctx| {
        let ctx = &mut *ctx.borrow_mut();

        ctx.output.clear();
        serde_json::to_writer(&mut ctx.output, &out)?;

        let input_len = unsafe { rulebook_trigger_io(&IoParams::new(&mut ctx.input, &ctx.output)) };
        assert!(input_len <= ctx.input.len());

        let input = serde_json::from_slice(&ctx.input[..input_len])?;

        Ok(input)
    })
}

fn report_error<T>(f: impl FnOnce() -> Result<T>) -> T {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let err = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => return v,
        Ok(Err(err)) => err,
        Err(err) => {
            if let Some(err) = err.downcast_ref::<String>() {
                anyhow::anyhow!("{err}")
            } else if let Some(err) = err.downcast_ref::<&'static str>() {
                anyhow::anyhow!("{err}")
            } else {
                anyhow::anyhow!("unknown panic msg")
            }
        }
    };
    _ = perform_io_raw::<(), ()>(Output::Error(format!("{err:?}")));
    unreachable!("rulebook_trigger_io imported function should not return after error output");
}

fn perform_io<I, O>(out: Output<O>) -> I
where
    I: DeserializeOwned + Debug,
    O: Serialize,
{
    report_error(|| perform_io_raw(out))
}

#[macro_export]
macro_rules! setup {
    ($game:ident) => {
        #[no_mangle]
        pub extern "C" fn rulebook_start_session(input_cap: usize, print_state: usize) {
            $crate::start_session(input_cap, print_state != 0, $game)
        }

        #[doc(hidden)]
        #[no_mangle]
        pub unsafe extern "C" fn rulebook_dummy_function_to_enforce_linkage() {
            use std::ptr;

            $crate::rulebook_trigger_io(ptr::null());
            $crate::rulebook_log(ptr::null(), 0);
        }
    };
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => {
        $crate::log(&format!($($t)*))
    };
}

#[derive(Debug)]
pub struct Store<T> {
    state: T,
}

impl<T: Serialize> Store<T> {
    pub fn get(&self) -> &T {
        &self.state
    }

    pub fn mutate(&mut self, f: impl FnOnce(&mut T)) {
        f(&mut self.state);

        CONTEXT.with(|ctx| {
            let print_state = ctx.borrow().print_state;

            if print_state {
                let () = perform_io(Output::UpdateState(&self.state));
            }
        });
    }

    pub fn set(&mut self, new_state: T) {
        self.mutate(|inner| *inner = new_state)
    }
}

pub trait State: Serialize {
    fn from_room_info(room_info: &RoomInfo) -> Self;
}

pub fn start_session<F, S>(input_cap: usize, print_state: bool, game: F)
where
    F: FnOnce(&RoomInfo, &mut Store<S>) -> Result<()>,
    S: State,
{
    let ctx = RefCell::new(Context {
        input: vec![0; input_cap].into_boxed_slice(),
        output: serde_json::to_vec(&()).unwrap(),
        print_state,
    });

    CONTEXT.set(&ctx, || {
        let room: RoomInfo = perform_io(Output::SessionStart::<()>);
        let mut store = Store {
            state: S::from_room_info(&room),
        };
        let () = perform_io(Output::UpdateState(store.get()));

        report_error(|| game(&room, &mut store));

        let () = perform_io(Output::SessionEnd::<()>);
    })
}

pub fn log(msg: &str) {
    unsafe { rulebook_log(msg.as_ptr(), msg.len()) }
}

pub fn random(start: i32, end: i32) -> i32 {
    assert!(start <= end, "start > end");
    perform_io(Output::Random::<()> { start, end })
}

pub fn do_if<F: FnOnce() -> T, T>(targets: Vec<PlayerId>, f: F) -> Option<T> {
    match perform_io(Output::DoTaskIf::<()> { allowed: targets }) {
        TaskResult::DoTask => {} // proceed
        TaskResult::SyncResult(()) => {
            report_error(|| Err::<(), _>(anyhow::anyhow!("unexpected syncResult response")));
            unreachable!();
        }
        TaskResult::Restricted => return None,
    }

    let res = f();
    let () = perform_io(Output::TaskDone {
        targets: vec![],
        value: (),
    });

    Some(res)
}

pub fn do_if_admin<F: FnOnce() -> T, T>(f: F) -> Option<T> {
    do_if(vec![], f)
}

pub fn sync_admin_if<F, T>(targets: Vec<PlayerId>, f: F) -> Option<T>
where
    F: FnOnce() -> T,
    T: Serialize + DeserializeOwned + Clone + Debug,
{
    match perform_io(Output::DoTaskIf::<()> { allowed: vec![] }) {
        TaskResult::DoTask => {} // proceed
        TaskResult::SyncResult(v) => return Some(v),
        TaskResult::Restricted => return None,
    }

    let res = f();
    let () = perform_io(Output::TaskDone {
        targets,
        value: res.clone(),
    });

    Some(res)
}

pub fn action<I, O>(from: PlayerId, param: O) -> I
where
    I: DeserializeOwned + Debug,
    O: Serialize,
{
    perform_io(Output::Action { from, param })
}
