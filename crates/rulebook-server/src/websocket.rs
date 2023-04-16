use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use futures::{ready, sink::Sink, stream::Stream};

#[derive(Debug)]
pub struct WebSocketStream {
    ws: WebSocket,
}

impl WebSocketStream {
    pub fn new(ws: WebSocket) -> Self {
        WebSocketStream { ws }
    }
}

impl Stream for WebSocketStream {
    type Item = Result<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match ready!(Pin::new(&mut self.ws).poll_next(cx)?) {
            Some(Message::Text(msg)) => Poll::Ready(Some(Ok(msg))),
            Some(_) => Poll::Pending,
            None => Poll::Ready(None),
        }
    }
}

impl Sink<String> for WebSocketStream {
    type Error = anyhow::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.ws).poll_ready(cx).map_err(Into::into)
    }

    fn start_send(mut self: Pin<&mut Self>, item: String) -> Result<()> {
        Pin::new(&mut self.ws)
            .start_send(Message::Text(item))
            .map_err(Into::into)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.ws).poll_flush(cx).map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.ws).poll_close(cx).map_err(Into::into)
    }
}
