//! A real Unix-socket Core fixture.
//!
//! The fixture keeps a holder independent of any one connection. A client can
//! close its attached view, reconnect, and see the same shell; resize, status,
//! and exit are deterministic.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use emthin_core_client::controller::{Endpoint, Mode, Session, SessionError};
use emthin_core_client::unix::Connection;
use emthin_core_client::wire::{self, Frame};
use emthin_core_client::{AttachRequest, Client, Event};
use serde_json::{json, Value};

static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(1);

struct Holder {
    running: bool,
    cols: u16,
    rows: u16,
    lines: Vec<String>,
}

impl Holder {
    fn new() -> Self {
        Self {
            running: true,
            cols: 80,
            rows: 24,
            lines: Vec::new(),
        }
    }
}

struct Server {
    path: PathBuf,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(connections: usize) -> Self {
        let path = std::env::temp_dir().join(format!(
            "emthin-core-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind fixture");
        let holder = Arc::new(Mutex::new(Holder::new()));
        let thread = thread::spawn(move || {
            for _ in 0..connections {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                let mut connection = Connection::from_stream(stream).expect("fixture connection");
                serve(&mut connection, &holder);
            }
        });
        Self {
            path,
            thread: Some(thread),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fixture thread");
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(connection: &mut Connection, holder: &Mutex<Holder>) {
    let mut channel = 0;
    while let Ok(Some(frame)) = connection.receive() {
        if frame.channel == 0 {
            let request: Value = serde_json::from_slice(&frame.payload).expect("request json");
            let id = request["id"].as_u64().expect("request id");
            match request["method"].as_str().expect("method") {
                "hello" => send_json(
                    connection,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "generation": 1,
                            "version": "fixture",
                            "core": {"id": "fixture-core", "name": "unix fixture"},
                            "framed": true,
                            "methods": ["hello", "terminal.attach", "events.subscribe"],
                            "features": ["resize-notifications"]
                        }
                    }),
                ),
                "terminal.attach" => {
                    let holder = holder.lock().expect("holder");
                    if !holder.running {
                        send_json(
                            connection,
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {
                                    "code": -32000,
                                    "message": "terminal has finished",
                                    "data": {"kind": "finished"}
                                }
                            }),
                        );
                        continue;
                    }
                    channel = 1;
                    send_json(
                        connection,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "channel": channel,
                                "terminal": {"id": "fixture", "cols": holder.cols, "rows": holder.rows},
                                "resize_notifications": true,
                                "automatic_input": false
                            }
                        }),
                    );
                }
                other => panic!("unexpected control method {other}"),
            }
            continue;
        }

        match frame.kind {
            wire::DATA => {
                let text = String::from_utf8_lossy(&frame.payload);
                for line in text.lines().filter(|line| !line.is_empty()) {
                    match line {
                        "exit" => {
                            {
                                let mut holder = holder.lock().expect("holder");
                                holder.running = false;
                            }
                            let _ = connection.send(Frame {
                                channel,
                                kind: wire::CLOSE,
                                payload: b"exited".to_vec(),
                            });
                            return;
                        }
                        "status" => {
                            let holder = holder.lock().expect("holder");
                            let reply = format!(
                                "state:{};size:{}x{};lines:{}\n",
                                if holder.running {
                                    "running"
                                } else {
                                    "finished"
                                },
                                holder.cols,
                                holder.rows,
                                holder.lines.len()
                            );
                            let _ = connection.send(Frame::data(channel, reply));
                        }
                        other => {
                            {
                                let mut holder = holder.lock().expect("holder");
                                holder.lines.push(other.to_owned());
                            }
                            let _ =
                                connection.send(Frame::data(channel, format!("echo:{other}\n")));
                        }
                    }
                }
            }
            wire::RESIZE => {
                let cols = u16::from_le_bytes([frame.payload[0], frame.payload[1]]);
                let rows = u16::from_le_bytes([frame.payload[2], frame.payload[3]]);
                {
                    let mut holder = holder.lock().expect("holder");
                    holder.cols = cols;
                    holder.rows = rows;
                }
                let _ = connection.send(Frame {
                    channel,
                    kind: wire::RESIZE,
                    payload: [cols.to_le_bytes(), rows.to_le_bytes()].concat(),
                });
            }
            wire::CLOSE => return,
            wire::CREDIT => {}
            kind => panic!("unexpected fixture frame kind {kind}"),
        }
    }
}

fn send_json(connection: &mut Connection, value: Value) {
    connection
        .send(Frame::data(0, serde_json::to_vec(&value).expect("json")))
        .expect("send reply");
}

fn handshake(connection: &mut Connection, client: &mut Client) {
    let (_, request) = client.hello().expect("hello request");
    connection.send(request).expect("send hello");
    let outcome = connection
        .pump(client)
        .expect("hello reply")
        .expect("reply");
    assert!(matches!(outcome.events.as_slice(), [Event::Hello(_)]));
}

fn attach(connection: &mut Connection, client: &mut Client) -> u32 {
    let (_, request) = client
        .attach(AttachRequest::viewer("fixture", 80, 24))
        .expect("attach request");
    connection.send(request).expect("send attach");
    let outcome = connection
        .pump(client)
        .expect("attach reply")
        .expect("reply");
    match outcome.events.as_slice() {
        [Event::Attached { channel, .. }] => *channel,
        other => panic!("expected attach, got {other:?}"),
    }
}

fn send_input(
    connection: &mut Connection,
    client: &mut Client,
    channel: u32,
    text: &str,
) -> Vec<u8> {
    connection
        .send(client.input(channel, text.as_bytes()).expect("input"))
        .expect("send input");
    let outcome = connection.pump(client).expect("data").expect("data");
    match outcome.events.as_slice() {
        [Event::Data { bytes, .. }] => bytes.clone(),
        other => panic!("expected data, got {other:?}"),
    }
}

#[test]
fn a_holder_survives_detach_and_reports_resize_status_and_exit() {
    let server = Server::start(3);

    let mut first = Connection::connect(server.path()).expect("connect first");
    let mut client = Client::new();
    handshake(&mut first, &mut client);
    let channel = attach(&mut first, &mut client);
    assert_eq!(
        send_input(&mut first, &mut client, channel, "one\n"),
        b"echo:one\n"
    );
    first
        .send(client.close(channel).expect("close view"))
        .expect("send close");
    drop(first); // The holder outlived the dropped view: a fresh connection attaches to the
                 // same shell and still sees the line history.
    let endpoint = Endpoint {
        core_id: "fixture-core".into(),
        socket: server.path().to_path_buf(),
    };
    let mut session = Session::new(endpoint.clone(), "fixture");
    let old_lease = session.take_control();
    session
        .reconnect(&endpoint)
        .expect("same endpoint reconnects");
    assert_eq!(session.mode(), Mode::Watching);
    assert_eq!(
        session.input(old_lease, b"stale\n"),
        Err(SessionError::StaleViewer)
    );
    let watch_lease = session.watch();
    assert_eq!(
        session.input(watch_lease, b"stale\n"),
        Err(SessionError::NotControlling)
    );
    let control = session.take_control();

    let mut second = Connection::connect(server.path()).expect("connect second");
    let mut client = Client::new();
    handshake(&mut second, &mut client);
    let channel = attach(&mut second, &mut client);
    assert_eq!(
        send_input(
            &mut second,
            &mut client,
            channel,
            std::str::from_utf8(&session.input(control, b"two\n").expect("new control"))
                .expect("ascii"),
        ),
        b"echo:two\n"
    );
    assert_eq!(
        send_input(&mut second, &mut client, channel, "status\n"),
        b"state:running;size:80x24;lines:2\n"
    );

    second
        .send(client.resize(channel, 100, 40).expect("resize"))
        .expect("send resize");
    let resized = second.pump(&mut client).expect("resize").expect("reply");
    assert!(matches!(
        resized.events.as_slice(),
        [Event::Resize {
            cols: 100,
            rows: 40,
            ..
        }]
    ));
    assert_eq!(
        send_input(&mut second, &mut client, channel, "status\n"),
        b"state:running;size:100x40;lines:2\n"
    );

    second
        .send(client.input(channel, b"exit\n").expect("exit"))
        .expect("send exit");
    let closed = second
        .pump(&mut client)
        .expect("close")
        .expect("close event");
    assert!(matches!(
        closed.events.as_slice(),
        [Event::Closed { reason, .. }] if reason == "exited"
    ));
    drop(second);

    let mut third = Connection::connect(server.path()).expect("connect third");
    let mut client = Client::new();
    handshake(&mut third, &mut client);
    let (_, request) = client
        .attach(AttachRequest::viewer("fixture", 80, 24))
        .expect("attach request");
    third.send(request).expect("send attach");
    let error = third.pump(&mut client).expect_err("attach must fail");
    assert!(
        error.to_string().contains("finished"),
        "unexpected error: {error}"
    );

    drop(third);
    server.join();
}
