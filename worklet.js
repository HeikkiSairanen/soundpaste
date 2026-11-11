// soundpaste: the AudioWorklet processor.
//
// Runs on the browser's audio thread. It:
// - copies microphone samples into the wasm module's buffer and speaker samples back
// - passes UI commands to the wasm module and events back
// - keeps the last 45 seconds of microphone audio for troubleshooting
//
// All signal processing and the protocol live in the wasm module (Rust). There
// are no network connections here and no TextDecoder (the audio thread has
// none), so events travel to the main thread as bytes.

const RING_SECONDS = 45;
const LEVEL_INTERVAL_S = 0.05;

class SoundpasteProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.x = null; // the wasm module's exports
    this.mem = null; // the wasm memory the views were created against
    this.ring = new Float32Array(Math.round(sampleRate * RING_SECONDS));
    this.ringPos = 0;
    this.ringFilled = 0;
    this.sinceLevel = 0;
    this.peak = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  views() {
    // When wasm memory grows the old views detach, so they are recreated.
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
        // A new module can be swapped in live: the microphone and audio stay on.
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
        // Ring buffer into chronological order.
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

    // Keep the recording (also before the module has loaded).
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

    // Events to the main thread.
    let events = null;
    for (;;) {
      let len = x.sp_poll_event(this.eventPtr, this.eventCap);
      if (len === 0) break;
      if (len < 0) {
        // The buffer was too small: allocate a bigger one and retry.
        x.sp_free(this.eventPtr, this.eventCap);
        this.eventCap = -len + 1024;
        this.eventPtr = x.sp_alloc(this.eventCap);
        continue;
      }
      const copy = new Uint8Array(x.memory.buffer, this.eventPtr, len).slice();
      (events || (events = [])).push(copy.buffer);
    }
    if (events) this.port.postMessage({ type: 'events', events, now: x.sp_now_ms() }, events);

    // Signal level and counters about 20 times a second.
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
