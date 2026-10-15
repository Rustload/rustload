//! `GET /api/events`: Server-Sent Events with live stats, progress, queue changes and logs.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt, stream};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use super::AppState;

pub async fn stream(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let events = BroadcastStream::new(state.engine.events.subscribe()).map(|item| match item {
        Ok(event) => Event::default().event(event.name).json_data(&event.data).ok(),
        // The client fell behind: tell it to reload everything.
        Err(BroadcastStreamRecvError::Lagged(_)) => Some(Event::default().event("resync").data("{}")),
    });
    let logs = BroadcastStream::new(state.logs.subscribe()).map(|item| match item {
        Ok(entry) => Event::default().event("log").json_data(&entry).ok(),
        Err(BroadcastStreamRecvError::Lagged(_)) => None,
    });

    let shutdown = state.engine.shutdown_token().clone();
    let merged = stream::select(events, logs)
        .filter_map(|event| async move { event.map(Ok) })
        // End the stream on shutdown so the server can stop gracefully.
        .take_until(async move { shutdown.cancelled().await });

    Sse::new(merged).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
