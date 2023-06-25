use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum Output<T> {
    Error(String),
    SessionStart,
    SessionEnd,
    UpdateState(T),
    DoTaskIf { allowed: Vec<PlayerId> },
    TaskDone { targets: Vec<PlayerId>, value: T },
    Random { start: i32, end: i32 },
    Action { from: PlayerId, param: T },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum TaskResult<T> {
    DoTask,
    SyncResult(T),
    Restricted,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    pub players: Vec<PlayerId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub room: RoomInfo,
    pub player: PlayerId,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::Display,
    strum::EnumIter,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum PlayerId {
    Red,
    Fuchsia,
    Green,
    Lime,
    Yellow,
    Blue,
    Aqua,
    Orange,
}

impl PlayerId {
    pub fn candidates() -> impl Iterator<Item = Self> + ExactSizeIterator + DoubleEndedIterator {
        use strum::IntoEnumIterator;

        Self::iter()
    }
}
