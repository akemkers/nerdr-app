//! The connection to herdr-core (`hc`): one socket shared by every terminal and the event
//! feed, with replies matched to requests and reconnect when the core goes away.
//!
//! The connection starts in line mode: one JSON-RPC message per line. `hello` with
//! `framed: true` switches it to framed mode once the reply has been written. From then on
//! every message in both directions is a frame:
//!
//! ```text
//! u32 LE channel | u8 kind | u32 LE length | payload (length bytes)
//! ```
//!
//! Channel 0 carries JSON-RPC (kind 0, one message per frame, no trailing newline).
//! `terminal.attach` and `events.subscribe` each return a stream channel; its frames are
//! described at [`kind`]. Events arrive on theirs as JSON lines and are credited like
//! terminal output.
//!
//! One reader thread owns the read half. It hands DATA to the [`Sink`] of the channel's
//! stream, one call per batch of already buffered frames, then pays for the batch with one
//! CREDIT per channel. When the socket ends (core restart, crash) it reconnects with
//! backoff, attaches every open stream again and resubscribes to events. A stream whose
//! terminal was lost and replaced in place is attached again after its `restarted` event.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use serde_json::{Value, json};

/// The JSON-RPC channel.
pub const CONTROL: u32 = 0;

/// Frame kinds on a stream channel. Learned from `hc attach` traffic (hc 0.1.18); the
/// OpenRPC schema names them but doesn't give their numbers.
#[allow(dead_code)]
pub mod kind {
    /// Core → viewer: terminal bytes (first the paint, then live output), or event lines.
    /// Viewer → core: input bytes, as if typed.
    pub const DATA: u8 = 0;
    /// Viewer → core: u32 LE, the number of DATA payload bytes consumed.
    pub const CREDIT: u8 = 1;
    /// Core → viewer: the stream ended. Payload is JSON:
    /// `{"exit_code":0,"finished_ms":…,"reason":"exited","signal":null}`.
    /// Viewer → core: detach (empty payload); the core answers with an empty CLOSE.
    pub const CLOSE: u8 = 2;
    /// Viewer → core: u16 LE cols, u16 LE rows.
    pub const RESIZE: u8 = 3;
    /// Viewer → core: one byte, 1 = focused, 0 = not.
    pub const FOCUS: u8 = 4;
    /// Not seen yet; named in the schema.
    pub const FACTS: u8 = 5;
    /// Viewer → core: input that isn't a person typing (`automatic_input` in the attach result).
    pub const AUTOMATIC_INPUT: u8 = 6;
}

const MAX_BATCH: usize = 1 << 20;
const BACKOFF_MIN: Duration = Duration::from_millis(50);
const BACKOFF_MAX: Duration = Duration::from_secs(2);
/// Events kept since the last snapshot, to replay to new listeners. Past this many the
/// client subscribes again, which brings a fresh snapshot.
const MAX_SINCE: usize = 1000;

pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Same order as hc's `paths.rs`: `$HC_SOCKET`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`,
/// else beside the state: `$XDG_STATE_HOME/herdr-core/<session>/<session>.sock`.
pub fn socket_path() -> Result<PathBuf> {
    let env = |var| std::env::var_os(var).filter(|v| !v.is_empty());
    if let Some(p) = env("HC_SOCKET") {
        return Ok(p.into());
    }
    let session = std::env::var("HC_SESSION").unwrap_or_default();
    let session = match session.trim() {
        "" => "default",
        s => s,
    };
    let sock = format!("{session}.sock");
    if let Some(run) = env("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(run).join("herdr-core").join(sock));
    }
    let state = match env("XDG_STATE_HOME") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env("HOME").context("HOME is not set")?).join(".local/state"),
    };
    Ok(state.join("herdr-core").join(session).join(sock))
}

/// Receives one terminal stream. Called on the reader thread, in order.
pub trait Sink: Send + Sync + 'static {
    /// The attach succeeded (first time or again after a reconnect); the paint follows.
    /// `result` is the `terminal.attach` result.
    fn attached(&self, result: &Value);
    /// Terminal output, the payloads of every DATA frame in one batch.
    fn data(&self, payloads: &[&[u8]]);
    /// The stream stopped.
    fn closed(&self, why: Closed);
}

// Fields are read through Debug until the UI shows them.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Closed {
    /// The core closed the stream (the program ended, or the terminal was lost), or the
    /// attach failed. The stream comes back only if the terminal is `restarted`.
    Ended(String),
    /// The connection to the core dropped; the stream is attached again once it's back.
    Lost(String),
}

/// What the client tells the app, in order.
#[allow(dead_code)] // Fields are read through Debug until the agent list uses them.
#[derive(Clone, Debug)]
pub enum Notice {
    /// Connected (again). `version` of the core.
    Connected { version: String },
    /// The connection dropped; reconnecting.
    Disconnected(String),
    /// Every terminal right now (on subscribe, and after a `missed` event).
    Snapshot { seq: u64, terminals: Vec<Value> },
    /// One fact change: `{"type":"status","terminal":"t7","seq":…,"data":{…}}`.
    Event(Value),
}

/// A handle to the connection. Cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct Client(Arc<Inner>);

struct Inner {
    state: Mutex<State>,
    /// The write half and the connection generation it belongs to.
    writer: Mutex<Option<(u64, UnixStream)>>,
}

#[derive(Default)]
struct State {
    /// Bumped on every connect; a write meant for an older connection is dropped.
    generation: u64,
    connected: bool,
    next_id: u64,
    next_stream: u64,
    pending: HashMap<u64, Pending>,
    streams: HashMap<u64, StreamEntry>,
    routes: HashMap<u32, Route>,
    listeners: Vec<UnboundedSender<Notice>>,
    /// Bytes of an event line not yet ended by `\n`.
    event_buf: Vec<u8>,
    events_channel: Option<u32>,
    /// The core's version while connected.
    version: Option<String>,
    /// The last snapshot and every event after it, replayed to a new listener.
    snapshot: Option<Notice>,
    since: Vec<Notice>,
}

enum Pending {
    Call(oneshot::Sender<Result<Value>>),
    Attach(u64),
    Subscribe,
}

#[derive(Clone, Copy)]
enum Route {
    Stream(u64),
    Events,
}

struct StreamEntry {
    sink: Arc<dyn Sink>,
    /// `terminal.attach` params, with the latest size.
    params: Value,
    /// The terminal's address from the last attach result (params may hold a name).
    address: Option<String>,
    state: StreamState,
    /// The params of the attach that's out, to tell if the size changed meanwhile.
    sent: Value,
    /// While attaching again for a new size: the channel still showing, closed once
    /// the new one is up. Until then the core keeps a typing viewer, so the terminal
    /// never falls back to its headless size.
    old: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum StreamState {
    /// Not attached; attach when connected.
    Waiting,
    Attaching,
    Attached(u32),
    /// Closed by the core; attach again on `restarted`.
    Ended,
}

/// One open line to the core, as returned by [`dial`].
struct Line {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    version: String,
}

fn dial() -> Result<Line> {
    let path = socket_path()?;
    let stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting to {} (is hc running?)", path.display()))?;
    let mut reader = BufReader::with_capacity(MAX_BATCH, stream.try_clone()?);
    let mut writer = stream;
    let hello = json!({"jsonrpc":"2.0","id":0,"method":"hello",
                       "params":{"generation":1,"framed":true}});
    writer.write_all(format!("{hello}\n").as_bytes())?;
    // The hello reply is the last line-mode message; frames follow it in the same buffer.
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        bail!("core closed the connection during hello");
    }
    let reply: Value = serde_json::from_str(&line).context("hello reply")?;
    if let Some(err) = reply.get("error") {
        bail!("hello: {err}");
    }
    if reply["result"]["framed"] != json!(true) {
        bail!("core did not switch to framed mode: {line}");
    }
    let version = reply["result"]["version"].as_str().unwrap_or("?").to_string();
    eprintln!(
        "nerdr: hc {version} on {}",
        reply["result"]["core"]["name"].as_str().unwrap_or("?")
    );
    Ok(Line {
        reader,
        writer,
        version,
    })
}

impl Client {
    /// Connects, or fails if the core can't be reached now. Later drops are reconnected.
    pub fn connect() -> Result<Self> {
        let line = dial()?;
        let client = Client(Arc::new(Inner {
            state: Mutex::new(State {
                next_id: 1,
                next_stream: 1,
                ..Default::default()
            }),
            writer: Mutex::new(None),
        }));
        let reader = client.start(line);
        let c = client.clone();
        std::thread::Builder::new()
            .name("hc-reader".into())
            .spawn(move || c.run(reader))
            .expect("reader thread");
        Ok(client)
    }

    /// Connection changes and events. Starts with the current state: `Connected` (if it
    /// is), the last snapshot and the events since.
    pub fn notices(&self) -> UnboundedReceiver<Notice> {
        let (tx, rx) = unbounded();
        let mut st = self.0.state.lock().unwrap();
        if let Some(version) = st.version.clone() {
            let _ = tx.unbounded_send(Notice::Connected { version });
        }
        for n in st.snapshot.iter().chain(&st.since) {
            let _ = tx.unbounded_send(n.clone());
        }
        st.listeners.push(tx);
        rx
    }

    /// A JSON-RPC call on channel 0. Fails at once when not connected, and when the
    /// connection drops before the reply.
    pub fn call(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value>> + use<> {
        let (tx, rx) = oneshot::channel();
        let sent = {
            let mut st = self.0.state.lock().unwrap();
            if st.connected {
                let id = st.take_id();
                st.pending.insert(id, Pending::Call(tx));
                Ok((st.generation, request(id, method, &params)))
            } else {
                Err(tx)
            }
        };
        match sent {
            Ok((generation, req)) => self.write(generation, CONTROL, kind::DATA, &req),
            Err(tx) => {
                let _ = tx.send(Err(anyhow!("{method}: not connected to hc")));
            }
        }
        let method = method.to_string();
        async move {
            rx.await
                .map_err(|_| anyhow!("{method}: connection closed"))?
        }
    }

    /// [`Client::call`], blocking the calling thread until the reply. Not on the reader thread.
    pub fn call_blocking(&self, method: &str, params: Value) -> Result<Value> {
        futures::executor::block_on(self.call(method, params))
    }

    /// Attaches a terminal stream. `params` are `terminal.attach` params (`terminal`, `cols`,
    /// `rows`, …). The outcome arrives at the sink; dropping the [`Stream`] detaches.
    pub fn attach(&self, params: Value, sink: Arc<dyn Sink>) -> Stream {
        let (id, send) = {
            let mut st = self.0.state.lock().unwrap();
            let id = st.next_stream;
            st.next_stream += 1;
            st.streams.insert(
                id,
                StreamEntry {
                    sink,
                    params,
                    address: None,
                    state: StreamState::Waiting,
                    sent: Value::Null,
                    old: None,
                },
            );
            let send = st.connected.then(|| st.attach_request(id));
            (id, send)
        };
        if let Some((generation, req)) = send.flatten() {
            self.write(generation, CONTROL, kind::DATA, &req);
        }
        Stream {
            client: self.clone(),
            id,
        }
    }

    fn write(&self, generation: u64, channel: u32, kind: u8, payload: &[u8]) {
        let mut buf = Vec::with_capacity(9 + payload.len());
        buf.extend_from_slice(&channel.to_le_bytes());
        buf.push(kind);
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        let mut w = self.0.writer.lock().unwrap();
        if let Some((g, s)) = w.as_mut()
            && *g == generation
            && s.write_all(&buf).is_err()
        {
            // The reader sees the same error and reconnects; stop writing to this one.
            *w = None;
        }
    }

    /// Sends a frame on a stream's current channel, if it's attached.
    fn stream_frame(&self, id: u64, kind: u8, payload: &[u8]) {
        let target = {
            let st = self.0.state.lock().unwrap();
            match st.streams.get(&id).map(|e| e.state) {
                Some(StreamState::Attached(ch)) => Some((st.generation, ch)),
                _ => None,
            }
        };
        if let Some((generation, ch)) = target {
            self.write(generation, ch, kind, payload);
        }
    }

    fn notify(&self, notice: Notice) {
        let resubscribe = {
            let mut st = self.0.state.lock().unwrap();
            st.listeners
                .retain(|l| l.unbounded_send(notice.clone()).is_ok());
            match &notice {
                Notice::Connected { version } => st.version = Some(version.clone()),
                Notice::Disconnected(_) => st.version = None,
                Notice::Snapshot { .. } => {
                    st.snapshot = Some(notice);
                    st.since.clear();
                }
                Notice::Event(_) => st.since.push(notice),
            }
            st.since.len() > MAX_SINCE && st.connected
        };
        if resubscribe {
            self.subscribe();
        }
    }

    /// (Re)subscribes to every event; the reply brings a fresh snapshot.
    fn subscribe(&self) {
        let (generation, old, req) = {
            let mut st = self.0.state.lock().unwrap();
            st.since.clear();
            let old = st.events_channel.take();
            if let Some(ch) = old {
                st.routes.remove(&ch);
            }
            st.event_buf.clear();
            let id = st.take_id();
            st.pending.insert(id, Pending::Subscribe);
            (st.generation, old, request(id, "events.subscribe", &json!({})))
        };
        if let Some(ch) = old {
            self.write(generation, ch, kind::CLOSE, &[]);
        }
        self.write(generation, CONTROL, kind::DATA, &req);
    }

    /// The reader thread: serve one connection until it ends, then reconnect, forever.
    fn run(self, mut reader: BufReader<UnixStream>) {
        loop {
            let err = self.serve(reader);
            self.lost(&err);
            let line = self.redial();
            reader = self.start(line);
        }
    }

    /// Installs a fresh connection, then subscribes and attaches every open stream.
    fn start(&self, line: Line) -> BufReader<UnixStream> {
        let Line {
            reader,
            writer,
            version,
        } = line;
        let mut out = Vec::new();
        let generation = {
            let mut st = self.0.state.lock().unwrap();
            st.generation += 1;
            st.connected = true;
            st.event_buf.clear();
            st.events_channel = None;
            *self.0.writer.lock().unwrap() = Some((st.generation, writer));
            let waiting: Vec<u64> = st
                .streams
                .iter()
                .filter(|(_, e)| e.state == StreamState::Waiting)
                .map(|(id, _)| *id)
                .collect();
            for sid in waiting {
                if let Some((_, req)) = st.attach_request(sid) {
                    out.push(req);
                }
            }
            st.generation
        };
        self.notify(Notice::Connected { version });
        self.subscribe();
        for req in out {
            self.write(generation, CONTROL, kind::DATA, &req);
        }
        reader
    }

    /// Reads frames until the connection fails.
    fn serve(&self, mut reader: BufReader<UnixStream>) -> anyhow::Error {
        let mut batch: Vec<Frame> = Vec::new();
        loop {
            // One blocking read, then whatever else is already buffered, so a flood of tiny
            // frames is parsed under one lock and paid for with one CREDIT per channel.
            match read_frame(&mut reader) {
                Ok(f) => batch.push(f),
                Err(e) => return e,
            }
            let mut bytes = batch[0].payload.len();
            while bytes < MAX_BATCH {
                match next_buffered(&mut reader) {
                    Some(Ok(f)) => {
                        bytes += f.payload.len();
                        batch.push(f);
                    }
                    Some(Err(e)) => return e,
                    None => break,
                }
            }
            self.dispatch(&batch);
            batch.clear();
        }
    }

    /// Hands out one batch. DATA is collected per channel and delivered before any other
    /// frame, so every sink sees its bytes in order and before its CLOSE.
    fn dispatch(&self, batch: &[Frame]) {
        let mut data: Vec<(u32, Vec<&[u8]>)> = Vec::new();
        for f in batch {
            if f.channel != CONTROL && f.kind == kind::DATA {
                match data.iter_mut().find(|(ch, _)| *ch == f.channel) {
                    Some((_, v)) => v.push(&f.payload),
                    None => data.push((f.channel, vec![&f.payload])),
                }
                continue;
            }
            self.flush_data(&mut data);
            if f.channel == CONTROL {
                self.on_control(&f.payload);
            } else if f.kind == kind::CLOSE {
                self.on_close(f.channel, &f.payload);
            } else {
                eprintln!(
                    "nerdr: channel {} frame kind {} ({} bytes): {:?}",
                    f.channel,
                    f.kind,
                    f.payload.len(),
                    String::from_utf8_lossy(&f.payload)
                );
            }
        }
        self.flush_data(&mut data);
    }

    fn flush_data(&self, data: &mut Vec<(u32, Vec<&[u8]>)>) {
        for (ch, payloads) in data.drain(..) {
            let (generation, route, sink) = {
                let st = self.0.state.lock().unwrap();
                let route = st.routes.get(&ch).copied();
                let sink = match route {
                    Some(Route::Stream(id)) => st.streams.get(&id).map(|e| e.sink.clone()),
                    _ => None,
                };
                (st.generation, route, sink)
            };
            let n: usize = payloads.iter().map(|p| p.len()).sum();
            match route {
                // Detached, or not ours: not paid for
                None => continue,
                Some(Route::Events) => self.on_events(&payloads),
                Some(Route::Stream(_)) => {
                    if let Some(sink) = sink {
                        sink.data(&payloads);
                    }
                }
            }
            self.write(generation, ch, kind::CREDIT, &(n as u32).to_le_bytes());
        }
    }

    fn on_control(&self, payload: &[u8]) {
        let msg: Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("nerdr: bad control message ({e}): {}", String::from_utf8_lossy(payload));
                return;
            }
        };
        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
            eprintln!("nerdr: control: {msg}");
            return;
        };
        let pending = self.0.state.lock().unwrap().pending.remove(&id);
        let result = match msg.get("error") {
            Some(err) => Err(err.clone()),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        match pending {
            None => eprintln!("nerdr: reply to unknown request {id}"),
            Some(Pending::Call(tx)) => {
                let _ = tx.send(result.map_err(|e| anyhow!("{e}")));
            }
            Some(Pending::Subscribe) => match result {
                Ok(r) => {
                    if let Some(ch) = r["channel"].as_u64() {
                        let mut st = self.0.state.lock().unwrap();
                        st.routes.insert(ch as u32, Route::Events);
                        st.events_channel = Some(ch as u32);
                    }
                    self.notify(snapshot(&r));
                }
                Err(e) => eprintln!("nerdr: events.subscribe: {e}"),
            },
            Some(Pending::Attach(sid)) => self.on_attached(sid, result),
        }
    }

    fn on_attached(&self, sid: u64, result: Result<Value, Value>) {
        let mut detach = None;
        let mut close = Vec::new();
        let sink = {
            let mut st = self.0.state.lock().unwrap();
            let generation = st.generation;
            let channel = result.as_ref().ok().and_then(|r| r["channel"].as_u64()).map(|c| c as u32);
            // The size changed while this attach was out: its paint is stale, ask again
            if let (Some(ch), Some(e)) = (channel, st.streams.get(&sid))
                && e.params != e.sent
            {
                let again = st.attach_request(sid);
                drop(st);
                self.write(generation, ch, kind::CLOSE, &[]);
                if let Some((g, req)) = again {
                    self.write(g, CONTROL, kind::DATA, &req);
                }
                return;
            }
            if let Some(old) = st.streams.get_mut(&sid).and_then(|e| e.old.take()) {
                st.routes.remove(&old);
                close.push(old);
            }
            let Some(e) = st.streams.get_mut(&sid) else {
                // Dropped while attaching: let the core go too
                if let Ok(r) = &result
                    && let Some(ch) = r["channel"].as_u64()
                {
                    detach = Some((generation, ch as u32));
                }
                drop(st);
                if let Some((g, ch)) = detach {
                    self.write(g, ch, kind::CLOSE, &[]);
                }
                return;
            };
            match (&result, channel) {
                (Ok(r), Some(ch)) => {
                    e.state = StreamState::Attached(ch);
                    e.address = r["terminal"]["address"].as_str().map(str::to_string);
                    let sink = e.sink.clone();
                    st.routes.insert(ch, Route::Stream(sid));
                    sink
                }
                _ => {
                    e.state = StreamState::Ended;
                    e.sink.clone()
                }
            }
        };
        let generation = self.0.state.lock().unwrap().generation;
        for ch in close {
            self.write(generation, ch, kind::CLOSE, &[]);
        }
        match result {
            Ok(r) if r["channel"].is_u64() => sink.attached(&r),
            Ok(r) => sink.closed(Closed::Ended(format!("attach: no channel in {r}"))),
            Err(e) => sink.closed(Closed::Ended(format!("attach: {e}"))),
        }
    }

    fn on_close(&self, ch: u32, payload: &[u8]) {
        let sink = {
            let mut st = self.0.state.lock().unwrap();
            match st.routes.remove(&ch) {
                Some(Route::Stream(id)) => st.streams.get_mut(&id).map(|e| {
                    e.state = StreamState::Ended;
                    if e.old == Some(ch) {
                        e.old = None;
                    }
                    e.sink.clone()
                }),
                Some(Route::Events) => {
                    eprintln!("nerdr: event stream closed");
                    None
                }
                None => None, // our own detach, acknowledged
            }
        };
        if let Some(sink) = sink {
            sink.closed(Closed::Ended(String::from_utf8_lossy(payload).into_owned()));
        }
    }

    fn on_events(&self, payloads: &[&[u8]]) {
        let lines: Vec<Vec<u8>> = {
            let mut st = self.0.state.lock().unwrap();
            for p in payloads {
                st.event_buf.extend_from_slice(p);
            }
            let Some(end) = st.event_buf.iter().rposition(|&b| b == b'\n') else {
                return;
            };
            let rest = st.event_buf.split_off(end + 1);
            let done = std::mem::replace(&mut st.event_buf, rest);
            done.split(|&b| b == b'\n')
                .filter(|l| !l.is_empty())
                .map(<[u8]>::to_vec)
                .collect()
        };
        for l in lines {
            let msg: Value = match serde_json::from_slice(&l) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("nerdr: bad event ({e}): {}", String::from_utf8_lossy(&l));
                    continue;
                }
            };
            if msg["method"] != "event" {
                continue; // heartbeat
            }
            let ev = msg["params"].clone();
            match ev["type"].as_str() {
                Some("missed") => self.notify(snapshot(&ev["data"])),
                Some("restarted") => {
                    if let Some(t) = ev["terminal"].as_str() {
                        self.reattach_ended(t);
                    }
                    self.notify(Notice::Event(ev));
                }
                _ => self.notify(Notice::Event(ev)),
            }
        }
    }

    /// The terminal's program was replaced in place: attach its ended streams again.
    fn reattach_ended(&self, address: &str) {
        let (generation, reqs) = {
            let mut st = self.0.state.lock().unwrap();
            let ids: Vec<u64> = st
                .streams
                .iter()
                .filter(|(_, e)| e.state == StreamState::Ended && e.address.as_deref() == Some(address))
                .map(|(id, _)| *id)
                .collect();
            let reqs: Vec<Vec<u8>> = ids
                .into_iter()
                .filter_map(|id| st.attach_request(id).map(|(_, r)| r))
                .collect();
            (st.generation, reqs)
        };
        for r in reqs {
            self.write(generation, CONTROL, kind::DATA, &r);
        }
    }

    /// The connection ended: fail what was waiting and mark every stream for reattach.
    fn lost(&self, err: &anyhow::Error) {
        let why = format!("connection to hc lost: {err}");
        eprintln!("nerdr: {why}");
        *self.0.writer.lock().unwrap() = None;
        let (calls, sinks) = {
            let mut st = self.0.state.lock().unwrap();
            st.connected = false;
            st.routes.clear();
            let calls: Vec<_> = st
                .pending
                .drain()
                .filter_map(|(_, p)| match p {
                    Pending::Call(tx) => Some(tx),
                    _ => None,
                })
                .collect();
            let mut sinks = Vec::new();
            for e in st.streams.values_mut() {
                e.old = None;
                if matches!(e.state, StreamState::Attached(_) | StreamState::Attaching) {
                    e.state = StreamState::Waiting;
                    sinks.push(e.sink.clone());
                }
            }
            (calls, sinks)
        };
        for tx in calls {
            let _ = tx.send(Err(anyhow!("{why}")));
        }
        for s in sinks {
            s.closed(Closed::Lost(why.clone()));
        }
        self.notify(Notice::Disconnected(why));
    }

    fn redial(&self) -> Line {
        let mut wait = BACKOFF_MIN;
        loop {
            std::thread::sleep(wait);
            match dial() {
                Ok(line) => return line,
                Err(_) => wait = (wait * 2).min(BACKOFF_MAX),
            }
        }
    }
}

impl State {
    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Marks the stream attaching and builds its request, or None if it's gone.
    fn attach_request(&mut self, sid: u64) -> Option<(u64, Vec<u8>)> {
        let params = {
            let e = self.streams.get_mut(&sid)?;
            e.state = StreamState::Attaching;
            e.sent = e.params.clone();
            e.params.clone()
        };
        let id = self.take_id();
        self.pending.insert(id, Pending::Attach(sid));
        Some((self.generation, request(id, "terminal.attach", &params)))
    }
}

fn request(id: u64, method: &str, params: &Value) -> Vec<u8> {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        .to_string()
        .into_bytes()
}

fn snapshot(r: &Value) -> Notice {
    Notice::Snapshot {
        seq: r["seq"].as_u64().unwrap_or(0),
        terminals: r["terminals"].as_array().cloned().unwrap_or_default(),
    }
}

/// One attached terminal stream. Dropping it detaches.
pub struct Stream {
    client: Client,
    id: u64,
}

impl Stream {
    /// Input bytes, as typed.
    pub fn input(&self, bytes: &[u8]) {
        self.client.stream_frame(self.id, kind::DATA, bytes);
    }

    /// A new size: attaches again, so the sink gets a fresh paint at that size. The
    /// paint breaks wrapped lines with `\r\n`, so the viewer can't wrap the old one
    /// again the way the core does (hc 0.1.22). One attach at a time: an attach that's
    /// out when the size changes is asked again with the latest one.
    pub fn resize(&self, cols: u16, rows: u16) {
        let send = {
            let mut st = self.client.0.state.lock().unwrap();
            let Some(e) = st.streams.get_mut(&self.id) else {
                return;
            };
            e.params["cols"] = cols.into();
            e.params["rows"] = rows.into();
            match e.state {
                StreamState::Attached(ch) => {
                    e.old = Some(ch);
                    st.attach_request(self.id)
                }
                // Attaching sees the change on its reply; the rest use it on their next attach
                _ => None,
            }
        };
        if let Some((generation, req)) = send {
            self.client.write(generation, CONTROL, kind::DATA, &req);
        }
    }

    pub fn focus(&self, focused: bool) {
        self.client
            .stream_frame(self.id, kind::FOCUS, &[focused as u8]);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let detach = {
            let mut st = self.client.0.state.lock().unwrap();
            let e = st.streams.remove(&self.id);
            let attached = match e.as_ref().map(|e| e.state) {
                Some(StreamState::Attached(ch)) => Some(ch),
                _ => None,
            };
            // An attach still out is closed on its reply
            let chs: Vec<u32> = attached.into_iter().chain(e.and_then(|e| e.old)).collect();
            for ch in &chs {
                st.routes.remove(ch);
            }
            (st.generation, chs)
        };
        let (generation, chs) = detach;
        for ch in chs {
            self.client.write(generation, ch, kind::CLOSE, &[]);
        }
    }
}

pub fn read_frame(r: &mut impl Read) -> Result<Frame> {
    let mut hdr = [0u8; 9];
    r.read_exact(&mut hdr)?;
    let channel = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let kind = hdr[4];
    let len = u32::from_le_bytes(hdr[5..9].try_into().unwrap()) as usize;
    let mut payload = vec![0; len];
    r.read_exact(&mut payload)?;
    Ok(Frame {
        channel,
        kind,
        payload,
    })
}

/// The next frame if it is already buffered whole, so callers can batch without blocking.
fn next_buffered(r: &mut BufReader<UnixStream>) -> Option<Result<Frame>> {
    let buf = r.buffer();
    if buf.len() < 9 {
        return None;
    }
    let len = u32::from_le_bytes(buf[5..9].try_into().unwrap()) as usize;
    if buf.len() < 9 + len {
        return None;
    }
    Some(read_frame(r))
}

/// Against a real core in its own session: `cargo test -- --ignored`. Needs `hc` on PATH.
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[derive(Default)]
    struct TestSink {
        out: Mutex<Vec<u8>>,
        attaches: Mutex<u32>,
        closes: Mutex<Vec<String>>,
    }

    impl Sink for TestSink {
        /// Keeps only what came since the latest attach.
        fn attached(&self, _: &Value) {
            self.out.lock().unwrap().clear();
            *self.attaches.lock().unwrap() += 1;
        }
        fn data(&self, payloads: &[&[u8]]) {
            let mut o = self.out.lock().unwrap();
            for p in payloads {
                o.extend_from_slice(p);
            }
        }
        fn closed(&self, why: Closed) {
            self.closes.lock().unwrap().push(format!("{why:?}"));
        }
    }

    impl TestSink {
        fn len(&self) -> usize {
            self.out.lock().unwrap().len()
        }
        fn contains(&self, s: &str) -> bool {
            String::from_utf8_lossy(&self.out.lock().unwrap()).contains(s)
        }
    }

    fn until(what: &str, secs: u64, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn hc(args: &[&str]) {
        let st = std::process::Command::new("hc")
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("hc");
        assert!(st.success(), "hc {args:?}");
    }

    fn run(c: &Client, script: &str) -> String {
        let r = c
            .call_blocking(
                "terminal.run",
                json!({"argv": ["sh", "-c", script], "no_parent": true, "no_notify": true,
                       "size": {"cols": 80, "rows": 24}}),
            )
            .unwrap();
        r["address"].as_str().unwrap().to_string()
    }

    fn attach(c: &Client, t: &str) -> (Stream, Arc<TestSink>) {
        let sink = Arc::new(TestSink::default());
        let s = c.attach(json!({"terminal": t, "cols": 80, "rows": 24}), sink.clone());
        (s, sink)
    }

    #[test]
    #[ignore = "starts its own hc core"]
    fn connection_layer() {
        let session = format!("nt{}", std::process::id());
        // SAFETY: the only test in this binary that touches the environment.
        unsafe { std::env::set_var("HC_SESSION", &session) };
        hc(&["server", "start"]);
        let c = Client::connect().unwrap();
        let mut notices = c.notices();
        let next_notice = |n: &mut UnboundedReceiver<Notice>| {
            futures::executor::block_on(async {
                futures::StreamExt::next(n).await.expect("notice")
            })
        };
        assert!(matches!(next_notice(&mut notices), Notice::Connected { .. }));
        while !matches!(next_notice(&mut notices), Notice::Snapshot { .. }) {}

        // Two terminals on one connection, each to its own sink
        let a = run(&c, "echo one-a; while read x; do echo got-$x; done");
        let b = run(&c, "echo two-b; sleep 600");
        let (sa, ka) = attach(&c, &a);
        let (_sb, kb) = attach(&c, &b);
        until("both paints", 5, || ka.contains("one-a") && kb.contains("two-b"));
        assert!(!ka.contains("two-b") && !kb.contains("one-a"));
        sa.input(b"hi\r");
        until("input echoed", 5, || ka.contains("got-hi"));

        // Events: created and finished for a short program
        let short = run(&c, "exit 7");
        let mut seen = Vec::new();
        while !seen.contains(&"finished".to_string()) {
            if let Notice::Event(e) = next_notice(&mut notices)
                && e["terminal"] == short.as_str()
            {
                seen.push(e["type"].as_str().unwrap().to_string());
            }
        }
        assert!(seen.contains(&"created".to_string()), "{seen:?}");

        // Credit: well past the core's ~256 KiB window
        let flood = run(&c, "sleep 0.3; seq 1 200000; echo flood-done; sleep 600");
        let (_sf, kf) = attach(&c, &flood);
        until("flood", 20, || kf.contains("flood-done"));
        assert!(kf.len() > 1_000_000, "{}", kf.len());

        // Detach: no more bytes after the stream is dropped
        let ticker = run(&c, "while :; do echo tick; sleep 0.05; done");
        let (st, kt) = attach(&c, &ticker);
        until("ticks", 5, || kt.contains("tick"));
        drop(st);
        std::thread::sleep(Duration::from_millis(100));
        let n = kt.len();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(kt.len(), n, "data after detach");

        // Resize: a fresh paint at the new width. The paint breaks wrapped lines with
        // `\r\n`, so the old one can't be wrapped again on our side.
        let long = run(&c, "printf '%0100d\\n' 0 | tr 0 q; sleep 600");
        let (sl, kl) = attach(&c, &long);
        let row = |n: usize| format!("{}\r\n", "q".repeat(n));
        until("paint at 80", 5, || kl.contains(&row(80)));
        sl.resize(40, 24);
        // Changed again while that attach is out: only the latest size counts
        sl.resize(50, 24);
        until("paint at 50", 5, || kl.contains(&row(50)) && !kl.contains(&row(80)));
        let viewers = c.call_blocking("terminal.get", json!({"terminal": long})).unwrap();
        let t = viewers.get("terminal").unwrap_or(&viewers);
        assert_eq!((t["cols"].as_u64(), t["rows"].as_u64()), (Some(50), Some(24)));
        assert!(kl.closes.lock().unwrap().is_empty(), "{:?}", kl.closes.lock().unwrap());

        // Core restart: streams come back by themselves, calls work again
        hc(&["server", "restart"]);
        until("lost", 5, || {
            ka.closes.lock().unwrap().iter().any(|c| c.starts_with("Lost"))
        });
        until("reattached", 10, || *ka.attaches.lock().unwrap() == 2);
        until("b reattached", 10, || *kb.attaches.lock().unwrap() == 2);
        until("repaint", 5, || ka.contains("got-hi"));
        sa.input(b"yo\r");
        until("input after restart", 5, || ka.contains("got-yo"));
        let list = c.call_blocking("terminal.list", json!({})).unwrap();
        assert!(list["terminals"].as_array().unwrap().len() >= 3);

        // A terminal ending closes its stream for good
        hc(&["kill", &b]);
        until("b closed", 5, || {
            kb.closes.lock().unwrap().iter().any(|c| c.starts_with("Ended"))
        });

        hc(&["server", "stop"]);
    }
}
