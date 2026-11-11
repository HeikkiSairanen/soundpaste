// soundpaste: AudioWorklet-prosessori.
//
// Pyörii selaimen äänisäikeessä. Tehtävät:
// - kopioi mikrofonin näytteet wasm-moduulin puskuriin ja kaiuttimen näytteet takaisin
// - välittää käyttöliittymän komennot wasm-moduulille ja tapahtumat takaisin
// - pitää tallessa viimeiset 45 sekuntia mikrofoniääntä vianetsintää varten
//
// Kaikki signaalinkäsittely ja protokolla on wasm-moduulissa (Rust). Tässä ei
// ole verkkoyhteyksiä eikä TextDecoderia (sitä ei ole äänisäikeessä), joten
// tapahtumat kulkevat pääsäikeelle tavuina.

const RING_SECONDS = 45;
const LEVEL_INTERVAL_S = 0.05;

class SoundpasteProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.x = null; // wasm-moduulin viennit
    this.mem = null; // wasm-muisti, jota vasten näkymät on luotu
    this.ring = new Float32Array(Math.round(sampleRate * RING_SECONDS));
    this.ringPos = 0;
    this.ringFilled = 0;
    this.sinceLevel = 0;
    this.peak = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  views() {
    // Wasm-muistin kasvaessa vanhat näkymät irtoavat, joten ne luodaan uudelleen.
    const x = this.x;
    if (this.mem !== x.memory.buffer) {
      this.mem = x.memory.buffer;
      const max = x.sp_max_block();
      this.inView = new Float32Array(this.mem, x.sp_input_ptr(), max);
      this.outView = new Float32Array(this.mem, x.sp_output_ptr(), max);
    }
  }

  onMessage(m) {
    try {
      if (m.type === 'load') {
        // Uusi moduuli voidaan vaihtaa lennossa: mikrofoni ja ääni pysyvät päällä.
        const module = new WebAssembly.Module(m.wasm);
        const x = new WebAssembly.Instance(module, {}).exports;
        x.sp_new(sampleRate, m.seed[0] >>> 0, m.seed[1] >>> 0, m.flags | 0);
        this.x = x;
        this.mem = null;
        this.eventCap = 4096;
        this.eventPtr = x.sp_alloc(this.eventCap);
        this.statsPtr = x.sp_alloc(32);
        this.port.postMessage({ type: 'ready', sampleRate, maxMessage: x.sp_max_message() });
        return;
      }
      const x = this.x;
      if (!x) return;
      if (m.type === 'send') {
        const bytes = new Uint8Array(m.bytes);
        const p = x.sp_alloc(bytes.length);
        new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
        const code = x.sp_send(p, bytes.length);
        x.sp_free(p, bytes.length);
        this.port.postMessage({ type: 'sendResult', code });
      } else if (m.type === 'cancel') {
        x.sp_cancel();
      } else if (m.type === 'estimate') {
        this.port.postMessage({ type: 'estimate', len: m.len, seconds: x.sp_estimate(m.len) });
      } else if (m.type === 'dump') {
        // Rengaspuskuri aikajärjestykseen.
        const n = this.ringFilled;
        const out = new Float32Array(n);
        const start = (this.ringPos - n + this.ring.length) % this.ring.length;
        const first = Math.min(n, this.ring.length - start);
        out.set(this.ring.subarray(start, start + first), 0);
        out.set(this.ring.subarray(0, n - first), first);
        this.port.postMessage({ type: 'recording', samples: out.buffer, sampleRate, tag: m.tag }, [out.buffer]);
      }
    } catch (err) {
      this.port.postMessage({ type: 'error', message: String(err && err.message ? err.message : err) });
    }
  }

  process(inputs, outputs) {
    const out = outputs[0] && outputs[0][0];
    if (!out) return true;
    const input = inputs[0] && inputs[0][0];
    const n = out.length;

    // Äänite talteen (myös ennen kuin moduuli on ladattu).
    if (input) {
      for (let i = 0; i < n; i++) {
        this.ring[this.ringPos] = input[i];
        this.ringPos = (this.ringPos + 1) % this.ring.length;
        const a = input[i] < 0 ? -input[i] : input[i];
        if (a > this.peak) this.peak = a;
      }
      this.ringFilled = Math.min(this.ringFilled + n, this.ring.length);
    }

    const x = this.x;
    if (!x) return true;
    this.views();
    if (input) this.inView.set(input);
    else this.inView.fill(0, 0, n);
    x.sp_process(n);
    this.views();
    out.set(this.outView.subarray(0, n));

    // Tapahtumat pääsäikeelle.
    let events = null;
    for (;;) {
      let len = x.sp_poll_event(this.eventPtr, this.eventCap);
      if (len === 0) break;
      if (len < 0) {
        // Puskuri oli liian pieni: varataan isompi ja yritetään uudelleen.
        x.sp_free(this.eventPtr, this.eventCap);
        this.eventCap = -len + 1024;
        this.eventPtr = x.sp_alloc(this.eventCap);
        continue;
      }
      const copy = new Uint8Array(x.memory.buffer, this.eventPtr, len).slice();
      (events || (events = [])).push(copy.buffer);
    }
    if (events) this.port.postMessage({ type: 'events', events, now: x.sp_now_ms() }, events);

    // Signaalitaso ja laskurit noin 20 kertaa sekunnissa.
    this.sinceLevel += n;
    if (this.sinceLevel >= sampleRate * LEVEL_INTERVAL_S) {
      this.sinceLevel = 0;
      x.sp_stats(this.statsPtr);
      const dv = new DataView(x.memory.buffer, this.statsPtr, 28);
      this.port.postMessage({
        type: 'level',
        rms: x.sp_level(),
        peak: this.peak,
        now: x.sp_now_ms(),
        stats: {
          preambles: dv.getUint32(0, true),
          framesOk: dv.getUint32(4, true),
          framesFailed: dv.getUint32(8, true),
          framesSent: dv.getUint32(12, true),
          corrected: dv.getUint32(16, true),
          snrDb: dv.getFloat32(20, true),
          lastRms: dv.getFloat32(24, true),
        },
      });
      this.peak = 0;
    }
    return true;
  }
}

registerProcessor('soundpaste', SoundpasteProcessor);
