use std::collections::hash_map::Entry;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};

use rulebook_runtime::{PlayerId, RoomInfo};

use crate::{new_id, Connection, Lobby, Room, Server};

pub(crate) async fn run_server(server: Arc<Server>, addr: SocketAddr) {
    let app = Router::new()
        .route(
            "/room",
            post(
                |State(server): State<Arc<Server>>, Json(req): Json<CreateRoomRequest>| async move {
                    println!("/room, req: {req:?}");
                    let room_id = new_id();
                    let session = match server.runtime.new_session(&req.game).await {
                        Ok(s) => s,
                        Err(err) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("failed to create session: {err}"),
                            )
                                .into_response()
                        }
                    };

                    match server.rooms.write().unwrap().entry(room_id.clone()) {
                        Entry::Occupied(_) => {
                            return (StatusCode::INTERNAL_SERVER_ERROR, "UnluckyError")
                                .into_response()
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(Arc::new(Mutex::new(Lobby {
                                session: Some(session),
                                connections: Vec::new(),
                            })));
                        }
                    }

                    Json(CreateRoomResponse { room: room_id }).into_response()
                },
            ),
        )
        .route(
            "/room/:room_id/connect",
            get(
                |State(server): State<Arc<Server>>,
                 Path(room_id): Path<String>,
                 Query(query): Query<ConnectQuery>,
                 ws_conn: WebSocketUpgrade| async move {
                    println!("/room/{room_id}/connect, q: {query:?}");
                    let Some(room) = server.rooms.read().unwrap().get(&room_id).cloned() else {
                        return (StatusCode::NOT_FOUND, "room not found").into_response();
                    };
                    let mut room = room.lock().await;

                    if room.session.is_none() {
                        return (StatusCode::NOT_FOUND, "room not found").into_response();
                    }
                    if room.connections.len() == PlayerId::candidates().len() {
                        println!("room full");
                        return (StatusCode::CONFLICT, "room is full").into_response();
                    }
                    let colors: Vec<_> = room.connections.iter().map(|c| c.player_id).collect();
                    if colors.contains(&query.color) {
                        println!("color dupe, current: {colors:?}");
                        return (StatusCode::CONFLICT, "requested color already taken")
                            .into_response();
                    }

                    let (sender, receiver) = oneshot::channel();
                    room.connections.push(Connection {
                        player_id: query.color,
                        ws: receiver,
                    });

                    ws_conn.on_upgrade(|sock| async {
                        if let Err(err) = sender.send(sock) {
                            println!("sock send failed: {err:?}")
                        }
                    })
                },
            ),
        )
        .route(
            "/room/:room_id/start",
            post(
                |State(server): State<Arc<Server>>, Path(room_id): Path<String>| async move {
                    let Some(room) = server.rooms.write().unwrap().remove(&room_id) else {
                    return (StatusCode::NOT_FOUND, "room not found").into_response();
                };
                    let mut room = room.lock().await;

                    let Some(mut session) = room.session.take() else {
                    return (StatusCode::NOT_FOUND, "room not found").into_response();
                };

                    let players = room.connections.iter().map(|conn| conn.player_id).collect();
                    let conns = std::mem::take(&mut room.connections);

                    tokio::spawn(async move {
                        let room = match Room::new(conns).await {
                            Ok(r) => r,
                            Err(err) => {
                                println!("room init err: {err:?}");
                                return;
                            }
                        };
                        let res = session
                            .start(16384, false, RoomInfo { players }, room)
                            .await;
                        if let Err(err) = res {
                            println!("session run err: {err:?}");
                        }
                    });

                    Json(StartRoomResponse { ok: true }).into_response()
                },
            ),
        )
        .with_state(server);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateRoomRequest {
    game: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateRoomResponse {
    room: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectQuery {
    color: PlayerId,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartRoomResponse {
    ok: bool,
}
