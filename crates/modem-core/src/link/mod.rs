//! Link layer: transferring a message as frames, acks and retransmissions.
//!
//! [`Link`] is a state machine that knows nothing about audio. It is told what
//! happened (frame received, preamble detected, own transmission played
//! to the end, clock advanced), and it tells what must be done ([`Action`]). Time
//! is supplied from outside in milliseconds, so the tests are deterministic.
//!
//! The same state machine handles both roles: it can send one message
//! (`outgoing`) and receive another (`incoming`). Protocol: DESIGN.md section 5.

pub mod frame;

use crate::crc::crc32;
use crate::rng::Rng;
pub use frame::{Frame, Quality};
use std::collections::VecDeque;

pub type Ms = u64;
const NEVER: Ms = Ms::MAX;

#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// Data per frame (bytes).
    pub chunk: usize,
    /// Maximum message length (bytes).
    pub max_message: usize,
    /// Data frames per burst before an ack. 0 = all missing ones at once,
    /// 1 = per-frame ack.
    pub burst: usize,
    /// A message that fits in one frame is sent with the first start frame
    /// ("inline start"). If it gets no reply, further attempts are short
    /// start frames so that a missing receiver is found out quickly.
    pub inline_start: bool,
    /// The responder waits this long from the end of the received frame (guard time for echo).
    pub response_delay: Ms,
    /// How long a reply is awaited from the end of our own frame (as heard at the microphone).
    pub reply_timeout: Ms,
    /// If our own frame is not heard, it is assumed to have ended this long after
    /// the end of the transmission (upper bound of the device's audio latency).
    pub echo_fallback: Ms,
    pub start_tries: u32,
    pub poll_tries: u32,
    /// Maximum number of retransmission rounds (in addition to the actual transmission).
    pub max_rounds: u32,
    /// The receiver gives up on a partial message when nothing has been heard from
    /// the sender for this long.
    pub rx_timeout: Ms,
    /// The channel must be quiet for this long before starting.
    pub listen: Ms,
    /// Random wait when the channel was busy or the start got no reply.
    pub backoff: (Ms, Ms),
    /// Pause between the frames of a burst.
    pub frame_gap: Ms,
    /// The receiver acks when it has been quiet for this long after a burst
    /// (covers the case where the last frame of the burst was lost).
    pub quiet_ack: Ms,
    /// Memory of completed messages for recognising duplicates.
    pub done_memory: usize,
    pub done_ttl: Ms,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig {
            chunk: 64,
            max_message: 512,
            burst: 0,
            inline_start: true,
            response_delay: 150,
            reply_timeout: 1500,
            echo_fallback: 500,
            start_tries: 3,
            poll_tries: 3,
            max_rounds: 6,
            rx_timeout: 6000,
            listen: 200,
            backoff: (200, 2200),
            frame_gap: 50,
            quiet_ack: 400,
            done_memory: 8,
            done_ttl: 60_000,
        }
    }
}

/// Transmission state for the user interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxStatus {
    /// Listening that the channel is free.
    Listening,
    /// Looking for a receiver (start frame sent).
    Starting { attempt: u32, of: u32 },
    /// Sending a data frame.
    Sending { frame: u32, of: u32, round: u32 },
    WaitingAck,
    /// No ack was heard, asking again.
    Polling { attempt: u32, of: u32 },
    /// The receiver is missing frames, resending them.
    Retrying { missing: u32 },
    /// The other machine started first: our message goes out after the reception.
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    /// Nobody replied to the start.
    NoReceiver,
    /// The receiver stopped replying in the middle of the transfer.
    NoResponse,
    /// The retransmission rounds ran out.
    TooManyRetries,
    /// The receiver acknowledged different content than was sent.
    Mismatch,
}

/// Practical advice on the cause of the failure, derived from the receiver's quality report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    None,
    TooQuiet,
    Clipping,
    Noisy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortReason {
    /// Nothing more was heard from the sender.
    Timeout,
    /// The sender cancelled.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Tx(TxStatus),
    Delivered { id: u16, elapsed: Ms },
    Failed { reason: FailReason, hint: Hint, quality: Option<Quality> },
    Cancelled,
    RxStarted { id: u16, len: usize },
    RxProgress { have: u32, of: u32 },
    Received { id: u16, data: Vec<u8> },
    RxAborted { reason: AbortReason },
    /// Debug log.
    Log(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send this frame through the physical layer.
    Transmit(Vec<u8>),
    /// Abort the transmission in progress immediately.
    AbortTx,
    Event(Event),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The previous transmission is still in progress.
    Busy,
    TooLong { len: usize, max: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Listen,
    /// Yielding to an incoming message.
    Held,
    /// START sent, waiting for a reply.
    StartWait,
    /// Sending a burst; the next frame at this time.
    Burst { next_at: Ms },
    /// Burst (or POLL) sent, waiting for an ack.
    AckWait,
    Cancelling,
}

struct Outgoing {
    id: u16,
    data: Vec<u8>,
    crc: u32,
    n: usize,
    acked: Vec<bool>,
    phase: Phase,
    started_at: Ms,
    not_before: Ms,
    backoff: Option<Ms>,
    start_attempts: u32,
    polls: u32,
    bursts: u32,
    queue: VecDeque<usize>,
    burst_len: u32,
    peer: Option<Quality>,
    sent_anything: bool,
}

struct Incoming {
    id: u16,
    len: usize,
    chunk: usize,
    crc: u32,
    parts: Vec<Option<Vec<u8>>>,
    last_valid: Ms,
    burst_seen: bool,
}

pub struct Link {
    cfg: LinkConfig,
    rng: Rng,
    outgoing: Option<Outgoing>,
    incoming: Option<Incoming>,
    /// The reply to send in the receiver role and the time to send it.
    reply: Option<(Ms, Frame)>,
    /// Completed messages: (identifier, CRC-32, length, completion time).
    done: VecDeque<(u16, u32, usize, Ms)>,
    actions: VecDeque<Action>,
    in_flight: bool,
    last_sent: Vec<u8>,
    /// Deadline for the reply in the sender role.
    deadline: Ms,
    last_activity: Ms,
    last_tx_done: Ms,
}

impl Link {
    pub fn new(cfg: LinkConfig, seed: u64) -> Self {
        Link {
            cfg,
            rng: Rng::new(seed),
            outgoing: None,
            incoming: None,
            reply: None,
            done: VecDeque::new(),
            actions: VecDeque::new(),
            in_flight: false,
            last_sent: Vec::new(),
            deadline: NEVER,
            last_activity: 0,
            last_tx_done: 0,
        }
    }

    pub fn config(&self) -> &LinkConfig {
        &self.cfg
    }

    pub fn poll_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    pub fn is_sending(&self) -> bool {
        self.outgoing.is_some()
    }

    pub fn is_receiving(&self) -> bool {
        self.incoming.is_some()
    }

    fn event(&mut self, e: Event) {
        self.actions.push_back(Action::Event(e));
    }

    fn log(&mut self, s: String) {
        self.actions.push_back(Action::Event(Event::Log(s)));
    }

    fn random_backoff(&mut self) -> Ms {
        let (lo, hi) = self.cfg.backoff;
        lo + self.rng.below(hi - lo + 1)
    }

    fn transmit(&mut self, f: Frame) {
        let bytes = f.encode();
        self.log(format!("-> {} id={:04x} ({} bytes)", f.name(), f.id(), bytes.len()));
        self.last_sent = bytes.clone();
        self.in_flight = true;
        self.deadline = NEVER;
        self.actions.push_back(Action::Transmit(bytes));
    }

    // ---------------------------------------------------------------- user actions

    /// Start sending a message.
    pub fn send(&mut self, now: Ms, data: &[u8]) -> Result<u16, SendError> {
        if self.outgoing.is_some() {
            return Err(SendError::Busy);
        }
        if data.len() > self.cfg.max_message {
            return Err(SendError::TooLong { len: data.len(), max: self.cfg.max_message });
        }
        let id = self.rng.next_u32() as u16;
        let n = data.len().div_ceil(self.cfg.chunk);
        let held = self.incoming.is_some();
        self.outgoing = Some(Outgoing {
            id,
            data: data.to_vec(),
            crc: crc32(data),
            n,
            acked: vec![false; n],
            phase: if held { Phase::Held } else { Phase::Listen },
            started_at: now,
            not_before: now,
            backoff: None,
            start_attempts: 0,
            polls: 0,
            bursts: 0,
            queue: VecDeque::new(),
            burst_len: 0,
            peer: None,
            sent_anything: false,
        });
        self.event(Event::Tx(if held { TxStatus::Deferred } else { TxStatus::Listening }));
        Ok(id)
    }

    /// Cancel the transmission in progress.
    pub fn cancel(&mut self, _now: Ms) {
        let Some(o) = self.outgoing.as_mut() else { return };
        if o.phase == Phase::Cancelling {
            return;
        }
        let (id, sent) = (o.id, o.sent_anything);
        if self.in_flight && self.last_sent_is_sender_frame() {
            self.actions.push_back(Action::AbortTx);
            self.in_flight = false;
        }
        if sent && !self.in_flight {
            // Tell the receiver so that it is not left waiting.
            self.outgoing.as_mut().unwrap().phase = Phase::Cancelling;
            self.transmit(Frame::Cancel { id });
        } else {
            self.outgoing = None;
            self.event(Event::Cancelled);
        }
    }

    fn last_sent_is_sender_frame(&self) -> bool {
        Frame::decode(&self.last_sent).map_or(false, |f| f.from_sender())
    }

    // ---------------------------------------------------------------- physical layer events

    fn activity(&mut self, now: Ms) {
        self.last_activity = self.last_activity.max(now);
        // The channel was busy during listening: wait a random time after it frees up.
        if matches!(self.outgoing.as_ref().map(|o| (o.phase, o.backoff)), Some((Phase::Listen, None))) {
            let r = self.random_backoff();
            self.outgoing.as_mut().unwrap().backoff = Some(r);
        }
    }

    /// Preamble detected. Counted only as channel activity: a bare preamble
    /// can be a false alarm (a click, speech), so it does not trigger an ack.
    pub fn on_preamble(&mut self, now: Ms) {
        self.activity(now);
    }

    /// The frame header decoded but the payload did not: someone really is transmitting.
    /// During reception it means that a burst is in progress. The echo of our own
    /// transmission is ruled out by timing.
    pub fn on_rx_failed(&mut self, now: Ms) {
        self.activity(now);
        if !self.in_flight && now >= self.last_tx_done + self.cfg.echo_fallback {
            if let Some(inc) = self.incoming.as_mut() {
                inc.burst_seen = true;
            }
        }
    }

    /// Our own transmission has been played to the end (on the audio output buffer's timeline).
    pub fn on_tx_done(&mut self, now: Ms) {
        if !self.in_flight {
            return;
        }
        self.in_flight = false;
        self.last_tx_done = now;
        let sent = Frame::decode(&self.last_sent);
        let Some(o) = self.outgoing.as_mut() else { return };
        match (o.phase, sent) {
            (Phase::Cancelling, _) => {
                self.outgoing = None;
                self.event(Event::Cancelled);
            }
            (Phase::StartWait, Some(Frame::Start { .. })) | (Phase::AckWait, Some(Frame::Poll { .. })) => {
                self.deadline = now + self.cfg.echo_fallback + self.cfg.reply_timeout;
            }
            (Phase::Burst { .. }, Some(Frame::Data { eob, .. })) => {
                if eob {
                    o.phase = Phase::AckWait;
                    self.deadline = now + self.cfg.echo_fallback + self.cfg.reply_timeout;
                    self.event(Event::Tx(TxStatus::WaitingAck));
                } else {
                    o.phase = Phase::Burst { next_at: now + self.cfg.frame_gap };
                }
            }
            _ => {}
        }
    }

    /// Intact frame received. `now` is the end time of the frame on the microphone's timeline.
    pub fn on_frame(&mut self, now: Ms, bytes: &[u8], quality: Quality) {
        self.activity(now);
        let Some(f) = Frame::decode(bytes) else {
            self.log(format!("<- unknown frame ({} bytes)", bytes.len()));
            return;
        };
        let id = f.id();
        let as_sender = self.outgoing.as_ref().map_or(false, |o| o.id == id);
        let as_receiver = self.incoming.as_ref().map_or(false, |i| i.id == id) || self.done.iter().any(|d| d.0 == id);

        // Own echo: a frame type that only I send in this role.
        if (f.from_sender() && as_sender) || (!f.from_sender() && !as_sender && as_receiver) {
            if bytes == self.last_sent.as_slice() && !self.in_flight && self.deadline != NEVER {
                // Exact anchor: the sound stopped just now.
                self.deadline = now + self.cfg.reply_timeout;
            }
            self.log(format!("<- {} id={id:04x} (own echo)", f.name()));
            return;
        }
        self.log(format!("<- {} id={id:04x} snr={} dB level={} dBFS", f.name(), quality.snr_db, quality.level_db));
        // A frame other than a data frame means that no burst is in progress: its preamble must
        // not trigger the silence-based ack.
        if !matches!(f, Frame::Data { .. }) {
            if let Some(inc) = self.incoming.as_mut() {
                inc.burst_seen = false;
            }
        }

        match f {
            Frame::Start { id, len, frames, chunk, crc, inline } => self.rx_start(now, id, len as usize, frames as usize, chunk as usize, crc, inline, quality),
            Frame::Data { id, seq, eob, data } => self.rx_data(now, id, seq as usize, eob, data, quality),
            Frame::Poll { id } => {
                if self.incoming.as_ref().map_or(false, |i| i.id == id) {
                    self.incoming.as_mut().unwrap().last_valid = now;
                    let reply = self.make_ack(now, quality);
                    self.reply = Some((now + self.cfg.response_delay, reply));
                } else if let Some(d) = self.done.iter().find(|d| d.0 == id) {
                    self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc: d.1 }));
                }
            }
            Frame::Cancel { id } => {
                if self.incoming.as_ref().map_or(false, |i| i.id == id) {
                    self.end_incoming(now);
                    self.reply = None;
                    self.event(Event::RxAborted { reason: AbortReason::Cancelled });
                }
            }
            Frame::StartAck { quality: q, .. } if as_sender => {
                let o = self.outgoing.as_mut().unwrap();
                if o.phase == Phase::StartWait {
                    o.peer = Some(q);
                    self.begin_burst(now);
                }
            }
            Frame::Ack { quality: q, restart, bitmap, .. } if as_sender => self.tx_ack(now, q, restart, &bitmap),
            Frame::Done { crc, .. } if as_sender => {
                let o = self.outgoing.take().unwrap();
                if o.phase == Phase::Cancelling {
                    self.outgoing = Some(o);
                } else if crc == o.crc {
                    self.deadline = NEVER;
                    self.event(Event::Delivered { id, elapsed: now.saturating_sub(o.started_at) });
                } else {
                    self.deadline = NEVER;
                    self.event(Event::Failed { reason: FailReason::Mismatch, hint: Hint::None, quality: o.peer });
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- receiver role

    #[allow(clippy::too_many_arguments)]
    fn rx_start(&mut self, now: Ms, id: u16, len: usize, frames: usize, chunk: usize, crc: u32, inline: Option<Vec<u8>>, q: Quality) {
        // Already completed message: the final ack was lost. Ack again, do not show it a second time.
        if self.done.iter().any(|d| d.0 == id && d.1 == crc && d.2 == len) {
            self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc }));
            return;
        }
        if let Some(inc) = self.incoming.as_mut() {
            if inc.id == id {
                // The start ack was lost: reply again without starting over.
                inc.last_valid = now;
                self.reply = Some((now + self.cfg.response_delay, Frame::StartAck { id, quality: q }));
            }
            return; // A foreign start in the middle of another reception: ignore.
        }
        if len > self.cfg.max_message || chunk == 0 || (inline.is_none() && frames != len.div_ceil(chunk)) {
            self.log(format!("start rejected: length {len}, frames {frames}, data size {chunk}"));
            return;
        }
        // Simultaneous transmission: do we yield?
        if let Some(o) = self.outgoing.as_mut() {
            let yield_now = match o.phase {
                Phase::Listen | Phase::Held => true,
                // Both managed to send a start: the smaller identifier keeps the turn.
                Phase::StartWait => id < o.id,
                _ => false,
            };
            if !yield_now {
                self.log("foreign start ignored: own transmission is in progress".into());
                return;
            }
            if self.in_flight {
                self.actions.push_back(Action::AbortTx);
                self.in_flight = false;
            }
            o.phase = Phase::Held;
            self.deadline = NEVER;
            self.event(Event::Tx(TxStatus::Deferred));
        }
        self.event(Event::RxStarted { id, len });
        if let Some(data) = inline {
            if data.len() != len || crc32(&data) != crc {
                self.log("the message that came with the start frame does not match".into());
                return;
            }
            self.complete(now, id, crc, data);
            return;
        }
        if frames == 0 {
            self.complete(now, id, crc, Vec::new());
            return;
        }
        self.incoming = Some(Incoming { id, len, chunk, crc, parts: vec![None; frames], last_valid: now, burst_seen: false });
        self.reply = Some((now + self.cfg.response_delay, Frame::StartAck { id, quality: q }));
    }

    fn rx_data(&mut self, now: Ms, id: u16, seq: usize, eob: bool, data: Vec<u8>, q: Quality) {
        if let Some(inc) = self.incoming.as_mut().filter(|i| i.id == id) {
            inc.last_valid = now;
            let n = inc.parts.len();
            let expected = if seq + 1 == n { inc.len - inc.chunk * (n - 1) } else { inc.chunk };
            if seq < n && data.len() == expected {
                inc.parts[seq] = Some(data);
                let have = inc.parts.iter().filter(|p| p.is_some()).count() as u32;
                self.event(Event::RxProgress { have, of: n as u32 });
            }
            let inc = self.incoming.as_mut().unwrap();
            if eob {
                inc.burst_seen = false;
                let reply = self.make_ack(now, q);
                self.reply = Some((now + self.cfg.response_delay, reply));
            } else {
                inc.burst_seen = true;
            }
        } else if let Some(d) = self.done.iter().find(|d| d.0 == id) {
            // The message is already complete, but the sender does not know it.
            if eob {
                self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc: d.1 }));
            }
        }
    }

    /// Ack of the current situation: DONE if the message is complete and intact.
    fn make_ack(&mut self, now: Ms, q: Quality) -> Frame {
        let inc = self.incoming.as_mut().unwrap();
        let id = inc.id;
        if inc.parts.iter().all(|p| p.is_some()) {
            let data: Vec<u8> = inc.parts.iter().flat_map(|p| p.clone().unwrap()).collect();
            let crc = inc.crc;
            if crc32(&data) == crc {
                self.complete(now, id, crc, data);
                return Frame::Done { id, crc };
            }
            // Some frame passed its check while corrupted: start over.
            inc.parts.iter_mut().for_each(|p| *p = None);
            self.log("assembled message does not match the checksum, requesting everything again".into());
            let n = self.incoming.as_ref().unwrap().parts.len();
            return Frame::Ack { id, quality: q, restart: true, bitmap: vec![0; n.div_ceil(8)] };
        }
        let mut bitmap = vec![0u8; inc.parts.len().div_ceil(8)];
        for (i, p) in inc.parts.iter().enumerate() {
            if p.is_some() {
                bitmap[i / 8] |= 0x80 >> (i % 8);
            }
        }
        Frame::Ack { id, quality: q, restart: false, bitmap }
    }

    /// The message has arrived: show it, remember the identifier and ack.
    fn complete(&mut self, now: Ms, id: u16, crc: u32, data: Vec<u8>) {
        self.done.push_back((id, crc, data.len(), now));
        while self.done.len() > self.cfg.done_memory {
            self.done.pop_front();
        }
        self.event(Event::Received { id, data });
        self.end_incoming(now);
        self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc }));
    }

    fn end_incoming(&mut self, now: Ms) {
        self.incoming = None;
        // Our own message that yielded now gets its turn.
        if let Some(o) = self.outgoing.as_mut().filter(|o| o.phase == Phase::Held) {
            o.phase = Phase::Listen;
            o.not_before = now;
            o.backoff = None;
            self.event(Event::Tx(TxStatus::Listening));
        }
    }

    // ---------------------------------------------------------------- sender role

    fn start_frame(&self) -> Frame {
        let o = self.outgoing.as_ref().unwrap();
        let inline = (self.cfg.inline_start && o.n <= 1 && o.start_attempts <= 1).then(|| o.data.clone());
        Frame::Start { id: o.id, len: o.data.len() as u16, frames: o.n as u8, chunk: self.cfg.chunk as u8, crc: o.crc, inline }
    }

    fn begin_burst(&mut self, now: Ms) {
        let burst = self.cfg.burst;
        let delay = self.cfg.response_delay;
        let o = self.outgoing.as_mut().unwrap();
        let missing = o.acked.iter().enumerate().filter(|(_, a)| !**a).map(|(i, _)| i);
        o.queue = if burst == 0 { missing.collect() } else { missing.take(burst).collect() };
        o.burst_len = o.queue.len() as u32;
        o.bursts += 1;
        o.polls = 0;
        o.phase = Phase::Burst { next_at: now + delay };
        self.deadline = NEVER;
    }

    fn tx_ack(&mut self, now: Ms, q: Quality, restart: bool, bitmap: &[u8]) {
        let per_burst = self.cfg.burst;
        let max_rounds = self.cfg.max_rounds;
        let o = self.outgoing.as_mut().unwrap();
        if o.phase != Phase::AckWait {
            return;
        }
        o.peer = Some(q);
        for (i, a) in o.acked.iter_mut().enumerate() {
            *a = !restart && bitmap.get(i / 8).map_or(false, |b| b & (0x80 >> (i % 8)) != 0);
        }
        let missing = o.acked.iter().filter(|a| !**a).count() as u32;
        if missing == 0 {
            // Everything delivered, but no final ack came: ask for it.
            self.deadline = now;
            return;
        }
        let first_pass = if per_burst == 0 { 1 } else { o.n.div_ceil(per_burst) as u32 };
        if o.bursts >= first_pass + max_rounds {
            self.fail(FailReason::TooManyRetries);
            return;
        }
        // During the first pass (per-frame ack) the missing
        // frames are still unsent, not lost.
        if o.bursts >= first_pass {
            self.event(Event::Tx(TxStatus::Retrying { missing }));
        }
        self.begin_burst(now);
    }

    fn fail(&mut self, reason: FailReason) {
        let Some(o) = self.outgoing.take() else { return };
        self.deadline = NEVER;
        let hint = match (reason, o.peer) {
            (FailReason::NoReceiver, _) | (_, None) => Hint::None,
            (_, Some(q)) if q.clipped => Hint::Clipping,
            (_, Some(q)) if q.snr_db < 8 && q.level_db < -55 => Hint::TooQuiet,
            (_, Some(q)) if q.snr_db < 8 => Hint::Noisy,
            _ => Hint::None,
        };
        self.event(Event::Failed { reason, hint, quality: o.peer });
    }

    // ---------------------------------------------------------------- clock

    /// The clock advanced. `busy`: the physical layer is currently receiving a frame.
    pub fn tick(&mut self, now: Ms, busy: bool) {
        if busy {
            self.activity(now);
        }
        while self.done.front().map_or(false, |d| now.saturating_sub(d.3) > self.cfg.done_ttl) {
            self.done.pop_front();
        }

        // Receiver: scheduled reply.
        if !self.in_flight {
            if let Some((at, _)) = self.reply {
                if now >= at {
                    let (_, f) = self.reply.take().unwrap();
                    self.transmit(f);
                }
            }
        }

        // Receiver: the burst ended without the "last frame" flag.
        if let Some(inc) = self.incoming.as_mut() {
            if inc.burst_seen && !busy && !self.in_flight && self.reply.is_none() && now.saturating_sub(self.last_activity) >= self.cfg.quiet_ack {
                inc.burst_seen = false;
                let reply = self.make_ack(now, Quality::default());
                self.log("burst ended without the end flag, acking".into());
                self.reply = Some((now, reply));
            }
        }

        // Receiver: the sender disappeared.
        if let Some(inc) = self.incoming.as_ref() {
            if !busy && now.saturating_sub(inc.last_valid.max(self.last_activity)) >= self.cfg.rx_timeout {
                self.end_incoming(now);
                self.reply = None;
                self.event(Event::RxAborted { reason: AbortReason::Timeout });
            }
        }

        let Some(o) = self.outgoing.as_mut() else { return };
        match o.phase {
            Phase::Listen => {
                let quiet_for = now.saturating_sub(self.last_activity);
                let need = o.backoff.unwrap_or(self.cfg.listen).max(self.cfg.listen);
                let free = !busy && !self.in_flight && self.incoming.is_none() && self.reply.is_none();
                if free && now >= o.not_before + self.cfg.listen && (quiet_for >= need || self.last_activity == 0) {
                    o.phase = Phase::StartWait;
                    o.sent_anything = true;
                    o.start_attempts += 1;
                    let (attempt, of) = (o.start_attempts, self.cfg.start_tries);
                    self.event(Event::Tx(TxStatus::Starting { attempt, of }));
                    let f = self.start_frame();
                    self.transmit(f);
                }
            }
            Phase::Burst { next_at } => {
                if !self.in_flight && now >= next_at {
                    if let Some(seq) = o.queue.pop_front() {
                        let eob = o.queue.is_empty();
                        let chunk = self.cfg.chunk;
                        let data = o.data[seq * chunk..((seq + 1) * chunk).min(o.data.len())].to_vec();
                        let (id, frame, of, round) = (o.id, o.burst_len - o.queue.len() as u32, o.burst_len, o.bursts);
                        self.event(Event::Tx(TxStatus::Sending { frame, of, round }));
                        self.transmit(Frame::Data { id, seq: seq as u8, eob, data });
                    }
                }
            }
            Phase::StartWait | Phase::AckWait => {
                // If a reply is arriving right now, wait for it to finish.
                if self.in_flight || busy || now < self.deadline {
                    return;
                }
                if o.phase == Phase::StartWait {
                    if o.start_attempts >= self.cfg.start_tries {
                        self.fail(FailReason::NoReceiver);
                    } else {
                        // Perhaps a collision with another start: a random wait breaks the symmetry.
                        let r = self.random_backoff();
                        let o = self.outgoing.as_mut().unwrap();
                        o.phase = Phase::Listen;
                        o.not_before = now + r;
                        o.backoff = None;
                        self.deadline = NEVER;
                        self.log(format!("no reply to start, retrying in {r} ms"));
                    }
                } else if o.polls >= self.cfg.poll_tries {
                    self.fail(FailReason::NoResponse);
                } else {
                    o.polls += 1;
                    let (id, attempt, of) = (o.id, o.polls, self.cfg.poll_tries);
                    self.event(Event::Tx(TxStatus::Polling { attempt, of }));
                    self.transmit(Frame::Poll { id });
                }
            }
            Phase::Held | Phase::Cancelling => {}
        }
    }
}

#[cfg(test)]
mod tests;
