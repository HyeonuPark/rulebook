use std::cmp::{self, Ord};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use rulebook::{action, do_if_admin, random, sync_admin_if, PlayerId, RoomInfo, Store};

rulebook::setup!(run);

fn run(room: &RoomInfo, store: &mut Store<State>) -> Result<()> {
    let target = do_if_admin(|| random(1, 99));

    loop {
        let turn_player = store.get().turns[0].player;
        let guess: i32 = action(turn_player, "Guess");
        store.mutate(|s| {
            s.turns[0].guess = Some(guess);
            s.turns[0].result = None;
        });

        let result: Ordering = sync_admin_if(room.players.clone(), || {
            Ord::cmp(&target.unwrap(), &guess).into()
        })
        .context("sync all result not received")?;

        match result {
            Ordering::Equal => {
                store.mutate(|s| {
                    s.turns[0].result = Some(result);
                    s.winner = Some(turn_player);
                });
                return Ok(());
            }
            _ => store.mutate(|s| {
                s.turns[0].result = Some(result);
                s.turns.rotate_left(1);
            }),
        }
    }
}

#[derive(Default, Serialize)]
#[serde(tag = "type")]
struct State {
    turns: Vec<Turn>,
    winner: Option<PlayerId>,
}

impl rulebook::State for State {
    fn from_room_info(room_info: &RoomInfo) -> Self {
        State {
            turns: room_info
                .players
                .iter()
                .map(|&player| Turn {
                    player,
                    guess: None,
                    result: None,
                })
                .collect(),
            winner: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct Turn {
    player: PlayerId,
    guess: Option<i32>,
    result: Option<Ordering>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum Ordering {
    Less,
    Equal,
    Greater,
}

impl From<cmp::Ordering> for Ordering {
    fn from(value: cmp::Ordering) -> Self {
        match value {
            cmp::Ordering::Less => Ordering::Less,
            cmp::Ordering::Equal => Ordering::Equal,
            cmp::Ordering::Greater => Ordering::Greater,
        }
    }
}
