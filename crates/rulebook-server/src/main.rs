use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context as _, Result};
use axum::extract::ws::WebSocket;
use clap::Parser;
use futures::stream::{self, StreamExt, TryStreamExt};
use serde_json::value::RawValue;
use tokio::sync::{oneshot, Mutex};

use rulebook_runtime::{
    channel::Channel, OutputHandler, PlayerId, RoomInfo, Runtime, Session, SessionInfo, TaskResult,
};

mod http;
mod websocket;

use websocket::WebSocketStream;

#[derive(Debug, Parser)]
struct Args {
    #[arg(short, long)]
    game: Vec<PathBuf>,
    #[arg(short, long)]
    addr: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    println!("ARGS: {args:?}");

    let server = Arc::new(Server {
        runtime: new_runtime(&args.game)?,
        rooms: Default::default(),
    });

    http::run_server(server, args.addr).await;

    Ok(())
}

struct Server {
    runtime: Runtime,
    rooms: RwLock<HashMap<String, Arc<Mutex<Lobby>>>>,
}

struct Lobby {
    session: Option<Session>,
    connections: Vec<Connection>,
}

struct Connection {
    player_id: PlayerId,
    ws: oneshot::Receiver<WebSocket>,
}

fn new_runtime(games: &[PathBuf]) -> Result<Runtime> {
    let runtime = Runtime::new(Default::default())?;

    for game in games {
        let file = std::fs::read(game)?;

        let name = game
            .file_name()
            .with_context(|| format!("filename not exist on {}", game.display()))?;
        let name = name.to_str().with_context(|| {
            format!("filename not a valid unicode string on {}", game.display())
        })?;
        let name = name.strip_suffix(".wasm").unwrap_or(name);
        println!("game added: {name}");

        runtime.add_game(name.into(), &file)?;
    }

    Ok(runtime)
}

fn new_id() -> String {
    use base64::{engine::general_purpose::URL_SAFE, Engine};

    let bytes: [u8; 12] = rand::random();
    URL_SAFE.encode(bytes)
}

#[derive(Debug)]
struct Room {
    chans: HashMap<PlayerId, Channel<websocket::WebSocketStream>>,
    visibility: Vec<Vec<PlayerId>>,
}

impl Room {
    async fn new(conns: Vec<Connection>) -> Result<Self> {
        let players: Vec<_> = conns.iter().map(|conn| conn.player_id).collect();
        let player_count = players.len();
        let conns: Result<HashMap<_, _>> = stream::iter(conns)
            .map(|conn| async {
                let conn = conn;
                let mut chan = Channel::new(WebSocketStream::new(conn.ws.await?));
                chan.send(&SessionInfo {
                    room: RoomInfo {
                        players: players.clone(),
                    },
                    player: conn.player_id,
                })
                .await?;

                Ok((conn.player_id, chan))
            })
            .buffer_unordered(player_count)
            .try_collect()
            .await;

        Ok(Room {
            chans: conns?,
            visibility: vec![],
        })
    }

    fn scope(&self) -> Vec<PlayerId> {
        self.visibility
            .last()
            .cloned()
            .unwrap_or_else(|| self.chans.keys().cloned().collect())
    }

    fn chan(&mut self, player: PlayerId) -> Result<&mut Channel<WebSocketStream>> {
        self.chans
            .get_mut(&player)
            .context("game tried to grab not existing player channel")
    }
}

#[async_trait::async_trait]
impl OutputHandler for Room {
    fn state(&mut self, _state: &RawValue) -> Result<()> {
        Ok(())
    }

    async fn do_task_if(&mut self, allowed: Vec<PlayerId>) -> Result<TaskResult<Box<RawValue>>> {
        let current_scope = self.scope();
        if allowed.iter().any(|p| !current_scope.contains(p)) {
            anyhow::bail!("game tries to extend visibility");
        }

        self.visibility.push(allowed);

        Ok(TaskResult::DoTask)
    }

    async fn task_done(&mut self, targets: Vec<PlayerId>, value: &RawValue) -> Result<()> {
        let last_frame = self
            .visibility
            .pop()
            .context("game requested taskDone event without previous doTaskIf")?;
        let scope = self.scope();

        for player in scope {
            let chan = self.chan(player)?;

            let res = if last_frame.contains(&player) {
                TaskResult::DoTask
            } else if targets.contains(&player) {
                TaskResult::SyncResult(value)
            } else {
                TaskResult::Restricted
            };
            chan.send(&res).await?;
        }

        Ok(())
    }

    async fn random(&mut self, start: i32, end: i32) -> Result<i32> {
        let value = fastrand::i32(start..=end);
        let scope = self.scope();

        for player in scope {
            self.chan(player)?.send(&value).await?;
        }

        Ok(value)
    }

    async fn action(&mut self, from: PlayerId, _param: &RawValue) -> Result<Box<RawValue>> {
        let value: Box<RawValue> = self.chan(from)?.receive().await?;
        let mut scope = self.scope();
        scope.retain(|&p| p != from);

        for player in scope {
            self.chan(player)?.send(&*value).await?;
        }

        Ok(value)
    }
}
