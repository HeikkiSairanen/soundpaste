//! Linkkikerroksen testit: kaksi tilakonetta ja simuloitu "ilma" kehystasolla.
//!
//! Ilma mallintaa sen, mikä linkkikerrokselle näkyy äänestä:
//! - kehyksellä on kesto, ja se saapuu perille keston ja viiveen jälkeen
//! - lähettävä kone ei kuule toista (vuorosuuntaisuus), ja päällekkäiset
//!   kehykset tuhoavat toisensa
//! - kone kuulee oman kehyksensä kaikuna
//! - testi voi vioittaa tai kadottaa valittuja kehyksiä
//!
//! Jokaisella DESIGN.md:n luvun 5.6 poikkeustilanteella on oma testinsä.

use super::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fate {
    Deliver,
    /// Alkumerkki kuuluu, mutta kehys ei purkaudu.
    Corrupt,
    /// Mitään ei kuulu.
    Vanish,
}

struct Ctx<'a> {
    from: usize,
    frame: &'a Frame,
    /// Monesko tämän tyyppinen kehys tältä koneelta (1 = ensimmäinen).
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
    /// Kaikki lähetetyt kehykset: (hetki, lähettäjä, kehys).
    sent: Vec<(Ms, usize, Frame)>,
}

const STEP: Ms = 5;
/// Alkumerkki havaitaan tämän verran kehyksen alun jälkeen.
const DETECT: Ms = 150;

fn duration(len: usize) -> Ms {
    // Suunnilleen v1-aaltomuodon kesto: 240 ms + 50 ms per 2,7 tavua, 5 ms:n tarkkuudella.
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
            // Mitä tämä kone kuulee juuri nyt?
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
                // Oma lähetys tai toinen päällekkäinen kehys peittää tämän.
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
                        let frame = Frame::decode(&bytes).expect("linkki lähetti kelvottoman kehyksen");
                        assert!(node.tx_until.is_none(), "kone {i} lähetti kehyksen edellisen päälle");
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

    /// Aja, kunnes ehto täyttyy. Paniikki, jos aikaraja ylittyy.
    fn run_until(&mut self, limit: Ms, what: &str, cond: impl Fn(&Net) -> bool) {
        let until = self.now + limit;
        while !cond(self) {
            assert!(self.now < until, "aikaraja ylittyi: {what}\n{}", self.trace());
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

/// Testien oletus: erillinen kättely, jotta START_ACK- ja DATA-polut ovat mukana
/// myös lyhyillä viesteillä. Pikalähetyksellä on omat testinsä.
fn handshake() -> LinkConfig {
    LinkConfig { inline_start: false, ..LinkConfig::default() }
}

const LINK: &[u8] = b"https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd";

fn message(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7) as u8).collect()
}

// ------------------------------------------------------------------ normaali sessio

#[test]
fn normal_session_short_message() {
    let mut net = Net::new(handshake());
    net.send(0, LINK);
    net.run_until(10_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "B:START_ACK", "A:DATA", "B:DONE"]);
    assert!(net.has(1, |e| matches!(e, Event::RxStarted { len: 60, .. })));
    net.run(1000);
    assert!(net.idle());
    // Lähettäjä ja vastaanottaja palaavat lepoon ja voivat vaihtaa rooleja.
    net.send(1, b"takaisin");
    net.run_until(10_000, "toinen suunta", |n| n.delivered(1));
    assert_eq!(net.received(0), vec![b"takaisin".to_vec()]);
}

#[test]
fn long_message_goes_in_one_burst() {
    let mut net = Net::new(handshake());
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!(net.count(0, "DATA"), 5);
    assert_eq!(net.count(1, "ACK"), 0);
    assert_eq!(net.count(1, "DONE"), 1);
    // Edistyminen näkyy vastaanottajalle kehys kerrallaan.
    let progress: Vec<u32> = net.nodes[1].events.iter().filter_map(|e| if let Event::RxProgress { have, .. } = e { Some(*have) } else { None }).collect();
    assert_eq!(progress, [1, 2, 3, 4, 5]);
}

#[test]
fn per_frame_ack_mode() {
    let cfg = LinkConfig { burst: 1, ..handshake() };
    let mut net = Net::new(cfg);
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(0, "DATA"), net.count(1, "ACK"), net.count(1, "DONE")), (4, 3, 1));
    assert!(!net.has(0, |e| matches!(e, Event::Tx(TxStatus::Retrying { .. }))));
}

#[test]
fn inline_start_delivers_in_two_frames() {
    let mut net = Net::new(LinkConfig::default());
    net.send(0, LINK);
    net.run_until(10_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "B:DONE"]);
    // Pidempi viesti käyttää tavallista kättelyä.
    let msg = message(100);
    net.send(0, &msg);
    net.run_until(20_000, "pitkä perille", |n| n.received(1).len() == 2);
    assert_eq!(net.count(1, "START_ACK"), 1);
}

// ------------------------------------------------------------------ poikkeustilanteet

#[test]
fn lost_data_frame_is_retransmitted_alone() {
    let mut net = Net::new(handshake()).rule(|c| if c.from == 0 && matches!(c.frame, Frame::Data { seq: 2, .. }) && c.nth <= 3 { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    // Viisi kehystä + yksi uudelleenlähetys, yksi koontikuittaus.
    assert_eq!((net.count(0, "DATA"), net.count(1, "ACK")), (6, 1));
    assert!(net.has(0, |e| matches!(e, Event::Tx(TxStatus::Retrying { missing: 1 }))));
}

#[test]
fn lost_last_frame_of_burst_is_acked_after_silence() {
    // Ryöpyn viimeinen kehys (jossa on loppumerkki) vioittuu: vastaanottaja
    // kuittaa silti, kun kanava hiljenee.
    let mut net = Net::new(handshake()).rule(|c| if matches!(c.frame, Frame::Data { eob: true, .. }) && c.nth == 5 { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(300);
    net.send(0, &msg);
    net.run_until(30_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(1, "ACK"), net.count(0, "POLL")), (1, 0), "{}", net.trace());
}

#[test]
fn lost_ack_is_requested_again() {
    let mut net = Net::new(handshake())
        .rule(|c| if (matches!(c.frame, Frame::Data { seq: 0, .. }) && c.nth == 1) || (c.frame.name() == "ACK" && c.nth == 1) { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(150);
    net.send(0, &msg);
    net.run_until(30_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!((net.count(0, "POLL"), net.count(1, "ACK")), (1, 2), "{}", net.trace());
    assert!(net.has(0, |e| matches!(e, Event::Tx(TxStatus::Polling { attempt: 1, .. }))));
}

#[test]
fn lost_start_ack_restarts_handshake_without_restarting_reception() {
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "START_ACK" && c.nth == 1 { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(15_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!((net.count(0, "START"), net.count(1, "START_ACK")), (2, 2));
    let starts = net.nodes[1].events.iter().filter(|e| matches!(e, Event::RxStarted { .. })).count();
    assert_eq!(starts, 1);
}

#[test]
fn lost_final_ack_does_not_show_message_twice() {
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "DONE" && c.nth <= 2 { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "perille", |n| n.delivered(0));
    // Kaksi loppukuittausta katosi, kolmas meni perille. Viesti näkyy kerran.
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!((net.count(0, "POLL"), net.count(1, "DONE")), (2, 3), "{}", net.trace());
}

#[test]
fn lost_final_ack_with_inline_start() {
    let mut net = Net::new(LinkConfig::default()).rule(|c| if c.frame.name() == "DONE" && c.nth == 1 { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    // Toinen aloitus on lyhyt (ilman viestiä), ja vastaanottaja tunnistaa
    // tunnisteen ja kuittaa uudelleen näyttämättä viestiä toista kertaa.
    assert_eq!(net.count(0, "START"), 2);
    assert!(matches!(net.sent[0].2, Frame::Start { inline: Some(_), .. }));
    assert!(net.sent.iter().filter(|(_, n, f)| *n == 0 && matches!(f, Frame::Start { inline: None, .. })).count() == 1);
}

#[test]
fn inline_start_lost_then_handshake_takes_over() {
    // Pikalähetys katoaa kokonaan: toinen yritys on tavallinen kättely.
    let mut net = Net::new(LinkConfig::default()).rule(|c| if c.frame.name() == "START" && c.nth == 1 { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(20_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![LINK.to_vec()]);
    assert_eq!(net.names(), ["A:START", "A:START", "B:START_ACK", "A:DATA", "B:DONE"]);
}

#[test]
fn inline_no_receiver_still_fails_fast() {
    let mut net = Net::new(LinkConfig::default()).rule(|_| Fate::Vanish);
    net.send(0, LINK);
    net.run_until(15_000, "epäonnistuu", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
    assert!(net.now < 10_500, "kesti {} ms", net.now);
}

#[test]
fn both_send_almost_at_the_same_time() {
    // Toinen painaa Lähetä, kun ensimmäisen aloitus on jo ilmassa tai juuri
    // lähdössä: jälkimmäinen väistää ja lähettää omansa vastaanoton jälkeen.
    for offset in [300, 600, 1500, 2500] {
        let mut net = Net::new(handshake());
        net.send(0, b"A:n viesti");
        net.run(offset);
        net.send(1, b"B:n viesti, hieman pidempi");
        net.run_until(60_000, "molemmat perille", |n| n.delivered(0) && n.delivered(1));
        assert_eq!(net.received(1), vec![b"A:n viesti".to_vec()], "offset {offset}\n{}", net.trace());
        assert_eq!(net.received(0), vec![b"B:n viesti, hieman pidempi".to_vec()], "offset {offset}\n{}", net.trace());
        net.run(1000);
        assert!(net.idle(), "offset {offset}");
    }
}

#[test]
fn both_send_at_exactly_the_same_time() {
    // Aloitukset törmäävät, kumpikaan ei kuule toista. Satunnainen odotus
    // ratkaisee tilanteen, mutta ei varmasti: jos odotukset osuvat alle noin
    // 0,2 s:n päähän toisistaan (alkumerkin havaitsemisviive), törmäys toistuu.
    // Testi mittaa onnistumisosuuden ja vaatii, että epäonnistuminen on siisti.
    let mut ok = 0;
    let total = 200;
    for seed in 0..total {
        let mut net = Net::new(handshake());
        net.nodes[0].link = Link::new(handshake(), 1000 + seed);
        net.nodes[1].link = Link::new(handshake(), 5000 + seed * 7);
        net.send(0, b"A:n viesti");
        net.run((seed % 4) * 50);
        net.send(1, b"B:n viesti");
        net.run_until(60_000, "ratkeaa", |n| (n.delivered(0) || n.failed(0).is_some()) && (n.delivered(1) || n.failed(1).is_some()));
        if net.delivered(0) && net.delivered(1) {
            assert_eq!(net.received(1), vec![b"A:n viesti".to_vec()]);
            assert_eq!(net.received(0), vec![b"B:n viesti".to_vec()]);
            ok += 1;
        } else {
            // Kumpikaan ei saanut toista kiinni: molemmat raportoivat saman syyn.
            assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
            assert_eq!(net.failed(1), Some(FailReason::NoReceiver));
        }
        net.run(8000);
        assert!(net.idle(), "siemen {seed}");
    }
    assert!(ok * 100 >= total * 93, "vain {ok}/{total} ratkesi");
}

#[test]
fn sender_vanishes_mid_message() {
    let mut net = Net::new(handshake());
    let msg = message(400);
    net.send(0, &msg);
    net.run_until(10_000, "ensimmäinen datakehys", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 2, .. })));
    // Lähettäjän kone sammuu kesken ryöpyn.
    net.nodes[0].alive = false;
    for f in net.air.iter_mut() {
        f.aborted = true;
    }
    let t0 = net.now;
    net.run_until(10_000, "vastaanottaja luopuu", |n| n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Timeout })));
    let waited = net.now - t0;
    assert!((5500..=7500).contains(&waited), "luopui {waited} ms kuluttua");
    assert!(net.received(1).is_empty());
    assert!(!net.nodes[1].link.is_receiving());
    // Vastaanottaja on taas käytettävissä.
    net.nodes[0] = Node { link: Link::new(handshake(), 99), events: Vec::new(), tx_until: None, hears_self: true, alive: true };
    net.send(0, b"uusi yritys");
    net.run_until(10_000, "uusi viesti perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![b"uusi yritys".to_vec()]);
}

#[test]
fn user_cancels_mid_transfer() {
    let mut net = Net::new(handshake());
    net.send(0, &message(400));
    net.run_until(10_000, "data käynnissä", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[0].link.cancel(now);
    net.run_until(3000, "peruutus valmis", |n| n.has(0, |e| *e == Event::Cancelled) && n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Cancelled })));
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
    net.run_until(10_000, "data käynnissä", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[0].link.cancel(now);
    net.run_until(9000, "vastaanottaja luopuu", |n| n.has(1, |e| matches!(e, Event::RxAborted { reason: AbortReason::Timeout })));
}

#[test]
fn no_receiver_fails_fast_without_sending_data() {
    let mut net = Net::new(handshake()).rule(|_| Fate::Vanish);
    net.send(0, LINK);
    net.run_until(12_000, "epäonnistuu", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoReceiver));
    assert_eq!((net.count(0, "START"), net.count(0, "DATA")), (3, 0));
    assert!(net.now < 9000, "kesti {} ms", net.now);
    assert!(!net.nodes[0].link.is_sending());
}

#[test]
fn loud_noise_in_the_middle_of_transfer() {
    // Kaikki 2,0–3,5 s välillä lähetetty vioittuu molempiin suuntiin (puhetta huoneessa).
    let mut net = Net::new(handshake()).rule(|c| if (2000..3500).contains(&c.now) { Fate::Corrupt } else { Fate::Deliver });
    let msg = message(500);
    net.send(0, &msg);
    net.run_until(60_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert!(net.count(0, "DATA") > 8, "{}", net.trace());
}

#[test]
fn hopeless_channel_gives_up_with_reason() {
    // Aloitus onnistuu, mutta yksikään datakehys ei mene läpi.
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "DATA" { Fate::Corrupt } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(120_000, "epäonnistuu", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::TooManyRetries));
    // 1 varsinainen lähetys + 6 uudelleenlähetyskierrosta.
    assert_eq!(net.count(0, "DATA"), 7);
    net.run_until(10_000, "vastaanottaja luopuu", |n| !n.nodes[1].link.is_receiving());
}

#[test]
fn receiver_stops_answering_mid_transfer() {
    let mut net = Net::new(handshake()).rule(|c| if c.from == 1 && c.frame.name() != "START_ACK" { Fate::Vanish } else { Fate::Deliver });
    net.send(0, LINK);
    net.run_until(30_000, "epäonnistuu", |n| n.failed(0).is_some());
    assert_eq!(net.failed(0), Some(FailReason::NoResponse));
    assert_eq!(net.count(0, "POLL"), 3);
}

#[test]
fn failure_hint_comes_from_receiver_quality_report() {
    let mut l = Link::new(handshake(), 1);
    l.send(0, LINK).unwrap();
    let mut now = 0;
    // Aja käsin: START ulos, START_ACK sisään huonolla laadulla, sitten ei mitään.
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
    // Kuulokkeet: kone ei kuule omaa lähetystään, joten aikakatkaisut
    // ankkuroituvat lähetyksen loppuun + varaan.
    let mut net = Net::new(handshake()).rule(|c| if c.frame.name() == "ACK" && c.nth == 1 { Fate::Corrupt } else if matches!(c.frame, Frame::Data { seq: 1, .. }) && c.nth == 2 { Fate::Corrupt } else { Fate::Deliver });
    net.nodes[0].hears_self = false;
    net.nodes[1].hears_self = false;
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
}

#[test]
fn slow_devices_with_long_latency() {
    // 400 ms:n ääniviive suuntaansa: vastaus alkaa myöhään, mutta odotus riittää.
    let mut net = Net::new(handshake());
    net.latency = 400;
    let msg = message(200);
    net.send(0, &msg);
    net.run_until(40_000, "perille", |n| n.delivered(0));
    assert_eq!(net.received(1), vec![msg]);
    assert_eq!(net.count(0, "POLL"), 0, "{}", net.trace());
    assert_eq!(net.count(0, "START"), 1);
}

// ------------------------------------------------------------------ sisältö

#[test]
fn content_umlauts_emoji_empty_and_max() {
    let texts: Vec<Vec<u8>> = vec!["Ääkköset: åäö ÅÄÖ €".as_bytes().to_vec(), "emoji 🎵🇫🇮👩‍💻 ja teksti".as_bytes().to_vec(), Vec::new(), message(512), vec![0u8; 64], vec![0xFF; 65]];
    let mut net = Net::new(handshake());
    for (i, t) in texts.iter().enumerate() {
        net.send(0, t);
        net.run_until(60_000, "perille", |n| n.received(1).len() == i + 1 && !n.nodes[0].link.is_sending());
        net.run(500);
    }
    assert_eq!(net.received(1), texts);
    // Emoji katkeaa kehysrajalla keskeltä merkkiä, mutta koottu viesti on ehjää UTF-8:aa.
    assert_eq!(String::from_utf8(net.received(1)[1].clone()).unwrap(), "emoji 🎵🇫🇮👩‍💻 ja teksti");
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
    // Simuloidaan fyysisen kerroksen läpi päässyt virheellinen datakehys:
    // koottu viesti ei täsmää CRC-32:een, joten kaikki pyydetään uudelleen.
    let mut net = Net::new(handshake());
    net.send(0, &message(100));
    net.run_until(5000, "aloitus kuitattu", |n| n.count(1, "START_ACK") == 1);
    // Ujutetaan väärä kehys 0 vastaanottajalle ennen oikeaa.
    let id = net.sent[0].2.id();
    let bad = Frame::Data { id, seq: 0, eob: false, data: vec![0xAA; 64] };
    net.run_until(3000, "oikea kehys 0 perillä", |n| n.has(1, |e| matches!(e, Event::RxProgress { have: 1, .. })));
    let now = net.now;
    net.nodes[1].link.on_frame(now, &bad.encode(), Quality::default());
    net.run_until(30_000, "perille", |n| n.delivered(0));
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
