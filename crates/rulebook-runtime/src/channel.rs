use anyhow::Result;
use futures::sink::{Sink, SinkExt};
use futures::stream::{Stream, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Frame<T> {
    Msg { id: u64, val: T },
    Ack { id: u64 },
}

#[derive(Debug)]
pub struct Channel<T> {
    inner: T,
    next_id: u64,
    received: Option<(u64, Box<RawValue>)>,
}

impl<T> Channel<T>
where
    T: Stream<Item = Result<String>> + Sink<String, Error = anyhow::Error> + Unpin,
{
    pub fn new(inner: T) -> Self {
        Channel {
            inner,
            next_id: 0,
            received: None,
        }
    }

    pub async fn send<M: Serialize + ?Sized>(&mut self, val: &M) -> Result<()> {
        let current_id = self.next_id;
        self.next_id += 1;

        let req = serde_json::to_string(&Frame::Msg {
            id: current_id,
            val,
        })?;
        self.inner.send(req).await?;

        while let Some(received) = self.inner.next().await {
            let received: Frame<Box<RawValue>> = serde_json::from_str(&received?)?;

            match received {
                Frame::Ack { id } => {
                    if id == current_id {
                        return Ok(());
                    }
                }
                Frame::Msg { id, val } => self.received = Some((id, val)),
            }
        }

        anyhow::bail!("connection closed before send complete")
    }

    pub async fn receive<M: DeserializeOwned>(&mut self) -> Result<M> {
        if let Some((id, val)) = self.received.take() {
            let msg = serde_json::from_str(val.get())?;
            let ack = serde_json::to_string(&Frame::Ack::<()> { id })?;
            self.inner.send(ack).await?;
            return Ok(msg);
        }

        while let Some(received) = self.inner.next().await {
            let received: Frame<_> = serde_json::from_str(&received?)?;

            if let Frame::Msg { id, val } = received {
                let ack = serde_json::to_string(&Frame::Ack::<()> { id })?;
                self.inner.send(ack).await?;
                return Ok(val);
            }
        }

        anyhow::bail!("connection closed before receive complete")
    }
}
