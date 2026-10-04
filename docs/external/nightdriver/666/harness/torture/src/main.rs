//! Deterministic TCP torture harness for a NightDriverStrip-style framed receiver.
//!
//! Drives a host build of the project's real socket-server source (see ../host/build.sh)
//! over loopback and checks what the receiver hands to its packet consumer.
//!
//! usage: nd666-torture <ndhost-binary> --label NAME --audio 0|1 [--only CASE] [--strict]
//!
//! Output: one tab separated line per case: CASE <tab> PASS|FAIL|INFO <tab> detail
//! PASS/FAIL cases encode a property the receiver must hold (stream stays in sync).
//! INFO cases record observed behaviour where the project has no stated contract.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NUM_LEDS: usize = 144; // must match the stub globals.h
const HDR: usize = 24;
const RESP: usize = 72;

// ---------- frame builders (wire layout per the sender, samples/audioserver/audioserver.py) ----------

fn fnv(b: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for x in b { h ^= *x as u32; h = h.wrapping_mul(16777619); }
    h
}

fn header(cmd: u16, w2: u16, len32: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(HDR);
    v.extend_from_slice(&cmd.to_le_bytes());
    v.extend_from_slice(&w2.to_le_bytes());
    v.extend_from_slice(&len32.to_le_bytes());
    v.extend_from_slice(&1_731_164_000u64.to_le_bytes());
    v.extend_from_slice(&0.25f64.to_le_bytes());
    v
}

/// Pixel frame (command 3) with n LEDs.
fn pixel(n: usize, seed: u8) -> Vec<u8> {
    let mut v = header(3, 0, n as u32);
    for i in 0..n * 3 { v.push((i as u8).wrapping_mul(7).wrapping_add(seed)); }
    v
}

/// Peak data frame (command 4) with `bands` float32 values; float[0] has the given bit pattern.
fn peak(bands: usize, f0_bits: u32) -> Vec<u8> {
    let mut v = header(4, bands as u16, (bands * 4) as u32);
    v.extend_from_slice(&f0_bits.to_le_bytes());
    for i in 1..bands { v.extend_from_slice(&(0.01f32 * i as f32).to_le_bytes()); }
    v
}

#[derive(Debug, Clone, PartialEq)]
struct Pkt { cmd: u16, len: usize, fnv: u32 }
fn expect(frame: &[u8]) -> Pkt {
    Pkt { cmd: u16::from_le_bytes([frame[0], frame[1]]), len: frame.len(), fnv: fnv(frame) }
}

// ---------- host process ----------

struct Host { child: Child, lines: Arc<Mutex<Vec<String>>>, port: u16, sessions: usize }

impl Host {
    fn spawn(bin: &str) -> Host {
        let port = { let l = TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
        let mut child = Command::new(bin).arg(port.to_string()).arg("2")
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn host");
        let out = child.stdout.take().unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l2 = lines.clone();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(out).lines().flatten() { l2.lock().unwrap().push(line); }
        });
        Host { child, lines, port, sessions: 0 }
    }
    fn count(&self, needle: &str) -> usize { self.lines.lock().unwrap().iter().filter(|l| l.contains(needle)).count() }
    /// Wait for the server to (re)start listening, then connect. One call per connection.
    fn connect(&mut self) -> TcpStream {
        self.sessions += 1;
        let t = Instant::now();
        while self.count("listening on port") < self.sessions {
            assert!(t.elapsed() < Duration::from_secs(10), "server never listened");
            std::thread::sleep(Duration::from_millis(10));
        }
        let s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        s.set_nodelay(true).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    }
    fn pkts(&self) -> Vec<Pkt> {
        self.lines.lock().unwrap().iter().filter_map(|l| {
            let r = l.strip_prefix("PKT ")?;
            let g = |k: &str| r.split_whitespace().find_map(|w| w.strip_prefix(k)).map(str::to_string);
            Some(Pkt { cmd: g("cmd=")?.parse().ok()?, len: g("len=")?.parse().ok()?, fnv: u32::from_str_radix(&g("fnv=")?, 16).ok()? })
        }).collect()
    }
    fn unknown(&self) -> Vec<u32> {
        self.lines.lock().unwrap().iter().filter_map(|l| {
            l.split("Unknown command in packet received: ").nth(1)?.trim().parse().ok()
        }).collect()
    }
    fn wait_pkts(&self, n: usize, ms: u64) {
        let t = Instant::now();
        while self.pkts().len() < n && t.elapsed() < Duration::from_millis(ms) { std::thread::sleep(Duration::from_millis(10)); }
    }
    fn settle(&self) { std::thread::sleep(Duration::from_millis(250)); }
}
impl Drop for Host { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

/// Read one response packet. Its first u32 is its own size (64 bytes before 2024-11-25, 72 after).
fn read_resp(s: &mut TcpStream) -> bool {
    let mut n = [0u8; 4];
    if s.read_exact(&mut n).is_err() { return false; }
    let size = u32::from_le_bytes(n) as usize;
    if !(8..=RESP).contains(&size) { return false; }
    let mut rest = vec![0u8; size - 4];
    s.read_exact(&mut rest).is_ok()
}
/// True if the peer closed (EOF or reset) within ms.
fn closed_by_peer(s: &mut TcpStream, ms: u64) -> bool {
    s.set_read_timeout(Some(Duration::from_millis(ms))).unwrap();
    let mut b = [0u8; 1];
    match s.read(&mut b) { Ok(0) => true, Ok(_) => false, Err(e) => !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) }
}
/// Write that tolerates the receiver having already closed (that is itself an observation, not a harness error).
fn wr(s: &mut TcpStream, b: &[u8]) { let _ = s.write_all(b); }
fn pause(ms: u64) { std::thread::sleep(Duration::from_millis(ms)); }

// ---------- cases ----------

enum V { Pass(String), Fail(String), Info(String) }
use V::*;

struct Env { bin: String, audio: bool }

fn check_exact(h: &Host, want: &[Pkt]) -> V {
    h.wait_pkts(want.len(), 3000);
    h.settle();
    let got = h.pkts();
    let unk = h.unknown();
    if got == want && unk.is_empty() { Pass(format!("{} packets, all matched", got.len())) }
    else { Fail(format!("want {} packets, got {} (unknown-command values: {:?})", want.len(), got.len(), unk)) }
}

fn c_whole_frames(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let f = [pixel(10, 1), pixel(10, 2), pixel(NUM_LEDS, 3)];
    for x in &f { wr(&mut s, x); if !read_resp(&mut s) { return Fail("no response packet".into()); } }
    check_exact(&h, &f.iter().map(|x| expect(x)).collect::<Vec<_>>())
}

fn c_split_every_offset(e: &Env) -> V {
    // One frame split into two writes at every possible offset, 15 ms apart, all on one connection.
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let f = pixel(10, 5);
    for k in 1..f.len() {
        wr(&mut s, &f[..k]); pause(15); wr(&mut s, &f[k..]);
        if !read_resp(&mut s) { return Fail(format!("no response after split at {k}")); }
    }
    check_exact(&h, &vec![expect(&f); f.len() - 1])
}

fn c_coalesced(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let (a, b, c) = (pixel(10, 1), pixel(20, 2), pixel(5, 3));
    let mut all = a.clone(); all.extend(&b); all.extend(&c);
    wr(&mut s, &all);                                   // three frames in one segment
    let mut d = pixel(7, 4); let mut tail = pixel(9, 5);
    let mut two = d.clone(); two.extend(&tail[..13]);             // frame + start of the next
    wr(&mut s, &two); pause(30); wr(&mut s, &tail[13..]);
    for _ in 0..5 { if !read_resp(&mut s) { return Fail("missing response".into()); } }
    d.truncate(d.len()); tail.truncate(tail.len());
    check_exact(&h, &[expect(&a), expect(&b), expect(&c), expect(&d), expect(&tail)])
}

fn c_one_byte_writes(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let f = [pixel(4, 9), pixel(4, 10)];
    for x in &f { for b in x { wr(&mut s, &[*b]); pause(1); } if !read_resp(&mut s) { return Fail("no response".into()); } }
    check_exact(&h, &f.iter().map(|x| expect(x)).collect::<Vec<_>>())
}

/// The audioserver.py shape: peak frame (12 bands) interleaved with pixel frames.
/// Property: the receiver never loses frame alignment (no "Unknown command"), whatever it does with audio.
/// float[0] low 16 bits are 0x79E2 = 31202, the value in issue 666.
fn c_peak12_with_pixels(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let p = peak(12, 0x3F00_79E2);
    let px = pixel(10, 1);
    for _ in 0..2 { wr(&mut s, &p); pause(20); wr(&mut s, &px); pause(20); }
    pause(400);
    let unk = h.unknown();
    let pk = h.pkts();
    if unk.is_empty() { Pass(format!("no desync; audio={} packets seen: {:?}", e.audio as u8, pk.iter().map(|x| (x.cmd, x.len)).collect::<Vec<_>>())) }
    else { Fail(format!("receiver lost frame alignment: Unknown command {:?}; packets {:?}", unk, pk.iter().map(|x| (x.cmd, x.len)).collect::<Vec<_>>())) }
}

fn c_peak16_with_pixels(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let p = peak(16, 0x3F00_79E2); let px = pixel(10, 1);
    wr(&mut s, &p); pause(20); wr(&mut s, &px); pause(20);
    wr(&mut s, &p); pause(20); wr(&mut s, &px);
    pause(500);
    let want: Vec<Pkt> = if e.audio { vec![expect(&p), expect(&px), expect(&p), expect(&px)] } else { vec![expect(&px), expect(&px)] };
    let got = h.pkts(); let unk = h.unknown();
    if got == want && unk.is_empty() { Pass(format!("audio={} sequence consumed in order, {} packets", e.audio as u8, got.len())) }
    else { Fail(format!("want {} packets got {}; unknown-command {:?}", want.len(), got.len(), unk)) }
}

fn c_http_get(e: &Env) -> V {
    // Issue 666 second reporter: a websocket client aimed at the raw port sends an HTTP upgrade request.
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    wr(&mut s, b"GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n");
    pause(500);
    let unk = h.unknown();
    let closed = closed_by_peer(&mut s, 1000);
    if unk == vec![17735] && closed { Pass("rejected with Unknown command 17735 (= 0x4547, bytes 'G','E'), connection closed".into()) }
    else { Fail(format!("unknown={:?} closed={}", unk, closed)) }
}

fn c_disconnect_partial_then_valid(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin);
    let f = pixel(10, 1);
    { let mut s = h.connect(); wr(&mut s, &f[..10]); pause(100); }      // drop mid-header
    { let mut s = h.connect(); wr(&mut s, &f[..40]); pause(100); }      // drop mid-body
    let mut s = h.connect(); wr(&mut s, &f); read_resp(&mut s);
    check_exact(&h, &[expect(&f)])
}

fn c_malformed_then_valid(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin);
    { let mut s = h.connect(); let mut bad = header(0x7777, 0, 0); bad.extend_from_slice(&[0; 8]); wr(&mut s, &bad); pause(300); }
    let f = pixel(10, 2);
    let mut s = h.connect(); wr(&mut s, &f); read_resp(&mut s);
    h.wait_pkts(1, 3000); h.settle();
    if h.pkts() == vec![expect(&f)] && h.unknown() == vec![0x7777] { Pass("bad command rejected (30583), next connection clean".into()) }
    else { Fail(format!("pkts={:?} unknown={:?}", h.pkts().len(), h.unknown())) }
}

fn c_max_and_oversize(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin);
    let max = pixel(NUM_LEDS, 1);
    { let mut s = h.connect(); wr(&mut s, &max); read_resp(&mut s); }
    let over = pixel(NUM_LEDS + 1, 2);
    let closed;
    { let mut s = h.connect(); wr(&mut s, &over); closed = closed_by_peer(&mut s, 1500); }
    let ok = pixel(3, 3);
    { let mut s = h.connect(); wr(&mut s, &ok); read_resp(&mut s); }
    h.wait_pkts(2, 3000); h.settle();
    if h.pkts() == vec![expect(&max), expect(&ok)] && closed { Pass("max frame accepted, max+1 rejected and connection closed, next clean".into()) }
    else { Fail(format!("pkts={} closed={}", h.pkts().len(), closed)) }
}

fn c_huge_length(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin);
    let mut bad = header(3, 0, 0xFFFF_FFFF); bad.extend_from_slice(&[0; 16]);
    let closed;
    { let mut s = h.connect(); wr(&mut s, &bad); closed = closed_by_peer(&mut s, 1500); }
    pause(200);
    if h.pkts().is_empty() && closed { Pass("length32=0xFFFFFFFF rejected on this 64-bit host".into()) } else { Fail(format!("pkts={} closed={}", h.pkts().len(), closed)) }
}

fn c_zero_length(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    wr(&mut s, &pixel(0, 0)); pause(300);
    let resp = { s.set_read_timeout(Some(Duration::from_millis(300))).unwrap(); read_resp(&mut s) };
    Info(format!("zero-LED pixel frame: packets={:?} response={} unknown={:?}", h.pkts().iter().map(|p| p.len).collect::<Vec<_>>(), resp, h.unknown()))
}

fn c_slow_sender_within_timeout(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let f = pixel(10, 4);
    wr(&mut s, &f[..5]); pause(1000); wr(&mut s, &f[5..30]); pause(1000); wr(&mut s, &f[30..]);
    read_resp(&mut s);
    check_exact(&h, &[expect(&f)])
}

fn c_slow_sender_past_timeout(e: &Env) -> V {
    // The receiver sets a 3 s read timeout so a stalled sender cannot hang it. Property: it drops the
    // connection and the next connection works.
    let mut h = Host::spawn(&e.bin);
    let f = pixel(10, 6);
    { let mut s = h.connect(); wr(&mut s, &f[..10]); pause(3600); wr(&mut s, &f[10..]); pause(200); }
    let g = pixel(10, 7);
    { let mut s = h.connect(); wr(&mut s, &g); read_resp(&mut s); }
    h.wait_pkts(1, 3000); h.settle();
    let got = h.pkts();
    if got == vec![expect(&g)] { Pass("stalled partial frame timed out and was discarded; next connection clean".into()) }
    else { Fail(format!("packets={:?} unknown={:?}", got.iter().map(|p| (p.cmd, p.len)).collect::<Vec<_>>(), h.unknown())) }
}

fn c_slow_receiver(e: &Env) -> V {
    // Client never reads the 72-byte response packets. Observe how many frames still get consumed.
    let mut h = Host::spawn(&e.bin); let mut s = h.connect();
    let f = pixel(2, 1);
    s.set_write_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut sent = 0usize;
    for _ in 0..4000 { if s.write_all(&f).is_err() { break; } sent += 1; }
    pause(1500);
    Info(format!("sent {} frames without reading responses; receiver consumed {}; unknown={:?}", sent, h.pkts().len(), h.unknown()))
}

fn c_sender_short_write(e: &Env) -> V {
    // Emulates a sender whose send() wrote only the first k bytes and never resent the tail
    // (the failure sendall() prevents). Records how the receiver reports it. k=40 cuts mid-body.
    let mut out = Vec::new();
    for k in [12usize, 24, 40] {
        let mut h = Host::spawn(&e.bin); let mut s = h.connect();
        let a = pixel(10, 1); let b = pixel(10, 2); let c = pixel(10, 3);
        wr(&mut s, &a[..k]); pause(20);
        wr(&mut s, &b); pause(20); wr(&mut s, &c); pause(400);
        let intact = h.pkts().iter().filter(|p| **p == expect(&a) || **p == expect(&b) || **p == expect(&c)).count();
        out.push(format!("k={}: packets={} intact={} unknown={:?}", k, h.pkts().len(), intact, h.unknown()));
    }
    Info(out.join(" | "))
}

fn c_reconnect_storm(e: &Env) -> V {
    let mut h = Host::spawn(&e.bin);
    let mut want = Vec::new();
    for i in 0..5u8 { let f = pixel(6, i); let mut s = h.connect(); wr(&mut s, &f); read_resp(&mut s); want.push(expect(&f)); }
    check_exact(&h, &want)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 2 { eprintln!("usage: nd666-torture <ndhost-binary> --label NAME --audio 0|1 [--only CASE] [--strict]"); std::process::exit(2); }
    let opt = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let label = opt("--label").unwrap_or_else(|| a[1].clone());
    let e = Env { bin: a[1].clone(), audio: opt("--audio").as_deref() == Some("1") };
    let only = opt("--only");
    let strict = a.iter().any(|x| x == "--strict");
    let cases: Vec<(&str, fn(&Env) -> V)> = vec![
        ("whole_frames", c_whole_frames), ("split_every_offset", c_split_every_offset), ("coalesced", c_coalesced),
        ("one_byte_writes", c_one_byte_writes), ("peak12_with_pixels", c_peak12_with_pixels), ("peak16_with_pixels", c_peak16_with_pixels),
        ("http_get_on_raw_port", c_http_get), ("disconnect_partial_then_valid", c_disconnect_partial_then_valid),
        ("malformed_then_valid", c_malformed_then_valid), ("max_and_oversize", c_max_and_oversize), ("huge_length", c_huge_length),
        ("zero_length", c_zero_length), ("slow_sender_within_timeout", c_slow_sender_within_timeout),
        ("slow_sender_past_timeout", c_slow_sender_past_timeout), ("slow_receiver", c_slow_receiver),
        ("sender_short_write", c_sender_short_write), ("reconnect_storm", c_reconnect_storm),
    ];
    let mut fails = 0;
    for (name, f) in cases {
        if only.as_deref().map_or(false, |o| o != name) { continue; }
        let (tag, d) = match f(&e) { Pass(d) => ("PASS", d), Fail(d) => { fails += 1; ("FAIL", d) } Info(d) => ("INFO", d) };
        println!("{label}\t{name}\t{tag}\t{d}");
    }
    println!("{label}\tSUMMARY\tfails={fails}");
    if strict && fails > 0 { std::process::exit(1); }
}
