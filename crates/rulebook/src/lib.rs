#![deny(clippy::float_arithmetic)]

use std::cell::{RefCell, UnsafeCell};

use anyhow::Result;
use scoped_tls::scoped_thread_local;
use serde::{de::DeserializeOwned, Serialize};

use rulebook_interface_types::{Output, TaskResult};

pub use {anyhow, serde, serde_json};

pub use rulebook_interface_types::{PlayerId, RoomInfo};

struct Context {
    wait_slot: UnsafeCell<i32>,
    input: Box<[u8]>,
    output: Vec<u8>,
    state: Vec<u8>,
    print_state: bool,
}

scoped_thread_local!(static CONTEXT: RefCell<Context>);

extern "C" {
    #[doc(hidden)]
    pub fn rulebook_trigger_io(
        wait_slot: *const UnsafeCell<i32>,
        input_ptr: *mut u8,
        input_cap: usize,
        output_ptr: *const u8,
        output_len: usize,
        state_ptr: *const u8,
        state_len: usize,
    ) -> usize;

    #[doc(hidden)]
    pub fn rulebook_log(msg_ptr: *const u8, msg_len: usize);
}

fn perform_io_raw<I, O>(out: Output<O>) -> Result<I>
where
    I: DeserializeOwned,
    O: Serialize,
{
    CONTEXT.with(|ctx| {
        let ctx = &mut *ctx.borrow_mut();

        ctx.output.clear();
        serde_json::to_writer(&mut ctx.output, &out)?;

        let input_len = unsafe {
            rulebook_trigger_io(
                &ctx.wait_slot,
                ctx.input.as_mut_ptr(),
                ctx.input.len(),
                ctx.output.as_ptr(),
                ctx.output.len(),
                ctx.state.as_ptr(),
                ctx.state.len(),
            )
        };
        assert!(input_len <= ctx.input.len());

        let input = serde_json::from_slice(&ctx.input[..input_len])?;

        Ok(input)
    })
}

fn report_error<T>(res: Result<T>) -> T {
    match res {
        Ok(v) => v,
        Err(err) => {
            _ = perform_io_raw::<(), ()>(Output::Error(format!("{err:?}")));
            unreachable!(
                "rulebook_trigger_io imported function should not return after error output"
            );
        }
    }
}

fn perform_io<I, O>(out: Output<O>) -> I
where
    I: DeserializeOwned,
    O: Serialize,
{
    report_error(perform_io_raw(out))
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

            $crate::rulebook_trigger_io(
                ptr::null(),
                ptr::null_mut(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            );
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

#[derive(Debug, Default)]
pub struct State<T> {
    inner: T,
}

impl<T: Serialize> State<T> {
    pub fn get(&self) -> &T {
        &self.inner
    }

    pub fn set(&mut self, new_state: T) {
        CONTEXT.with(|ctx| {
            let mut ctx = ctx.borrow_mut();

            if ctx.print_state {
                ctx.state.clear();
                serde_json::to_writer(&mut ctx.state, &new_state)
                    .expect("failed to serialize state");
            }
        });

        self.inner = new_state;
    }
}

pub fn start_session<F, S>(input_cap: usize, print_state: bool, game: F)
where
    F: FnOnce(&RoomInfo, &mut State<S>) -> Result<()>,
    S: Serialize + Default,
{
    let mut state = State::default();

    let ctx = RefCell::new(Context {
        wait_slot: UnsafeCell::new(0),
        input: vec![0; input_cap].into_boxed_slice(),
        output: serde_json::to_vec(&()).unwrap(),
        state: serde_json::to_vec(state.get()).unwrap(),
        print_state,
    });

    CONTEXT.set(&ctx, || {
        let room: RoomInfo = perform_io(Output::SessionStart::<()>);

        report_error(game(&room, &mut state));

        let () = perform_io(Output::SessionEnd::<()>);
    })
}

pub fn log(msg: &str) {
    unsafe { rulebook_log(msg.as_ptr(), msg.len()) }
}

pub fn pause() {
    perform_io(Output::Pause::<()>)
}

pub fn random(start: i32, end: i32) -> i32 {
    perform_io(Output::Random::<()> { start, end })
}

pub fn do_if<F: FnOnce() -> T, T>(targets: Vec<PlayerId>, f: F) -> Option<T> {
    match perform_io(Output::DoTaskIf::<()>(targets)) {
        TaskResult::DoTask => {} // proceed
        TaskResult::SyncResult(()) => {
            report_error(Err::<(), _>(anyhow::anyhow!(
                "unexpected syncResult response"
            )));
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
    T: Serialize + DeserializeOwned + Clone,
{
    match perform_io(Output::DoTaskIf::<()>(vec![])) {
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
    I: DeserializeOwned,
    O: Serialize,
{
    perform_io(Output::Action { from, param })
}
