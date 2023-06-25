use std::collections::hash_map::{Entry, HashMap};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde_json::value::RawValue;
use tokio::sync::Mutex;
use wasmtime::{Caller, Engine, Extern, Func, Instance, Memory, Module, OptLevel, Store};

use rulebook_interface_types::Output;

pub use rulebook_interface_types::{PlayerId, RoomInfo, SessionInfo, TaskResult};

pub mod channel;
pub mod task;

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub enable_state: bool,
    pub enable_logging: bool,
}

pub struct Runtime {
    engine: Engine,
    modules: RwLock<HashMap<Arc<str>, Module>>,
    conf: Config,
}

pub struct Session {
    game_key: Arc<str>,
    store: Store<RoomInfo>,
    module: Module,
    conf: Config,
}

#[async_trait::async_trait]
pub trait OutputHandler: Send + 'static {
    fn state(&mut self, json: &RawValue) -> Result<()>;
    async fn do_task_if(&mut self, allowed: Vec<PlayerId>) -> Result<TaskResult<Box<RawValue>>>;
    async fn task_done(&mut self, targets: Vec<PlayerId>, value: &RawValue) -> Result<()>;
    async fn random(&mut self, start: i32, end: i32) -> Result<i32>;
    async fn action(&mut self, from: PlayerId, param: &RawValue) -> Result<Box<RawValue>>;
}

impl Runtime {
    pub fn new(conf: Config) -> Result<Self> {
        let engine = Engine::new(
            wasmtime::Config::new()
                .async_support(true)
                // .epoch_interruption(true) // TODO: enable to split long running wasm code
                .cranelift_opt_level(OptLevel::Speed)
                .cranelift_nan_canonicalization(true),
        )?;

        Ok(Runtime {
            engine,
            modules: Default::default(),
            conf,
        })
    }

    pub fn add_game(&self, key: Arc<str>, code: &[u8]) -> Result<()> {
        // fail fast on dupe
        if self.modules.read().unwrap().contains_key(&key) {
            anyhow::bail!("game key {key} already exist")
        }

        let module = Module::new(&self.engine, code)?;

        match self.modules.write().unwrap().entry(key.clone()) {
            Entry::Occupied(_) => anyhow::bail!("game key {key} already exist"),
            Entry::Vacant(entry) => {
                entry.insert(module);
            }
        }

        Ok(())
    }

    pub fn remove_game(&self, key: &str) -> bool {
        self.modules.write().unwrap().remove(key).is_some()
    }

    pub async fn new_session(&self, game_key: &str) -> Result<Session> {
        let store = Store::new(&self.engine, RoomInfo::default());
        let (game_key, module) = self
            .modules
            .read()
            .unwrap()
            .get_key_value(game_key)
            .map(|(k, v)| (k.clone(), v.clone()))
            .context("game key not exis")?;

        Ok(Session {
            game_key,
            store,
            module,
            conf: self.conf.clone(),
        })
    }
}

impl Session {
    pub fn game_key(&self) -> &str {
        &self.game_key
    }

    pub async fn start<T>(
        &mut self,
        input_caps: u32,
        print_state: bool,
        room: RoomInfo,
        handler: T,
    ) -> Result<()>
    where
        T: OutputHandler,
    {
        *self.store.data_mut() = room;

        let Config {
            enable_state,
            enable_logging,
        } = self.conf;

        let handler = Arc::new(Mutex::new(handler));
        let func_trigger_io = Func::wrap1_async(
            &mut self.store,
            move |mut caller: Caller<'_, _>, params_ptr: u32| {
                let handler = handler.clone();

                Box::new(async move {
                    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                        anyhow::bail!("wasm memory is not exported under the name `memory`")
                    };
                    let (input_ptr, input_cap, output): (usize, usize, Output<Box<RawValue>>) = {
                        use bytes::Buf;

                        let params_len = 4 * std::mem::size_of::<u32>() as u32;
                        let mut params = slice(&memory, &caller, params_ptr, params_len);

                        let input_ptr = params.get_u32_ne();
                        let input_cap = params.get_u32_ne();
                        let output_ptr = params.get_u32_ne();
                        let output_len = params.get_u32_ne();

                        let output = slice_str(&memory, &caller, output_ptr, output_len)?;
                        println!("got wasm output: {output}");

                        (
                            input_ptr as _,
                            input_cap as _,
                            serde_json::from_str(output)?,
                        )
                    };

                    let json = match output {
                        Output::Error(msg) => anyhow::bail!("game logic error: {msg}"),
                        Output::SessionStart => serde_json::to_string(caller.data())?,
                        Output::SessionEnd => serde_json::to_string(&())?,
                        Output::UpdateState(state) => {
                            if enable_state {
                                handler.lock().await.state(&state)?;
                            }
                            serde_json::to_string(&())?
                        }
                        Output::DoTaskIf { allowed } => {
                            let result = handler.lock().await.do_task_if(allowed).await?;
                            serde_json::to_string(&result)?
                        }
                        Output::TaskDone { targets, value } => {
                            handler.lock().await.task_done(targets, &value).await?;
                            serde_json::to_string(&())?
                        }
                        Output::Random { start, end } => {
                            let result = handler.lock().await.random(start, end).await?;
                            serde_json::to_string(&result)?
                        }
                        Output::Action { from, param } => handler
                            .lock()
                            .await
                            .action(from, &param)
                            .await?
                            .get()
                            .into(),
                    };

                    anyhow::ensure!(json.len() <= input_cap);
                    memory.write(&mut caller, input_ptr, json.as_bytes())?;
                    Ok(json.len() as u32)
                })
            },
        );
        let func_log = Func::wrap(
            &mut self.store,
            move |mut caller: Caller<'_, RoomInfo>, msg_ptr: u32, msg_len: u32| -> Result<()> {
                if !enable_logging {
                    return Ok(());
                };

                let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                    anyhow::bail!("wasm memory is not exported under the name `memory`")
                };
                let msg = slice_str(&memory, &caller, msg_ptr, msg_len)?;

                println!("LOG: {msg}");
                Ok(())
            },
        );

        let instance = Instance::new_async(
            &mut self.store,
            &self.module,
            &[func_trigger_io.into(), func_log.into()],
        )
        .await?;

        instance
            .get_typed_func::<(u32, u32), ()>(&mut self.store, "rulebook_start_session")?
            .call_async(&mut self.store, (input_caps, print_state as u32))
            .await?;

        Ok(())
    }
}

fn slice<'a>(memory: &Memory, caller: &'a Caller<'_, RoomInfo>, ptr: u32, len: u32) -> &'a [u8] {
    &memory.data(caller)[ptr as usize..][..len as usize]
}

fn slice_str<'a>(
    memory: &Memory,
    caller: &'a Caller<'_, RoomInfo>,
    ptr: u32,
    len: u32,
) -> Result<&'a str> {
    std::str::from_utf8(slice(memory, caller, ptr, len)).context("wasm memory slice not a string")
}
