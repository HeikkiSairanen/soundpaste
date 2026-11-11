//! Linkkikerros: viestin siirto kehyksinä, kuittaukset ja uudelleenlähetykset.
//!
//! [`Link`] on tilakone, joka ei tiedä äänestä mitään. Sille kerrotaan, mitä
//! tapahtui (kehys vastaanotettu, alkumerkki havaittu, oma lähetys soitettu
//! loppuun, kello eteni), ja se kertoo, mitä pitää tehdä ([`Action`]). Aika
//! annetaan ulkoa millisekunteina, joten testit ovat deterministisiä.
//!
//! Sama tilakone hoitaa molemmat roolit: se voi lähettää yhtä viestiä
//! (`outgoing`) ja vastaanottaa toista (`incoming`). Protokolla: DESIGN.md luku 5.

pub mod frame;

use crate::crc::crc32;
use crate::rng::Rng;
pub use frame::{Frame, Quality};
use std::collections::VecDeque;

pub type Ms = u64;
const NEVER: Ms = Ms::MAX;

#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// Dataa per kehys (tavua).
    pub chunk: usize,
    /// Viestin enimmäispituus (tavua).
    pub max_message: usize,
    /// Datakehyksiä per ryöppy ennen kuittausta. 0 = kaikki puuttuvat kerralla,
    /// 1 = kehyskohtainen kuittaus.
    pub burst: usize,
    /// Yhteen kehykseen mahtuva viesti lähetetään ensimmäisen aloituskehyksen
    /// mukana ("pikalähetys"). Jos siihen ei vastata, uudet yritykset ovat lyhyitä
    /// aloituskehyksiä, jotta puuttuva vastaanottaja selviää nopeasti.
    pub inline_start: bool,
    /// Vastaaja odottaa tämän verran vastaanotetun kehyksen lopusta (kaiun suoja-aika).
    pub response_delay: Ms,
    /// Kauanko vastausta odotetaan oman kehyksen lopusta (mikrofonissa kuultuna).
    pub reply_timeout: Ms,
    /// Jos omaa kehystä ei kuulu, sen oletetaan päättyneen tämän ajan kuluttua
    /// lähetyksen lopusta (laitteen ääniviiveen yläraja).
    pub echo_fallback: Ms,
    pub start_tries: u32,
    pub poll_tries: u32,
    /// Uudelleenlähetyskierroksia enintään (varsinaisen lähetyksen lisäksi).
    pub max_rounds: u32,
    /// Vastaanottaja luopuu osittaisesta viestistä, kun lähettäjästä ei ole
    /// kuulunut tähän aikaan.
    pub rx_timeout: Ms,
    /// Kanavan pitää olla hiljaa tämän verran ennen aloitusta.
    pub listen: Ms,
    /// Satunnainen odotus, kun kanava oli varattu tai aloitus jäi vastaamatta.
    pub backoff: (Ms, Ms),
    /// Tauko ryöpyn kehysten välissä.
    pub frame_gap: Ms,
    /// Vastaanottaja kuittaa, kun ryöpyn jälkeen on ollut näin kauan hiljaista
    /// (kattaa tapauksen, jossa ryöpyn viimeinen kehys katosi).
    pub quiet_ack: Ms,
    /// Valmiiden viestien muisti kaksoiskappaleiden tunnistamiseen.
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

/// Lähetyksen tila käyttöliittymälle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxStatus {
    /// Kuunnellaan, että kanava on vapaa.
    Listening,
    /// Etsitään vastaanottajaa (aloituskehys lähetetty).
    Starting { attempt: u32, of: u32 },
    /// Lähetetään datakehystä.
    Sending { frame: u32, of: u32, round: u32 },
    WaitingAck,
    /// Kuittausta ei kuulunut, pyydetään uudelleen.
    Polling { attempt: u32, of: u32 },
    /// Vastaanottajalta puuttuu kehyksiä, lähetetään ne uudelleen.
    Retrying { missing: u32 },
    /// Toinen kone ehti aloittaa ensin: oma viesti lähtee vastaanoton jälkeen.
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    /// Kukaan ei vastannut aloitukseen.
    NoReceiver,
    /// Vastaanottaja lakkasi vastaamasta kesken siirron.
    NoResponse,
    /// Uudelleenlähetyskierrokset loppuivat.
    TooManyRetries,
    /// Vastaanottaja kuittasi eri sisällön kuin lähetettiin.
    Mismatch,
}

/// Käytännön neuvo epäonnistumisen syystä, johdettu vastaanottajan laaturaportista.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    None,
    TooQuiet,
    Clipping,
    Noisy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortReason {
    /// Lähettäjästä ei kuulunut enää.
    Timeout,
    /// Lähettäjä peruutti.
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
    /// Vianetsintäloki.
    Log(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Lähetä tämä kehys fyysisen kerroksen läpi.
    Transmit(Vec<u8>),
    /// Keskeytä käynnissä oleva lähetys heti.
    AbortTx,
    Event(Event),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// Edellinen lähetys on vielä kesken.
    Busy,
    TooLong { len: usize, max: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Listen,
    /// Väistetään saapuvaa viestiä.
    Held,
    /// START lähetetty, odotetaan vastausta.
    StartWait,
    /// Lähetetään ryöppyä; seuraava kehys tällä hetkellä.
    Burst { next_at: Ms },
    /// Ryöppy (tai POLL) lähetetty, odotetaan kuittausta.
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
    /// Vastaanottajan roolissa lähetettävä vastaus ja sen lähetyshetki.
    reply: Option<(Ms, Frame)>,
    /// Valmiit viestit: (tunniste, CRC-32, pituus, valmistumishetki).
    done: VecDeque<(u16, u32, usize, Ms)>,
    actions: VecDeque<Action>,
    in_flight: bool,
    last_sent: Vec<u8>,
    /// Vastauksen takaraja lähettäjän roolissa.
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
        self.log(format!("-> {} id={:04x} ({} tavua)", f.name(), f.id(), bytes.len()));
        self.last_sent = bytes.clone();
        self.in_flight = true;
        self.deadline = NEVER;
        self.actions.push_back(Action::Transmit(bytes));
    }

    // ---------------------------------------------------------------- käyttäjän toimet

    /// Aloita viestin lähetys.
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

    /// Peruuta käynnissä oleva lähetys.
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
            // Kerrotaan vastaanottajalle, jotta se ei jää odottamaan.
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

    // ---------------------------------------------------------------- fyysisen kerroksen tapahtumat

    fn activity(&mut self, now: Ms) {
        self.last_activity = self.last_activity.max(now);
        // Kanava oli varattu kuuntelun aikana: odotetaan satunnainen aika sen vapauduttua.
        if matches!(self.outgoing.as_ref().map(|o| (o.phase, o.backoff)), Some((Phase::Listen, None))) {
            let r = self.random_backoff();
            self.outgoing.as_mut().unwrap().backoff = Some(r);
        }
    }

    /// Alkumerkki havaittu. Lasketaan vain kanavan varaukseksi: pelkkä alkumerkki
    /// voi olla väärä hälytys (naksahdus, puhe), joten se ei laukaise kuittausta.
    pub fn on_preamble(&mut self, now: Ms) {
        self.activity(now);
    }

    /// Kehyksen otsake purkautui, mutta hyötykuorma ei: joku lähettää oikeasti.
    /// Vastaanoton aikana se tarkoittaa, että ryöppy on käynnissä. Oman
    /// lähetyksen kaiku suljetaan pois ajan perusteella.
    pub fn on_rx_failed(&mut self, now: Ms) {
        self.activity(now);
        if !self.in_flight && now >= self.last_tx_done + self.cfg.echo_fallback {
            if let Some(inc) = self.incoming.as_mut() {
                inc.burst_seen = true;
            }
        }
    }

    /// Oma lähetys on soitettu loppuun (äänen lähtöpuskurin aikajanalla).
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

    /// Ehjä kehys vastaanotettu. `now` on kehyksen loppuhetki mikrofonin aikajanalla.
    pub fn on_frame(&mut self, now: Ms, bytes: &[u8], quality: Quality) {
        self.activity(now);
        let Some(f) = Frame::decode(bytes) else {
            self.log(format!("<- tuntematon kehys ({} tavua)", bytes.len()));
            return;
        };
        let id = f.id();
        let as_sender = self.outgoing.as_ref().map_or(false, |o| o.id == id);
        let as_receiver = self.incoming.as_ref().map_or(false, |i| i.id == id) || self.done.iter().any(|d| d.0 == id);

        // Oma kaiku: kehystyyppi, jonka vain minä tässä roolissa lähetän.
        if (f.from_sender() && as_sender) || (!f.from_sender() && !as_sender && as_receiver) {
            if bytes == self.last_sent.as_slice() && !self.in_flight && self.deadline != NEVER {
                // Tarkka ankkuri: ääni lakkasi juuri nyt.
                self.deadline = now + self.cfg.reply_timeout;
            }
            self.log(format!("<- {} id={id:04x} (oma kaiku)", f.name()));
            return;
        }
        self.log(format!("<- {} id={id:04x} snr={} dB taso={} dBFS", f.name(), quality.snr_db, quality.level_db));
        // Muu kuin datakehys tarkoittaa, ettei ryöppy ole kesken: sen alkumerkki ei
        // saa laukaista hiljaisuuteen perustuvaa kuittausta.
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

    // ---------------------------------------------------------------- vastaanottajan rooli

    #[allow(clippy::too_many_arguments)]
    fn rx_start(&mut self, now: Ms, id: u16, len: usize, frames: usize, chunk: usize, crc: u32, inline: Option<Vec<u8>>, q: Quality) {
        // Jo valmis viesti: loppukuittaus katosi. Kuitataan uudelleen, ei näytetä toista kertaa.
        if self.done.iter().any(|d| d.0 == id && d.1 == crc && d.2 == len) {
            self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc }));
            return;
        }
        if let Some(inc) = self.incoming.as_mut() {
            if inc.id == id {
                // Aloituksen kuittaus katosi: vastataan uudelleen aloittamatta alusta.
                inc.last_valid = now;
                self.reply = Some((now + self.cfg.response_delay, Frame::StartAck { id, quality: q }));
            }
            return; // Vieras aloitus kesken toisen vastaanoton: ohitetaan.
        }
        if len > self.cfg.max_message || chunk == 0 || (inline.is_none() && frames != len.div_ceil(chunk)) {
            self.log(format!("aloitus hylätty: pituus {len}, kehyksiä {frames}, datakoko {chunk}"));
            return;
        }
        // Yhtäaikainen lähetys: väistetäänkö?
        if let Some(o) = self.outgoing.as_mut() {
            let yield_now = match o.phase {
                Phase::Listen | Phase::Held => true,
                // Molemmat ehtivät lähettää aloituksen: pienempi tunniste pitää vuoron.
                Phase::StartWait => id < o.id,
                _ => false,
            };
            if !yield_now {
                self.log("vieras aloitus ohitettu: oma lähetys on kesken".into());
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
                self.log("aloituskehyksen mukana tullut viesti ei täsmää".into());
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
            // Viesti on jo valmis, mutta lähettäjä ei tiedä sitä.
            if eob {
                self.reply = Some((now + self.cfg.response_delay, Frame::Done { id, crc: d.1 }));
            }
        }
    }

    /// Kuittaus nykyisestä tilanteesta: DONE, jos viesti on koossa ja ehjä.
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
            // Jokin kehys läpäisi tarkistuksensa virheellisenä: aloitetaan alusta.
            inc.parts.iter_mut().for_each(|p| *p = None);
            self.log("koottu viesti ei täsmää tarkistussummaan, pyydetään kaikki uudelleen".into());
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

    /// Viesti on perillä: näytä se, muista tunniste ja kuittaa.
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
        // Väistetty oma viesti pääsee nyt vuoroon.
        if let Some(o) = self.outgoing.as_mut().filter(|o| o.phase == Phase::Held) {
            o.phase = Phase::Listen;
            o.not_before = now;
            o.backoff = None;
            self.event(Event::Tx(TxStatus::Listening));
        }
    }

    // ---------------------------------------------------------------- lähettäjän rooli

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
            // Kaikki perillä, mutta loppukuittausta ei tullut: pyydetään se.
            self.deadline = now;
            return;
        }
        let first_pass = if per_burst == 0 { 1 } else { o.n.div_ceil(per_burst) as u32 };
        if o.bursts >= first_pass + max_rounds {
            self.fail(FailReason::TooManyRetries);
            return;
        }
        // Ensimmäisen läpikäynnin aikana (kehyskohtainen kuittaus) puuttuvat
        // kehykset ovat vielä lähettämättä, eivät kadonneita.
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

    // ---------------------------------------------------------------- kello

    /// Kello eteni. `busy`: fyysinen kerros vastaanottaa parhaillaan kehystä.
    pub fn tick(&mut self, now: Ms, busy: bool) {
        if busy {
            self.activity(now);
        }
        while self.done.front().map_or(false, |d| now.saturating_sub(d.3) > self.cfg.done_ttl) {
            self.done.pop_front();
        }

        // Vastaanottaja: ajastettu vastaus.
        if !self.in_flight {
            if let Some((at, _)) = self.reply {
                if now >= at {
                    let (_, f) = self.reply.take().unwrap();
                    self.transmit(f);
                }
            }
        }

        // Vastaanottaja: ryöppy loppui ilman "viimeinen kehys" -lippua.
        if let Some(inc) = self.incoming.as_mut() {
            if inc.burst_seen && !busy && !self.in_flight && self.reply.is_none() && now.saturating_sub(self.last_activity) >= self.cfg.quiet_ack {
                inc.burst_seen = false;
                let reply = self.make_ack(now, Quality::default());
                self.log("ryöppy päättyi ilman loppumerkkiä, kuitataan".into());
                self.reply = Some((now, reply));
            }
        }

        // Vastaanottaja: lähettäjä katosi.
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
                // Jos vastaus on parhaillaan tulossa, odotetaan se loppuun.
                if self.in_flight || busy || now < self.deadline {
                    return;
                }
                if o.phase == Phase::StartWait {
                    if o.start_attempts >= self.cfg.start_tries {
                        self.fail(FailReason::NoReceiver);
                    } else {
                        // Ehkä törmäys toisen aloituksen kanssa: satunnainen odotus rikkoo symmetrian.
                        let r = self.random_backoff();
                        let o = self.outgoing.as_mut().unwrap();
                        o.phase = Phase::Listen;
                        o.not_before = now + r;
                        o.backoff = None;
                        self.deadline = NEVER;
                        self.log(format!("aloitukseen ei vastattu, uusi yritys {r} ms kuluttua"));
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
