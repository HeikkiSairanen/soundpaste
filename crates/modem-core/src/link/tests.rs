//! Link layer tests: two state machines and a simulated "air" at the frame level.
//!
//! The air models what the link layer sees of the audio:
//! - a frame has a duration, and it arrives after the duration and a delay
//! - a transmitting machine does not hear the other (half duplex), and overlapping
//!   frames destroy each other
//! - a machine hears its own frame as an echo
//! - a test can corrupt or drop selected frames
//!
//! Every exceptional case in DESIGN.md section 5.6 has its own test.

use super::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fate {
    Deliver,
    /// The preamble is heard, but the frame does not decode.
    Corrupt,
    /// Nothing is heard.
    Vanish,
}

struct Ctx<'a> {
    from: usize,
    frame: &'a Frame,
    /// Which frame of this type from this machine this is (1 = first).
    nth: u32,
    now: Ms,
}

struct OnAir {
    from: usize,
    start: Ms,
    end: Ms,
    bytes: Vec<u8>,
    fate: Fate,
    aborted: bool,
}

struct Node {
    link: Link,
    events: Vec<Event>,
    tx_until: Option<Ms>,
    hears_self: bool,
    alive: bool,
}

struct Net {
    nodes: Vec<Node>,
    now: Ms,
    air: Vec<OnAir>,
    latency: Ms,
    rule: Box<dyn FnMut(&Ctx) -> Fate>,
    counts: std::collections::HashMap<(usize, &'static str), u32>,
    /// All frames sent: (time, sender, frame).
    sent: Vec<(Ms, usize, Frame)>,
}

const STEP: Ms = 5;
/// The preamble is detected this long after the start of the frame.
const DETECT: Ms = 150;

fn duration(len: usize) -> Ms {
    // Roughly the duration of the v1 waveform: 240 ms + 50 ms per 2.7 bytes, to 5 ms precision.
    (240 + (len as Ms + 2) * 75 / 4 + 4) / STEP * STEP
}

impl Net {
    fn new(cfg: LinkConfig) -> Net {
        Net::with_configs(cfg.clone(), cfg)
    }

    fn with_configs(a: LinkConfig, b: LinkConfig) -> Net {
        let node = |cfg, seed| Node { link: Link::new(cfg, seed), events: Vec::new(), tx_until: None, hears_self: true, alive: true };
        Net {
            nodes: vec![node(a, 11), node(b, 22)],
            now: 0,
            air: Vec::new(),
            latency: 40,
            rule: Box::new(|_| Fate::Deliver),
            counts: Default::default(),
            sent: Vec::new(),
        }
    }

    fn rule(mut self, f: impl FnMut(&Ctx) -> Fate + 'static) -> Net {
        self.rule = Box::new(f);
        self
    }

    fn transmitting(&self, node: usize, from: Ms, to: Ms) -> bool {
        self.air.iter().any(|f| f.from == node && !f.aborted && f.start < to && f.end > from)
    }

    fn step(&mut self) {
        self.now += STEP;
        let now = self.now;
        for i in 0..2 {
            if !self.nodes[i].alive {
                continue;
            }
            let peer = 1 - i;
            if self.nodes[i].tx_until == Some(now) {
                self.nodes[i].tx_until = None;
                self.nodes[i].link.on_tx_done(now);
            }
            // What does this machine hear right now?
            let mut busy = false;
            let mut deliveries: Vec<(Vec<u8>, bool)> = Vec::new();
            let mut preambles = 0;
            let mut failures = 0;
            for f in &self.air {
                if f.aborted {
                    continue;
                }
                let (heard_from, heard_to) = (f.start + self.latency, f.end + self.latency);
                if f.from == i {
                    if self.nodes[i].hears_self && heard_to == now {
                        deliveries.push((f.bytes.clone(), true));
                    }
                    continue;
                }
                debug_assert_eq!(f.from, peer);
                if f.fate == Fate::Vanish {
                    continue;
                }
                // Our own transmission or another overlapping frame masks this one.
                let masked = self.transmitting(i, f.start, f.end);
                if masked {
                    continue;
                }
                if now == heard_from + DETECT {
                    preambles += 1;
                }
                if now >= heard_from + DETECT && now < heard_to {
                    busy = true;
                }
                if heard_to == now {
                    if f.fate == Fate::Deliver {
                        deliveries.push((f.bytes.clone(), false));
                    } else {
                        failures += 1;
                    }
                }
            }
            let node = &mut self.nodes[i];
            for _ in 0..preambles {
                node.link.on_preamble(now);
            }
            for _ in 0..failures {
                node.link.on_rx_failed(now);
            }
            for (bytes, _own) in deliveries {
                node.link.on_frame(now, &bytes, Quality { snr_db: 25, level_db: -30, clipped: false });
            }
            node.link.tick(now, busy);
            while let Some(action) = node.link.poll_action() {
                match action {
                    Action::Transmit(bytes) => {
                        let frame = Frame::decode(&bytes).expect("the link sent an invalid frame");
                        assert!(node.tx_until.is_none(), "machine {i} sent a frame on top of the previous one");
                        let nth = self.counts.entry((i, frame.name())).or_insert(0);
                        *nth += 1;
                        let fate = (self.rule)(&Ctx { from: i, frame: &frame, nth: *nth, now });
                        let end = now + duration(bytes.len());
                        self.sent.push((now, i, frame));
                        self.air.push(OnAir { from: i, start: now, end, bytes, fate, aborted: false });
                        node.tx_until = Some(end);
                    }
                    Action::AbortTx => {
                        node.tx_until = None;
                        if let Some(f) = self.air.iter_mut().rev().find(|f| f.from == i && f.end > now) {
                            f.aborted = true;
                        }
                    }
                    Action::Event(e) => node.events.push(e),
                }
            }
        }
        self.air.retain(|f| f.end + 1000 > now);
    }

    fn run(&mut self, ms: Ms) {
        let until = self.now + ms;
        while self.now < until {
            self.step();
        }
    }

    /// Run until the condition holds. Panics if the time limit is exceeded.
    fn run_until(&mut self, limit: Ms, what: &str, cond: impl Fn(&Net) -> bool) {
        let until = self.now + limit;
        while !cond(self) {
            assert!(self.now < until, "time limit exceeded: {what}\n{}", self.trace());
            self.step();
        }
    }

    fn send(&mut self, node: usize, data: &[u8]) {
        let now = self.now;
        self.nodes[node].link.send(now, data).unwrap();
    }

    fn received(&self, node: usize) -> Vec<Vec<u8>> {
        self.nodes[node].events.iter().filter_map(|e| if let Event::Received { data, .. } = e { Some(data.clone()) } else { None }).collect()
    }

    fn has(&self, node: usize, pred: impl Fn(&Event) -> bool) -> bool {
        self.nodes[node].events.iter().any(pred)
    }

    fn delivered(&self, node: usize) -> bool {
        self.has(node, |e| matches!(e, Event::Delivered { .. }))
    }

    fn failed(&self, node: usize) -> Option<FailReason> {
        self.nodes[node].events.iter().find_map(|e| if let Event::Failed { reason, .. } = e { Some(*reason) } else { None })
    }

    fn count(&self, node: usize, name: &str) -> usize {
        self.sent.iter().filter(|(_, n, f)| *n == node && f.name() == name).count()
    }

    fn names(&self) -> Vec<String> {
        self.sent.iter().map(|(_, n, f)| format!("{}:{}", ["A", "B"][*n], f.name())).collect()
    }

    fn trace(&self) -> String {
        let mut s = String::new();
        for (t, n, f) in &self.sent {
            s += &format!("{t:>6} ms  {}  {}\n", ["A", "B"][*n], f.name());
        }
        s
    }

    fn idle(&self) -> bool {
        self.nodes.iter().all(|n| !n.link.is_sending() && !n.link.is_receiving() && n.tx_until.is_none())
    }
}

/// Default for the tests: a separate handshake so that the START_ACK and DATA paths are covered
/// with short messages too. Inline start has its own tests.
fn handshake() -> LinkConfig {
    LinkConfig { inline_start: false, ..LinkConfig::default() }
}

const LINK: &[u8] = b"https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd";

fn message(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7) as u8).collect()
}

// ------------------------------------------------------------------ normal session

#[test]
fn normal_session_short_message() {
    let mut net = Net::new(handshake());
    net.send(0, LINK);
    net.run_until(10_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "B:START_ACK", "A:DATA", "B:DONE"]);
    assert!(net.has(1, |e| matches!(e, Event::RxStarted { len: 60, .. })));
    net.run(1000);
    assert!(net.idle());
    // The sender and receiver return to idle and can swap roles.
    net.send(1, b"response");
    net.run_until(10_000, "other direction", |n| n.delivered(1));
    assert_eq!(net.received(0), vec![b"response".to_vec()]);
}

#[test]
fn long_message_goes_in_one_burst() {
    let mut net = Net::new(handshake());
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!(net.count(0, "DATA"), 5);
    assert_eq!(net.count(1, "ACK"), 0);
    assert_eq!(net.count(1, "DONE"), 1);
    // Progress is visible to the receiver one frame at a time.
    let progress: Vec<u32> = net.nodes[1].events.iter().filter_map(|e| if let Event::RxProgress { have, .. } = e { Some(*have) } else { None }).collect();
    assert_eq!(progress, [1, 2, 3, 4, 5]);
}

#[test]
fn per_frame_ack_mode() {
    let cfg = LinkConfig { burst: 1, ..handshake() };
    let mut net = Net::new(cfg);
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(0, "DATA"), net.count(1, "ACK"), net.count(1, "DONE")), (4, 3, 1));
    assert!(!net.has(0, |e| matches!(e, Event::Tx(TxStatus::Retrying { .. }))));
}

#[test]
fn inline_start_delivers_in_two_frames() {
    let mut net = Net::new(LinkConfig::default());
    net.send(0, LINK);
    net.run_until(10_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "B:DONE"]);
    // A longer message uses the ordinary handshake.
    let msg = message(100);
    net.send(0, &msg);
    net.run_until(20_000, "long one delivered", |n| n.received(1).len() == 2);
    assert_eq!(net.count(1, "START_ACK"), 1);
}

// ------------------------------------------------------------------ exceptional cases

#[test]
fn lost_data_frame_is_retransmitted_alone() {
    let mut net = Net::new(handshake()).rule(|c| if c.from == 0 && matches!(c.frame, Frame::Data { seq: 2, .. }) && c.nth <= 3 { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    // Five frames + one retransmission, one combined ack.
    assert_eq!((net.count(0, "DATA"), net.count(1, "ACK")), (6, 1));
    assert!(net.has(0, |e| matches!(e, Event::Tx(TxStatus::Retrying { missing: 1 }))));
}

#[test]
fn lost_last_frame_of_burst_is_acked_after_silence() {
    // The last frame of the burst (the one with the end flag) is corrupted: the receiver
    // still acks when the channel goes quiet.
    let mut net = Net::new(handshake()).rule(|c| if matches!(c.frame, Frame::Data { eob: true, .. }) && c.nth == 5 { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(1, "ACK"), net.count(0, "POLL")), (1, 0), "{}", net.trace());
}

#[test]
fn lost_ack_is_requested_again() {
    let mut net = Net::new(handshake())
        .rule(|c| if (matches!(c.frame, Frame::Data { seq: 0, .. }) && c.nth == 1) || (c.frame.name() == "ACK" && c.nth == 1) { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(150);
    net.send(0, &msg);
    net.run_until(30_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(0, "POLL"), net.count(1, "ACK")), (1, 2), "{}", net.trace());
    assert!(net.has(0, |e| matches!(e, Event::Tx(TxStatus::Polling { attempt: 1, .. }))));
}

#[test]
fn lost_start_ack_restarts_handshake_without_restarting_reception() {
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "START_ACK" && c.nth == 1 { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(15_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!((net.count(0, "START"), net.count(1, "START_ACK")), (2, 2));
    let starts = net.nodes[1].events.iter().filter(|e| matches!(e, Event::RxStarted { .. })).count();
    assert_eq!(starts, 1);
}

#[test]
fn lost_final_ack_does_not_show_message_twice() {
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "DONE" && c.nth <= 2 { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "delivered", |n| n.delivered(0));
    // Two final acks were lost, the third got through. The message is shown once.
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!((net.count(0, "POLL"), net.count(1, "DONE")), (2, 3), "{}", net.trace());
}

#[test]
fn lost_final_ack_with_inline_start() {
    let mut net = Net::new(LinkConfig::default()).rule(|c| if c.frame.name() == "DONE" && c.nth == 1 { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    // The second start is short (without the message), and the receiver recognises
    // the identifier and acks again without showing the message a second time.
    assert_eq!(net.count(0, "START"), 2);
    assert!(matches!(net.sent[0].2, Frame::Start { inline: Some(_), .. }));
    assert!(net.sent.iter().filter(|(_, n, f)| *n == 0 && matches!(f, Frame::Start { inline: None, .. })).count() == 1);
}

#[test]
fn inline_start_lost_then_handshake_takes_over() {
    // The inline start is lost entirely: the second attempt is an ordinary handshake.
    let mut net = Net::new(LinkConfig::default()).rule(|c| if c.frame.name() == "START" && c.nth == 1 { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "A:START", "B:START_ACK", "A:DATA", "B:DONE"]);
}

#[test]
fn inline_no_receiver_still_fails_fast() {
    let mut net = Net::new(LinkConfig::default()).rule(|_| Fate::Vanish);
    net.send(0, LINK);
    net.run_until(15_000, "fails", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
    assert!(net.now < 10_500, "took {} ms", net.now);
}

#[test]
fn both_send_almost_at_the_same_time() {
    // The other presses Send when the first one's start is already in the air or just
    // leaving: the latter yields and sends its own after the reception.
    for offset in [300, 600, 1500, 2500] {
        let mut net = Net::new(handshake());
        net.send(0, b"msg from A");
        net.run(offset);
        net.send(1, b"msg from B, that is longer");
        net.run_until(60_000, "both delivered", |n| n.delivered(0) && n.delivered(1));
        assert_eq!(net.received(1), vec![b"msg from A".to_vec()], "offset {offset}\n{}", net.trace());
        assert_eq!(net.received(0), vec![b"msg from B, that is longer".to_vec()], "offset {offset}\n{}", net.trace());
        net.run(1000);
        assert!(net.idle(), "offset {offset}");
    }
}

#[test]
fn both_send_at_exactly_the_same_time() {
    // The starts collide, neither hears the other. A random wait
    // resolves the situation, but not for certain: if the waits land within about
    // 0.2 s of each other (the preamble detection delay), the collision repeats.
    // The test measures the success rate and requires that failure is clean.
    let mut ok = 0;
    let total = 200;
    for seed in 0..total {
        let mut net = Net::new(handshake());
        net.nodes[0].link = Link::new(handshake(), 1000 + seed);
        net.nodes[1].link = Link::new(handshake(), 5000 + seed * 7);
        net.send(0, b"msg from A");
        net.run((seed % 4) * 50);
        net.send(1, b"msg from B");
        net.run_until(60_000, "resolves", |n| (n.delivered(0) || n.failed(0).is_some()) && (n.delivered(1) || n.failed(1).is_some()));
        if net.delivered(0) && net.delivered(1) {
            assert_eq!(net.received(1), vec![b"msg from A".to_vec()]);
            assert_eq!(net.received(0), vec![b"msg from B".to_vec()]);
            ok += 1;
        } else {
            // Neither caught the other: both report the same reason.
            assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
            assert_eq!(net.failed(1), Some(FailReason::NoReceiver));
        }
        net.run(8000);
        assert!(net.idle(), "seed {seed}");
    }
    assert!(ok * 100 >= total * 93, "only {ok}/{total} resolved");
}

#[test]
fn sender_vanishes_mid_message() {
    let mut net = Net::new(handshake());
    let msg = message(400);
    net.send(0, &msg);
    net.run_until(10_000, "first data frame", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 2, .. })));
    // The sender's machine shuts down in the middle of the burst.
    net.nodes[0].alive = false;
    for f in net.air.iter_mut() {
        f.aborted = true;
    }
    let t0 = net.now;
    net.run_until(10_000, "receiver gives up", |n| n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Timeout })));
    let waited = net.now - t0;
    assert!((5500..=7500).contains(&waited), "gave up after {waited} ms");
    assert!(net.received(1).is_empty());
    assert!(!net.nodes[1].link.is_receiving());
    // The receiver is available again.
    net.nodes[0] = Node { link: Link::new(handshake(), 99), events: Vec::new(), tx_until: None, hears_self: true, alive: true };
    net.send(0, b"new attempt");
    net.run_until(10_000, "new message delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![b"new attempt".to_vec()]);
}

#[test]
fn user_cancels_mid_transfer() {
    let mut net = Net::new(handshake());
    net.send(0, &message(400));
    net.run_until(10_000, "data in progress", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[0].link.cancel(now);
    net.run_until(3000, "cancellation done", |n| n.has(0, |e| *e == Event::Cancelled) && n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Cancelled })));
    assert!(net.received(1).is_empty());
    assert!(!net.delivered(0));
    net.run(1000);
    assert!(net.idle());
    assert_eq!(net.count(0, "CANCEL"), 1);
}

#[test]
fn cancel_before_anything_was_sent_is_silent() {
    let mut net = Net::new(handshake());
    net.send(0, LINK);
    net.nodes[0].link.cancel(0);
    net.run(2000);
    assert!(net.sent.is_empty());
    assert!(net.has(0, |e| *e == Event::Cancelled));
}

#[test]
fn lost_cancel_is_covered_by_receiver_timeout() {
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "CANCEL" { Fate::Vanish } else { Fate::Deliver });
    net.send(0, &message(400));
    net.run_until(10_000, "data in progress", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[0].link.cancel(now);
    net.run_until(9000, "receiver gives up", |n| n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Timeout })));
}

#[test]
fn no_receiver_fails_fast_without_sending_data() {
    let mut net = Net::new(handshake()).rule(|_| Fate::Vanish);
    net.send(0, LINK);
    net.run_until(12_000, "fails", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
    assert_eq!((net.count(0, "START"), net.count(0, "DATA")), (3, 0));
    assert!(net.now < 9000, "took {} ms", net.now);
    assert!(!net.nodes[0].link.is_sending());
}

#[test]
fn loud_noise_in_the_middle_of_transfer() {
    // Everything sent between 2.0 and 3.5 s is corrupted in both directions (speech in the room).
    let mut net = Net::new(handshake()).rule(|c| if (2000..3500).contains(&c.now) { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(500);
    net.send(0, &msg);
    net.run_until(60_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert!(net.count(0, "DATA") > 8, "{}", net.trace());
}

#[test]
fn hopeless_channel_gives_up_with_reason() {
    // The start succeeds, but not a single data frame gets through.
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "DATA" { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(120_000, "fails", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::TooManyRetries));
    // 1 actual transmission + 6 retransmission rounds.
    assert_eq!(net.count(0, "DATA"), 7);
    net.run_until(10_000, "receiver gives up", |n| !n.nodes[1].link.is_receiving());
}

#[test]
fn receiver_stops_answering_mid_transfer() {
    let mut net = Net::new(handshake()).rule(|c| if c.from == 1 && c.frame.name() != "START_ACK" { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(30_000, "fails", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoResponse));
    assert_eq!(net.count(0, "POLL"), 3);
}

#[test]
fn failure_hint_comes_from_receiver_quality_report() {
    let mut l = Link::new(handshake(), 1);
    l.send(0, LINK).unwrap();
    let mut now = 0;
    // Run by hand: START out, START_ACK in with poor quality, then nothing.
    let mut failed = None;
    while failed.is_none() && now < 60_000 {
        now += 10;
        l.tick(now, false);
        while let Some(a) = l.poll_action() {
            match a {
                Action::Transmit(bytes) => {
                    let f = Frame::decode(&bytes).unwrap();
                    let id = f.id();
                    l.on_tx_done(now);
                    if f.name() == "START" {
                        let ack = Frame::StartAck { id, quality: Quality { snr_db: 3, level_db: -70, clipped: false } };
                        l.on_frame(now + 5, &ack.encode(), Quality::default());
                    }
                }
                Action::Event(Event::Failed { reason, hint, .. }) => failed = Some((reason, hint)),
                _ => {}
            }
        }
    }
    assert_eq!(failed, Some((FailReason::NoResponse, Hint::TooQuiet)));
}

#[test]
fn works_without_hearing_own_echo() {
    // Headphones: the machine does not hear its own transmission, so timeouts
    // are anchored to the end of the transmission + a margin.
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "ACK" && c.nth == 1 { Fate::Corrupt } else if matches!(c.frame, Frame::Data { seq: 1, .. }) && c.nth == 2 { Fate::Corrupt } else { Fate::Deliver });
    net.nodes[0].hears_self = false;
    net.nodes[1].hears_self = false;
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
}

#[test]
fn slow_devices_with_long_latency() {
    // 400 ms audio latency each way: the reply starts late, but the wait is long enough.
    let mut net = Net::new(handshake());
    net.latency = 400;
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!(net.count(0, "POLL"), 0, "{}", net.trace());
    assert_eq!(net.count(0, "START"), 1);
}

// ------------------------------------------------------------------ content

#[test]
fn content_umlauts_emoji_empty_and_max() {
    let texts: Vec<Vec<u8>> = vec!["Diacritics: åéü ÅÉÜ €".as_bytes().to_vec(), "emoji 🎵🇫🇮👩‍💻 plus text".as_bytes().to_vec(), Vec::new(), message(512), vec![0u8; 64], vec![0xFF; 65]];
    let mut net = Net::new(handshake());
    for (i, t) in texts.iter().enumerate() {
        net.send(0, t);
        net.run_until(60_000, "delivered", |n| n.received(1).len() == i + 1 && !n.nodes[0].link.is_sending());
        net.run(500);
    }
    assert_eq!(net.received(1), texts);
    // An emoji is split mid-character at a frame boundary, but the assembled message is intact UTF-8.
    assert_eq!(String::from_utf8(net.received(1)[1].clone()).unwrap(), "emoji 🎵🇫🇮👩‍💻 plus text");
}

#[test]
fn too_long_message_is_rejected_up_front() {
    let mut l = Link::new(LinkConfig::default(), 1);
    assert_eq!(l.send(0, &message(513)), Err(SendError::TooLong { len: 513, max: 512 }));
    assert!(l.send(0, &message(512)).is_ok());
    assert_eq!(l.send(0, b"x"), Err(SendError::Busy));
}

#[test]
fn corrupted_frame_passing_checks_triggers_restart() {
    // Simulate a corrupted data frame that got through the physical layer:
    // the assembled message does not match the CRC-32, so everything is requested again.
    let mut net = Net::new(handshake());
    net.send(0, &message(100));
    net.run_until(5000, "start acked", |n| n.count(1, "START_ACK") == 1);
    // Slip a wrong frame 0 to the receiver before the right one.
    let id = net.sent[0].2.id();
    let bad = Frame::Data { id, seq: 0, eob: false, data: vec![0xAA; 64] };
    net.run_until(3000, "right frame 0 delivered", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[1].link.on_frame(now, &bad.encode(), Quality::default());
    net.run_until(30_000, "delivered", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![message(100)]);
    assert!(net.sent.iter().any(|(_, _, f)| matches!(f, Frame::Ack { restart: true, .. })));
}

#[test]
fn stray_frames_are_ignored() {
    let mut l = Link::new(LinkConfig::default(), 1);
    for f in [Frame::Data { id: 5, seq: 0, eob: true, data: vec![1] }, Frame::Ack { id: 5, quality: Quality::default(), restart: false, bitmap: vec![0xFF] }, Frame::Done { id: 5, crc: 1 }, Frame::Poll { id: 5 }, Frame::Cancel { id: 5 }, Frame::StartAck { id: 5, quality: Quality::default() }] {
        l.on_frame(100, &f.encode(), Quality::default());
    }
    l.on_frame(100, &[0xF0, 1, 2, 3], Quality::default());
    l.on_frame(100, &[], Quality::default());
    for t in 0..100 {
        l.tick(100 + t * 10, false);
    }
    while let Some(a) = l.poll_action() {
        assert!(matches!(a, Action::Event(Event::Log(_))), "{a:?}");
    }
}
