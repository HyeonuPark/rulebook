use std::cmp::{self, Ord};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use rulebook::{action, do_if_admin, log, pause, random, sync_admin_if, PlayerId, RoomInfo};

rulebook::setup!(run);

fn run(room: &RoomInfo, state: &mut rulebook::State<State>) -> Result<()> {
    let target = do_if_admin(|| random(1, 99));

    for &turn in room.players.iter().cycle() {
        state.set(State::TurnStart { turn });

        let guess: i32 = action(turn, Action::Guess);
        state.set(State::Guessing { turn, guess });

        let result: Ordering = sync_admin_if(room.players.clone(), || {
            Ord::cmp(&target.unwrap(), &guess).into()
        })
        .context("sync all result not received")?;

        match result {
            Ordering::Equal => {
                state.set(State::Done { winner: turn });
                return Ok(());
            }
            _ => {
                state.set(State::Failed { turn, result });
                log!("player {turn} guessed {guess} but failed - {result:?}");
                pause();
            }
        }
    }

    Ok(())
}

#[derive(Default, Serialize)]
enum State {
    #[default]
    Init,
    TurnStart {
        turn: PlayerId,
    },
    Guessing {
        turn: PlayerId,
        guess: i32,
    },
    Failed {
        turn: PlayerId,
        result: Ordering,
    },
    Done {
        winner: PlayerId,
    },
}

#[derive(Serialize)]
enum Action {
    Guess,
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
