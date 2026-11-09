//! A client for the herdr-core (`hc`) socket.
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
//! `terminal.attach` returns a stream channel; its frames are described at [`Kind`].

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

/// The JSON-RPC channel.
pub const CONTROL: u32 = 0;

/// Frame kinds on a stream channel. Learned from `hc attach` traffic (hc 0.1.18); the
/// OpenRPC schema names them but doesn't give their numbers.
#[allow(dead_code)]
pub mod kind {
    /// Core → viewer: terminal bytes (first the paint, then live output).
    /// Viewer → core: input bytes, as if typed.
    pub const DATA: u8 = 0;
    /// Viewer → core: u32 LE, the number of DATA payload bytes consumed.
    pub const CREDIT: u8 = 1;
    /// Core → viewer: the stream ended. Payload is JSON:
    /// `{"exit_code":0,"finished_ms":…,"reason":"exited","signal":null}`.
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

pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// `$HC_SOCKET`, else `$XDG_RUNTIME_DIR/herdr-core/<$HC_SESSION or default>.sock`.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("HC_SOCKET") {
        return Ok(p.into());
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let session = std::env::var("HC_SESSION").unwrap_or_else(|_| "default".into());
    Ok(PathBuf::from(dir)
        .join("herdr-core")
        .join(format!("{session}.sock")))
}

/// The write half; cheap to clone, safe to use from any thread.
#[derive(Clone)]
pub struct Writer(Arc<Mutex<UnixStream>>);

impl Writer {
    pub fn frame(&self, channel: u32, kind: u8, payload: &[u8]) -> Result<()> {
        let mut buf = Vec::with_capacity(9 + payload.len());
        buf.extend_from_slice(&channel.to_le_bytes());
        buf.push(kind);
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        let mut s = self.0.lock().unwrap();
        s.write_all(&buf)?;
        Ok(())
    }
}

pub struct Conn {
    reader: BufReader<UnixStream>,
    pub writer: Writer,
    next_id: u64,
    /// Stream frames that arrived while waiting for a reply.
    pending: VecDeque<Frame>,
}

impl Conn {
    pub fn connect() -> Result<Self> {
        let path = socket_path()?;
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("connecting to {} (is hc running?)", path.display()))?;
        let mut reader = BufReader::with_capacity(1 << 20, stream.try_clone()?);
        let mut w = stream;
        let hello = json!({"jsonrpc":"2.0","id":0,"method":"hello",
                           "params":{"generation":1,"framed":true}});
        w.write_all(format!("{hello}\n").as_bytes())?;
        // The hello reply is the last line-mode message; frames follow it in the same buffer.
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let reply: Value = serde_json::from_str(&line).context("hello reply")?;
        if let Some(err) = reply.get("error") {
            bail!("hello: {err}");
        }
        if reply["result"]["framed"] != json!(true) {
            bail!("core did not switch to framed mode: {line}");
        }
        eprintln!(
            "nerdr: hc {} on {}",
            reply["result"]["version"].as_str().unwrap_or("?"),
            reply["result"]["core"]["name"].as_str().unwrap_or("?")
        );
        Ok(Self {
            reader,
            writer: Writer(Arc::new(Mutex::new(w))),
            next_id: 1,
            pending: VecDeque::new(),
        })
    }

    /// A JSON-RPC call on channel 0. Blocks until its reply.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.writer
            .frame(CONTROL, kind::DATA, req.to_string().as_bytes())?;
        loop {
            let f = read_frame(&mut self.reader)?;
            if f.channel != CONTROL {
                self.pending.push_back(f);
                continue;
            }
            let msg: Value = serde_json::from_slice(&f.payload)?;
            if msg.get("id") != Some(&json!(id)) {
                eprintln!("nerdr: control message while waiting: {msg}");
                continue;
            }
            if let Some(err) = msg.get("error") {
                return Err(anyhow!("{method}: {err}"));
            }
            return Ok(msg["result"].clone());
        }
    }

    /// Hands the read half over to a stream reader, with the frames already received.
    pub fn into_reader(self) -> FrameReader {
        FrameReader {
            reader: self.reader,
            pending: self.pending,
        }
    }
}

pub struct FrameReader {
    reader: BufReader<UnixStream>,
    pending: VecDeque<Frame>,
}

impl FrameReader {
    /// The next frame; blocks.
    pub fn next(&mut self) -> Result<Frame> {
        match self.pending.pop_front() {
            Some(f) => Ok(f),
            None => read_frame(&mut self.reader),
        }
    }

    /// The next frame if it is already buffered whole, so callers can batch work without
    /// blocking.
    pub fn next_buffered(&mut self) -> Option<Result<Frame>> {
        if let Some(f) = self.pending.pop_front() {
            return Some(Ok(f));
        }
        let buf = self.reader.buffer();
        if buf.len() < 9 {
            return None;
        }
        let len = u32::from_le_bytes(buf[5..9].try_into().unwrap()) as usize;
        if buf.len() < 9 + len {
            return None;
        }
        Some(read_frame(&mut self.reader))
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
