use std::{
    collections::VecDeque,
    error::Error,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::spawn,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use csv::ReaderBuilder;
use smol_str::SmolStr;
use tokio::sync::watch::{self, Receiver, Sender};
use tower_http::{compression::CompressionLayer, services::ServeDir};

mod camera;

#[derive(Clone)]
pub struct AppState {
    points_seen: Arc<Mutex<VecDeque<(SmolStr, (f32, f32))>>>,
    sender: Arc<Sender<(SmolStr, (f32, f32))>>,
}

const POINTS_HISTORY_LIMIT: usize = 80000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let points_seen = Arc::new(Mutex::new(VecDeque::with_capacity(POINTS_HISTORY_LIMIT)));
    let (sender, _receiver) = watch::channel((SmolStr::default(), (f32::NAN, f32::NAN)));
    let sender = Arc::new(sender);

    let state = AppState {
        points_seen: Arc::clone(&points_seen),
        sender: Arc::clone(&sender),
    };

    let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

    let app = Router::new()
        .fallback_service(ServeDir::new(assets_dir))
        .route("/points_history", get(points_history))
        .route("/ws", get(ws_handler))
        .layer(CompressionLayer::new())
        .with_state(state);

    spawn(move || {
        println!("Created background task");

        let stream = TcpStream::connect("127.0.0.1:30003").expect("failed to connect to source");
        let mut reader = ReaderBuilder::new().flexible(true).from_reader(stream);

        for record in reader.records() {
            let record = record.expect("failed to parse source info");

            let lat_long = record
                .get(14)
                .map(str::parse::<f32>)
                .map(Result::ok)
                .flatten()
                .zip(
                    record
                        .get(15)
                        .map(str::parse::<f32>)
                        .map(Result::ok)
                        .flatten(),
                );

            let mode_s = SmolStr::new(record.get(4).unwrap_or_default());

            if let Some((lat, long)) = lat_long {
                let mut points_seen = points_seen.lock().expect("points lock poisoned");

                points_seen.push_back((mode_s.clone(), (lat, long)));

                while points_seen.len() >= POINTS_HISTORY_LIMIT {
                    points_seen.pop_front();
                }

                sender.send_replace((mode_s, (lat, long)));
            }
        }
    });

    axum::Server::bind(&"[::]:12345".parse().unwrap())
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;

    Ok(())
}

async fn points_history(State(state): State<AppState>) -> impl IntoResponse {
    let points = state
        .points_seen
        .lock()
        .expect("lock is poisoned")
        .make_contiguous()
        .to_vec();

    Json::from(points)
}

/// The handler for the HTTP request (this gets called when the HTTP GET lands at the start
/// of websocket negotiation). After this completes, the actual switching from HTTP to
/// websocket protocol will occur.
/// This is the last point where we can extract TCP/IP metadata such as IP address of the client
/// as well as things from HTTP headers such as user-agent of the browser etc.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    println!("{addr} connected.");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| handle_socket(socket, addr, state.sender.subscribe()))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(
    mut socket: WebSocket,
    who: SocketAddr,
    mut receiver: Receiver<(SmolStr, (f32, f32))>,
) {
    loop {
        match receiver.changed().await {
            Ok(()) => {
                let (mode_s, (lat, long)) = receiver.borrow().clone();
                println!("got change");

                match socket
                    .send(Message::Text(format!("[\"{mode_s}\",[{lat},{long}]]")))
                    .await
                {
                    Ok(()) => {
                        println!("update sent to {who}");
                    }
                    Err(e) => {
                        eprintln!("Got error while sending: {e}");

                        break;
                    }
                }
            }
            Err(e) => {
                eprintln!("Got error while checking for updates: {e}");

                break;
            }
        }
    }

    println!("Websocket context {who} destroyed");
}
