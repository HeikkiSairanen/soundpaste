//! Kehityspalvelin (vain kehityskäyttöön, ei osa julkaistavaa sivua).
//!
//! - tarjoilee sivun lähiverkkoon ja lisää siihen testiaseman koodin (`dev/dev.js`)
//! - kertoo versiotunnisteen, jotta testiasema huomaa uuden wasm-moduulin tai sivun
//! - ottaa vastaan testiaseman lokit ja mikrofoniäänitteet
//! - välittää komennot (`soundpaste remote ...`) testiasemalle
//!
//! Toteutus on tarkoituksella pieni: std::net, säie per yhteys, ei riippuvuuksia.

use crate::args::Args;
use crate::bench;
use crate::corpus;
use crate::json::Json;
use modem_core::measure::{self, Sweep};
use modem_core::phy::PhyKind;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

const WASM_PATH: &str = "target/wasm32-unknown-unknown/wasm/soundpaste_wasm.wasm";
const DEV_DATA: &str = "devdata";
const MAX_BODY: usize = 64 << 20;

#[derive(Default)]
struct Shared {
    /// Komentojono testiasemille: (järjestysnumero, rivi).
    commands: Vec<(u64, String)>,
    /// Montako seuraavaa sessioäänitettä kopioidaan korpukseen.
    collect: u32,
    /// Asemien viimeisin elonmerkki: (kellonaika, kuvaus).
    seen: HashMap<String, (String, String)>,
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 3 <= b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 65536 {
            return None;
        }
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let length: usize = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.trim().parse().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return None;
    }
    let mut body = buf[header_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(length);
    let (path, query_str) = target.split_once('?').unwrap_or((&target, ""));
    let query = query_str.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).map(|(k, v)| (percent_decode(k), percent_decode(v))).collect();
    Some(Request { method, path: path.to_string(), query, body })
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Sivun tiedostot: (osoite, tiedosto levyllä).
const PAGE_FILES: &[(&str, &str)] = &[("/index.html", "web/index.html"), ("/style.css", "web/style.css"), ("/app.js", "web/app.js"), ("/worklet.js", "web/worklet.js"), ("/dev/dev.js", "dev/dev.js")];

fn version() -> (u32, u32) {
    let mut page = Vec::new();
    for (_, file) in PAGE_FILES {
        page.extend(std::fs::read(file).unwrap_or_default());
    }
    (modem_core::crc::crc32(&page), modem_core::crc::crc32(&std::fs::read(WASM_PATH).unwrap_or_default()))
}

fn clean_id(s: &str) -> String {
    let id: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect::<String>().to_lowercase();
    if id.is_empty() {
        "asema".into()
    } else {
        id
    }
}

fn clock() -> String {
    let t = bench::utc_now();
    if t.len() >= 15 {
        format!("{}:{}:{}", &t[9..11], &t[11..13], &t[13..15])
    } else {
        t
    }
}

fn save_recording(req: &Request, shared: &Mutex<Shared>) -> Result<String, String> {
    let station = clean_id(req.query.get("asema").map_or("", |s| s));
    let kind = req.query.get("tyyppi").map_or("muu", |s| s.as_str());
    let w = modem_core::wav::decode(&req.body).map_err(|e| e.to_string())?;
    let secs = w.samples.len() as f64 / w.sample_rate as f64;
    let device = req.query.get("laite").cloned().unwrap_or_default();
    let note = req.query.get("huom").cloned().unwrap_or_default();
    let now = bench::utc_now();
    match kind {
        "kanava" => {
            // Mac soitti pyyhkäisyn ja testiasema äänitti: kanava Macista asemalle.
            let gain: f32 = req.query.get("gain").and_then(|g| g.parse().ok()).unwrap_or(0.3);
            let sweep = Sweep { amplitude: gain, ..Sweep::default() };
            let m = measure::analyze(&w.samples, w.sample_rate as f64, &sweep)?;
            let id = format!("{now}-mac-{station}-g{:03.0}", gain * 100.0);
            println!("[{}] {station}: kanavamittaus Mac -> {station} ({secs:.1} s)", clock());
            crate::acoustic::print_measurement(&m);
            let holdout = req.query.get("vertailu").map_or(false, |v| v == "1");
            crate::acoustic::store_channel(
                &id,
                w.sample_rate,
                &w.samples,
                &sweep,
                &m,
                vec![
                    ("id", Json::str(&id)),
                    ("kind", Json::str("kanava")),
                    ("time_utc", Json::str(&now)),
                    ("device", Json::str(&format!("kaiutin: Mac; mikrofoni: {station} ({device})"))),
                    ("sample_rate", Json::num(w.sample_rate)),
                    ("gain", Json::num(gain)),
                    ("distance_m", Json::opt(req.query.get("etaisyys").and_then(|d| d.parse().ok()))),
                    ("note", Json::str(&note)),
                    ("holdout", Json::Bool(holdout)),
                    ("label", Json::str(&format!("mac-{station}"))),
                ],
            )?;
            Ok(id)
        }
        "sessio" => {
            let text = req.query.get("teksti").cloned().unwrap_or_default();
            let ok = req.query.get("ok").map_or(false, |v| v == "1");
            let phy = PhyKind::DEFAULT.name();
            let id = format!("{now}-{station}-sessio-{phy}");
            let meta = |holdout: bool| {
                Json::obj(vec![
                    ("id", Json::str(&id)),
                    ("kind", Json::str("sessio")),
                    ("time_utc", Json::str(&now)),
                    ("device", Json::str(&format!("{station} ({device})"))),
                    ("sample_rate", Json::num(w.sample_rate)),
                    ("distance_m", Json::opt(req.query.get("etaisyys").and_then(|d| d.parse().ok()))),
                    ("note", Json::str(&note)),
                    ("holdout", Json::Bool(holdout)),
                    ("phy", Json::str(phy)),
                    ("text", Json::str(&text)),
                    ("ok_when_recorded", Json::Bool(ok)),
                ])
            };
            // Kaikki talteen kehityshakemistoon; korpukseen vain pyydettäessä.
            let dir = Path::new(DEV_DATA).join("aanitteet");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{id}.wav")), &req.body).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("{id}.json")), meta(false).pretty()).map_err(|e| e.to_string())?;
            let mut to_corpus = false;
            {
                let mut s = shared.lock().unwrap();
                if s.collect > 0 && !text.is_empty() {
                    s.collect -= 1;
                    to_corpus = true;
                }
            }
            if to_corpus {
                std::fs::create_dir_all(corpus::TX_DIR).map_err(|e| e.to_string())?;
                let base = Path::new(corpus::TX_DIR).join(&id);
                std::fs::write(base.with_extension("wav"), &req.body).map_err(|e| e.to_string())?;
                std::fs::write(base.with_extension("json"), meta(corpus::auto_holdout(&id)).pretty()).map_err(|e| e.to_string())?;
            }
            println!("[{}] {station}: sessioäänite {secs:.1} s, {} ({}){}", clock(), if ok { "ok" } else { "EPÄONNISTUI" }, text.chars().take(40).collect::<String>(), if to_corpus { " -> korpus" } else { "" });
            Ok(id)
        }
        _ => {
            let dir = Path::new(DEV_DATA).join("aanitteet");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let id = format!("{now}-{station}-{}", clean_id(kind));
            std::fs::write(dir.join(format!("{id}.wav")), &req.body).map_err(|e| e.to_string())?;
            println!("[{}] {station}: äänite {secs:.1} s -> {}/{id}.wav", clock(), dir.display());
            Ok(id)
        }
    }
}

fn handle(mut stream: TcpStream, shared: Arc<Mutex<Shared>>) {
    let Some(req) = read_request(&mut stream) else { return };
    let path = if req.path == "/" { "/index.html" } else { req.path.as_str() };
    match (req.method.as_str(), path) {
        ("GET", "/soundpaste.wasm") => match std::fs::read(WASM_PATH) {
            Ok(bytes) => respond(&mut stream, "200 OK", "application/wasm", &bytes),
            Err(_) => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "wasm puuttuu: aja ./rakenna.sh".as_bytes()),
        },
        ("GET", "/dev/version") => {
            let (page, wasm) = version();
            respond(&mut stream, "200 OK", "application/json", Json::obj(vec![("page", Json::num(page)), ("wasm", Json::num(wasm))]).pretty().as_bytes());
        }
        ("GET", "/dev/commands") => {
            let after: u64 = req.query.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
            let station = clean_id(req.query.get("asema").map_or("", |s| s));
            let mut s = shared.lock().unwrap();
            s.seen.insert(station, (clock(), req.query.get("tila").cloned().unwrap_or_default()));
            let latest = s.commands.last().map_or(0, |c| c.0);
            // after = 0: uusi sivu aloittaa nykyhetkestä eikä toista vanhoja komentoja.
            let list: Vec<Json> = if after == 0 { Vec::new() } else { s.commands.iter().filter(|c| c.0 > after).map(|c| Json::str(&c.1)).collect() };
            respond(&mut stream, "200 OK", "application/json", Json::obj(vec![("seq", Json::num(latest.max(1) as f64)), ("commands", Json::Arr(list))]).pretty().as_bytes());
        }
        ("GET", "/dev/status") => {
            let s = shared.lock().unwrap();
            let mut out = String::new();
            for (name, (time, state)) in &s.seen {
                out += &format!("{name}: viimeksi {time} UTC, {state}\n");
            }
            if out.is_empty() {
                out = "yhtään testiasemaa ei ole ilmoittautunut\n".into();
            }
            out += &format!("korpukseen kerättäviä äänitteitä jäljellä: {}\n", s.collect);
            respond(&mut stream, "200 OK", "text/plain; charset=utf-8", out.as_bytes());
        }
        ("POST", "/dev/command") => {
            let line = String::from_utf8_lossy(&req.body).trim().to_string();
            let mut s = shared.lock().unwrap();
            let mut words = line.split_whitespace();
            if words.next() == Some("keraa") {
                s.collect = words.next().and_then(|n| n.parse().ok()).unwrap_or(5);
                let n = s.collect;
                respond(&mut stream, "200 OK", "text/plain; charset=utf-8", format!("seuraavat {n} sessioäänitettä kopioidaan korpukseen\n").as_bytes());
                return;
            }
            let seq = s.commands.last().map_or(1, |c| c.0) + 1;
            s.commands.push((seq, line.clone()));
            println!("[{}] komento #{seq}: {line}", clock());
            respond(&mut stream, "200 OK", "text/plain; charset=utf-8", format!("komento #{seq} jonossa: {line}\n").as_bytes());
        }
        ("POST", "/dev/log") => {
            let station = clean_id(req.query.get("asema").map_or("", |s| s));
            let text = String::from_utf8_lossy(&req.body);
            let _ = std::fs::create_dir_all(DEV_DATA);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(Path::new(DEV_DATA).join(format!("{station}.log"))) {
                let _ = f.write_all(text.as_bytes());
            }
            for line in text.lines() {
                println!("[{station}] {line}");
            }
            respond(&mut stream, "204 No Content", "text/plain", b"");
        }
        ("POST", "/dev/aanite") => match save_recording(&req, &shared) {
            Ok(id) => respond(&mut stream, "200 OK", "text/plain; charset=utf-8", id.as_bytes()),
            Err(e) => {
                println!("[{}] äänitteen käsittely epäonnistui: {e}", clock());
                respond(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", e.as_bytes());
            }
        },
        ("GET", p) => match PAGE_FILES.iter().find(|(url, _)| *url == p) {
            Some((_, file)) => match std::fs::read(file) {
                Ok(mut bytes) => {
                    if p == "/index.html" {
                        // Testiaseman koodi lisätään vain kehityspalvelimessa.
                        let html = String::from_utf8_lossy(&bytes).replace("</body>", "<script src=\"./dev/dev.js\"></script>\n</body>");
                        bytes = html.into_bytes();
                    }
                    respond(&mut stream, "200 OK", content_type(p), &bytes);
                }
                Err(_) => respond(&mut stream, "404 Not Found", "text/plain", b"ei ole"),
            },
            None => respond(&mut stream, "404 Not Found", "text/plain", b"ei ole"),
        },
        _ => respond(&mut stream, "405 Method Not Allowed", "text/plain", b""),
    }
}

/// Tämän koneen osoite lähiverkossa (ei lähetä mitään: UDP-pistoke vain valitsee reitin).
fn lan_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

fn local_hostname() -> Option<String> {
    let out = std::process::Command::new("scutil").args(["--get", "LocalHostName"]).output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then(|| format!("{name}.local"))
}

pub fn cmd_serve(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let port: u16 = a.num("port", 8787)?;
    if !Path::new(WASM_PATH).exists() {
        return Err(format!("{WASM_PATH} puuttuu: aja ensin ./rakenna.sh"));
    }
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("portti {port}: {e}"))?;
    println!("KEHITYSPALVELIN (vain kehityskäyttöön)");
    println!("  tämä kone:   http://localhost:{port}/");
    for host in [local_hostname(), lan_ip()].into_iter().flatten() {
        println!("  lähiverkko:  http://{host}:{port}/?testi=vastaanotto&asema=pc");
    }
    println!("  testitilat:  ?testi=vastaanotto | kaiku | majakka (&vali=20), asema nimetään &asema=nimi");
    println!("  komennot:    soundpaste remote tila kaiku | laheta <teksti> | pyyhkaisy | aanita 9 kanava | keraa 5 | lataa");
    let shared = Arc::new(Mutex::new(Shared::default()));
    for stream in listener.incoming().flatten() {
        let shared = shared.clone();
        std::thread::spawn(move || handle(stream, shared));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn http(port: u16, method: &str, path: &str, body: &[u8]) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("kehityspalvelin ei vastaa portissa {port} ({e}): käynnistä se komennolla soundpaste serve"))?;
    let head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    s.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out).into_owned();
    Ok(text.split_once("\r\n\r\n").map_or(String::new(), |(_, b)| b.to_string()))
}

/// `soundpaste remote <komento>`: lähetä komento testiasemille kehityspalvelimen kautta.
pub fn cmd_remote(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let port: u16 = a.num("port", 8787)?;
    let line = a.positional.join(" ");
    if line.is_empty() || line == "tila" {
        print!("{}", http(port, "GET", "/dev/status", b"")?);
    } else {
        print!("{}", http(port, "POST", "/dev/command", line.as_bytes())?);
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(dead_code)]
pub fn dev_data_dir() -> PathBuf {
    PathBuf::from(DEV_DATA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b+c%C3%A4"), "a b cä");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn ids_are_sanitized() {
        assert_eq!(clean_id("../../etc/Passwd"), "etcpasswd");
        assert_eq!(clean_id(""), "asema");
    }
}
