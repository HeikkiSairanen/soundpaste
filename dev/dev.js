// soundpaste: test-station code. DEVELOPMENT ONLY.
//
// The dev server (soundpaste serve) injects this into the page. The published
// version does not contain this file, and the production page contacts nothing.
//
// The test station is passive: once set up it needs no touching.
// - Mode from the URL parameter ?test=receive | echo | beacon (&interval=20), name &station=pc
// - Checks every two seconds for a new version: a new wasm is swapped in live
//   (the microphone stays on), a changed page is reloaded.
// - Sends the log and each session's microphone recording to the dev server.
// - Fetches commands from the server (soundpaste remote ...).

(() => {
  'use strict';
  const sp = window.soundpaste;
  const params = new URLSearchParams(location.search);
  const station = (params.get('station') || 'station').replace(/[^a-z0-9-]/gi, '').toLowerCase() || 'station';
  let mode = params.get('test') || 'receive';
  let beaconSeconds = Math.max(8, Number(params.get('interval')) || 20);
  const BEACON_TEXT = 'https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd';
  const device = navigator.userAgent.replace(/^Mozilla\/5\.0 /, '').slice(0, 120);

  // A visible sign that this is a test station.
  const banner = document.createElement('p');
  banner.className = 'notice warning';
  document.querySelector('main').prepend(banner);
  const showBanner = () => {
    banner.textContent = `TEST STATION "${station}", mode: ${mode}${mode === 'beacon' ? ` (every ${beaconSeconds} s)` : ''}. Logs and microphone recordings are sent to the dev server.`;
  };
  showBanner();

  // ---------------------------------------------------------------- log to the server
  let pendingLog = [];
  const post = (path, body, type) => fetch(path, { method: 'POST', body, headers: type ? { 'Content-Type': type } : undefined }).catch(() => {});
  setInterval(() => {
    if (!pendingLog.length) return;
    const text = pendingLog.join('\n') + '\n';
    pendingLog = [];
    post(`./dev/log?station=${station}`, text, 'text/plain');
  }, 2000);

  // ---------------------------------------------------------------- recordings
  let sessionStart = 0; // station clock (ms) when the session started
  let lastSentText = '';

  async function uploadRecording(kind, query, seconds) {
    const { samples, sampleRate } = await sp.recording();
    const keep = Math.min(samples.length, Math.round(seconds * sampleRate));
    const wav = sp.wavBytes(samples.subarray(samples.length - keep), sampleRate);
    const q = new URLSearchParams({ station, kind, device, ...query });
    await post(`./dev/recording?${q}`, wav, 'audio/wav');
  }

  function sessionSeconds() {
    const now = sp.state().stationNow;
    return Math.min(44, Math.max(4, (now - sessionStart) / 1000 + 3));
  }

  // ---------------------------------------------------------------- events
  let beaconTimer = null;
  function scheduleBeacon() {
    clearTimeout(beaconTimer);
    if (mode !== 'beacon') return;
    beaconTimer = setTimeout(() => {
      if (sp.state().ready && !sp.state().sending) {
        lastSentText = BEACON_TEXT;
        sp.send(BEACON_TEXT);
      }
      scheduleBeacon();
    }, beaconSeconds * 1000);
  }

  sp.onEvent((e) => {
    switch (e.kind) {
      case 'log':
        pendingLog.push(e.text);
        break;
      case 'ready':
        scheduleBeacon();
        break;
      case 'rxStarted':
        sessionStart = sp.state().stationNow;
        break;
      case 'tx':
        if (e.sub === 0) sessionStart = sp.state().stationNow;
        break;
      case 'received':
        // Upload the recording a moment later so the final ack is included too.
        setTimeout(() => uploadRecording('session', { text: e.text, ok: '1', note: `mode ${mode}, received` }, sessionSeconds()), 1200);
        if (mode === 'echo') {
          // Echo mode: the same text back as a new message.
          setTimeout(() => {
            lastSentText = e.text;
            sp.send(e.text);
          }, 2500);
        }
        break;
      case 'rxAborted':
        setTimeout(() => uploadRecording('session', { text: '', ok: '0', note: `mode ${mode}, receive aborted` }, sessionSeconds()), 500);
        break;
      case 'failed':
        setTimeout(() => uploadRecording('session', { text: '', ok: '0', note: `mode ${mode}, send failed (reason ${e.reason})` }, sessionSeconds()), 500);
        break;
    }
  });

  // ---------------------------------------------------------------- version check
  let version = null;
  async function checkVersion() {
    try {
      const v = await (await fetch('./dev/version', { cache: 'no-store' })).json();
      if (!version) {
        version = v;
        return;
      }
      if (v.page !== version.page) {
        sp.log('the page has changed, reloading');
        setTimeout(() => location.reload(), 300);
        return;
      }
      if (v.wasm !== version.wasm && sp.state().ready && !sp.state().sending) {
        version = v;
        const buffer = await (await fetch('./soundpaste.wasm', { cache: 'no-store' })).arrayBuffer();
        sp.log(`new modem (${buffer.byteLength} bytes), swapping it in live`);
        sp.reloadWasm(buffer);
      }
    } catch (err) {
      // The server is not running: try again later.
    }
  }
  setInterval(checkVersion, 2000);
  checkVersion();

  // ---------------------------------------------------------------- commands
  // Channel-measurement sweep: the same signal as soundpaste measure plays.
  function playSweep(amplitude) {
    const ctx = sp.context();
    if (!ctx) return;
    const fs = ctx.sampleRate;
    const [f1, f2, T] = [100, 20000, 3.0];
    const lead = Math.round(1.0 * fs);
    const n = Math.round(T * fs);
    const buffer = ctx.createBuffer(1, lead + n + Math.round(1.5 * fs), fs);
    const data = buffer.getChannelData(0);
    const k = Math.log(f2 / f1);
    const fade = Math.round(0.02 * fs);
    for (let i = 0; i < n; i++) {
      const t = i / fs;
      const phase = ((2 * Math.PI * f1 * T) / k) * (Math.exp((t * k) / T) - 1);
      const edge = Math.min(i, n - 1 - i);
      const g = edge < fade ? 0.5 - 0.5 * Math.cos((Math.PI * edge) / fade) : 1;
      data[lead + i] = Math.sin(phase) * g * amplitude;
    }
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    source.start();
    sp.log(`playing the sweep, amplitude ${amplitude}`);
  }

  let commandSeq = 0;
  async function pollCommands() {
    try {
      const state = `${mode}${sp.state().ready ? '' : ' (not started)'}`;
      const r = await (await fetch(`./dev/commands?after=${commandSeq}&station=${station}&mode=${encodeURIComponent(state)}`, { cache: 'no-store' })).json();
      commandSeq = r.seq;
      for (const line of r.commands) runCommand(line);
    } catch (err) {
      // no connection
    }
  }
  setInterval(pollCommands, 1000);

  function runCommand(line) {
    const [cmd, ...rest] = line.trim().split(/\s+/);
    sp.log(`command: ${line}`);
    if (cmd === 'mode' && rest[0]) {
      mode = rest[0];
      if (rest[1]) beaconSeconds = Math.max(8, Number(rest[1]) || beaconSeconds);
      showBanner();
      scheduleBeacon();
    } else if (cmd === 'send') {
      lastSentText = line.trim().slice(cmd.length + 1);
      sp.send(lastSentText);
    } else if (cmd === 'cancel') {
      sp.cancel();
    } else if (cmd === 'sweep') {
      playSweep(Number(rest[0]) || 0.3);
    } else if (cmd === 'record') {
      // Record N seconds from now and upload (e.g. a sweep played by the Mac).
      const seconds = Math.min(40, Number(rest[0]) || 8);
      const kind = rest[1] || 'other';
      const gain = rest[2] || '0.3';
      // An optional leading word "holdout" marks the recording as part of the holdout set.
      const holdout = rest[3] === 'holdout';
      const query = { gain, note: rest.slice(holdout ? 4 : 3).join(' ') };
      if (holdout) query.holdout = '1';
      setTimeout(() => uploadRecording(kind, query, seconds), seconds * 1000);
    } else if (cmd === 'reload') {
      location.reload();
    }
  }

  // ---------------------------------------------------------------- automatic start
  // The test station's browser is launched with flags that allow audio and the
  // microphone without a click (see README). Without them the Start button works.
  window.addEventListener('load', () => setTimeout(() => sp.start(), 300));
})();
