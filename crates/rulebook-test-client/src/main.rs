use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::value::RawValue;
use tokio_tungstenite::connect_async;

use rulebook_runtime::{
    channel::Channel, Config, OutputHandler, PlayerId, Runtime, SessionInfo, TaskResult,
};

mod websocket;

#[derive(Debug, Parser)]
struct Args {
    #[arg(short, long)]
    game: PathBuf,
    #[arg(short, long)]
    addr: String,
    #[arg(short, long)]
    player: PlayerId,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let (sender, receiver) = async_channel::unbounded();
    std::thread::spawn(move || {
        use std::io::BufRead;

        for line in std::io::stdin().lock().lines() {
            sender.send_blocking(line.unwrap()).unwrap();
        }
    });

    let runtime = Runtime::new(Config {
        enable_state: true,
        enable_logging: true,
    })?;

    let game_name = args
        .game
        .file_name()
        .with_context(|| format!("filename not exist on {}", args.game.display()))?;
    let game_name = game_name.to_str().with_context(|| {
        format!(
            "filename not a valid unicode string on {}",
            args.game.display()
        )
    })?;
    runtime.add_game(game_name.into(), &std::fs::read(&args.game)?)?;

    // TODO: use url crate
    let addr = format!("{}?color={}", args.addr, args.player);
    let (ws, _resp) = connect_async(addr).await.context("ws connect failed")?;
    anyhow::ensure!(_resp.status().as_u16() < 300, "err resp: {_resp:?}");
    let mut chan = Channel::new(websocket::WebSocketStream::new(ws));

    let session_info: SessionInfo = chan.receive().await?;

    let mut session = runtime.new_session(game_name).await?;
    session
        .start(
            16 * 1024,
            true,
            session_info.room,
            Agent {
                player_id: session_info.player,
                chan,
                receiver,
            },
        )
        .await?;

    Ok(())
}

#[derive(Debug)]
struct Agent {
    player_id: PlayerId,
    chan: Channel<websocket::WebSocketStream>,
    receiver: async_channel::Receiver<String>,
}

#[async_trait::async_trait]
impl OutputHandler for Agent {
    fn state(&mut self, json: &RawValue) -> Result<()> {
        println!("STATE: {json}");
        Ok(())
    }

    async fn do_task_if(&mut self, targets: Vec<PlayerId>) -> Result<TaskResult<Box<RawValue>>> {
        println!("doTaskIf, targets: {targets:?}, me: {}", self.player_id);

        if targets.contains(&self.player_id) {
            Ok(TaskResult::DoTask)
        } else {
            println!("waiting sync msg...");
            let res: TaskResult<Box<RawValue>> = self.chan.receive().await?;
            anyhow::ensure!(!matches!(res, TaskResult::DoTask));
            println!("escaped doTaskIf block");
            Ok(res)
        }
    }

    async fn task_done(&mut self, _targets: Vec<PlayerId>, _value: &RawValue) -> Result<()> {
        println!("waiting sync msg...");
        let res: TaskResult<()> = self.chan.receive().await?;
        anyhow::ensure!(matches!(res, TaskResult::DoTask));
        println!("escaped doTaskIf block");
        Ok(())
    }

    async fn random(&mut self, _start: i32, _end: i32) -> Result<i32> {
        println!("waiting random number");
        Ok(self.chan.receive().await?)
    }

    async fn action(&mut self, from: PlayerId, param: &RawValue) -> Result<Box<RawValue>> {
        if from == self.player_id {
            println!("action requested, param:\n{param}\nINPUT ACTION:");
            let input = RawValue::from_string(self.receiver.recv().await?)?;
            self.chan.send(&input).await?;
            Ok(input)
        } else {
            println!("waiting action from player {from}");
            let msg = self.chan.receive().await?;
            println!("received {msg}");
            Ok(msg)
        }
    }
}
