// soundpaste: main-thread glue.
//
// Handles only the browser APIs: microphone (getUserMedia), audio
// (AudioContext, AudioWorklet), clipboard and the view. The modem and the
// protocol live in the wasm module, which runs on the audio thread (worklet.js).
//
// Received text is untrusted input: it is always shown with textContent,
// never as HTML, and links are never opened.

(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const encoder = new TextEncoder();
  const decoder = new TextDecoder('utf-8', { fatal: false });
  const params = new URLSearchParams(location.search);

  const app = {
    ctx: null,
    node: null,
    stream: null,
    ready: false,
    sending: false,
    maxMessage: 512,
    sampleRate: 0,
    received: '',
    stats: null,
    stationNow: 0,
    listeners: [], // dev-tool listeners (dev.js), empty in production
    pendingRecording: new Map(),
    peakHold: -80,
    clipSince: 0,
  };

  // ------------------------------------------------------------------ log

  const logLines = [];
  function log(text) {
    const t = new Date();
    const stamp = t.toTimeString().slice(0, 8) + '.' + String(t.getMilliseconds()).padStart(3, '0');
    const station = app.stationNow ? (app.stationNow / 1000).toFixed(2).padStart(8) + ' s' : '          ';
    const line = `${stamp} ${station}  ${text}`;
    logLines.push(line);
    if (logLines.length > 2000) logLines.shift();
    const el = $('log');
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 4;
    el.textContent = logLines.join('\n');
    if (atBottom) el.scrollTop = el.scrollHeight;
    emit({ kind: 'log', text: line });
  }

  function emit(event) {
    for (const fn of app.listeners) {
      try { fn(event); } catch (err) { console.error(err); }
    }
  }

  // ------------------------------------------------------------------ startup

  async function start() {
    const button = $('start');
    const status = $('start-status');
    button.disabled = true;
    try {
      if (!window.isSecureContext || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
        throw new Error('The microphone needs a secure connection (https) and a browser that supports audio capture.');
      }
      if (!window.AudioWorkletNode || !window.WebAssembly) {
        throw new Error('This browser does not support AudioWorklet or WebAssembly.');
      }
      status.textContent = 'Requesting the microphone…';
      // Speech processing breaks the modem signal, so we ask for it to be turned off.
      app.stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
        video: false,
      });
      const track = app.stream.getAudioTracks()[0];
      const settings = track.getSettings ? track.getSettings() : {};
      log(`microphone: ${track.label || 'unnamed'}; echo cancellation=${settings.echoCancellation}, noise suppression=${settings.noiseSuppression}, auto gain=${settings.autoGainControl}`);
      if (settings.echoCancellation || settings.noiseSuppression || settings.autoGainControl) {
        log('WARNING: the browser did not turn speech processing off. The transfer may fail.');
      }

      status.textContent = 'Loading the modem…';
      // The sample rate is not assumed: the browser picks the device rate and
      // the modem reads it from the AudioContext.
      app.ctx = new AudioContext({ latencyHint: 'interactive' });
      await app.ctx.audioWorklet.addModule('./worklet.js');
      const wasm = await fetchWasm();
      app.node = new AudioWorkletNode(app.ctx, 'soundpaste', {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        channelCount: 1,
        channelCountMode: 'explicit',
        channelInterpretation: 'discrete',
      });
      app.node.port.onmessage = (e) => onWorkletMessage(e.data);
      app.node.onprocessorerror = () => fatal('Audio processing crashed. Reload the page.');
      loadWasm(wasm);
      app.ctx.createMediaStreamSource(app.stream).connect(app.node);
      app.node.connect(app.ctx.destination);
      await app.ctx.resume();
      app.sampleRate = app.ctx.sampleRate;
      const outLatency = ((app.ctx.baseLatency || 0) + (app.ctx.outputLatency || 0)) * 1000;
      log(`audio: ${app.ctx.sampleRate} Hz, output latency about ${outLatency.toFixed(0)} ms`);
      track.addEventListener('ended', () => fatal('The microphone was disconnected. Reload the page.'));
      app.ctx.addEventListener('statechange', () => {
        log(`audio state: ${app.ctx.state}`);
        if (app.ctx.state === 'suspended') app.ctx.resume().catch(() => {});
      });
    } catch (err) {
      button.disabled = false;
      const denied = err && (err.name === 'NotAllowedError' || err.name === 'SecurityError');
      const missing = err && err.name === 'NotFoundError';
      status.textContent = denied
        ? 'Microphone access was not allowed. Allow the microphone from the address bar and press Start again.'
        : missing
          ? 'No microphone found.'
          : `Startup failed: ${err && err.message ? err.message : err}`;
      log(`startup error: ${err && err.name ? err.name + ': ' : ''}${err && err.message ? err.message : err}`);
    }
  }

  async function fetchWasm() {
    const response = await fetch('./soundpaste.wasm', { cache: 'no-cache' });
    if (!response.ok) throw new Error(`loading the modem failed (${response.status})`);
    return response.arrayBuffer();
  }

  function loadWasm(buffer) {
    const seed = crypto.getRandomValues(new Uint32Array(2));
    const flags = params.get('handshake') === '1' ? 1 : 0;
    app.node.port.postMessage({ type: 'load', wasm: buffer, seed: [seed[0], seed[1]], flags }, [buffer]);
  }

  function fatal(message) {
    $('start-section').hidden = false;
    $('start').hidden = true;
    $('start-status').textContent = message;
    log(message);
  }

  // ------------------------------------------------------------------ messages from the audio thread

  function onWorkletMessage(m) {
    switch (m.type) {
      case 'ready':
        app.ready = true;
        app.maxMessage = m.maxMessage;
        app.sending = false;
        $('start-section').hidden = true;
        for (const id of ['meter-section', 'send-section', 'receive-section']) $(id).hidden = false;
        $('download').disabled = false;
        log(`modem ready: ${m.sampleRate} Hz, maximum message length ${m.maxMessage} bytes`);
        updateInput();
        setSending(false);
        emit({ kind: 'ready', sampleRate: m.sampleRate });
        break;
      case 'events':
        app.stationNow = m.now;
        for (const buffer of m.events) onEvent(new Uint8Array(buffer));
        break;
      case 'level':
        app.stationNow = m.now;
        app.stats = m.stats;
        updateMeter(m.rms, m.peak);
        break;
      case 'estimate':
        showEstimate(m.len, m.seconds);
        break;
      case 'sendResult':
        if (m.code !== 0) {
          setSending(false);
          setTx(m.code === 1 ? 'The previous send is still in progress.' : 'The message is too long.', false);
        }
        break;
      case 'recording': {
        const resolve = app.pendingRecording.get(m.tag);
        app.pendingRecording.delete(m.tag);
        if (resolve) resolve({ samples: new Float32Array(m.samples), sampleRate: m.sampleRate });
        break;
      }
      case 'error':
        log(`error on the audio thread: ${m.message}`);
        if (!app.ready) fatal(`Starting the modem failed: ${m.message}`);
        break;
    }
  }

  const FAIL_TEXT = [
    'No receiver found.',
    'The receiver stopped responding during the transfer.',
    'The transfer failed despite retries.',
    'The receiver acknowledged different content than was sent.',
  ];
  const FAIL_ADVICE = [
    'Check that the other computer has this page open with Start pressed, that the computers are close together and that sound is on. Try turning the volume up.',
    'Try again. If it keeps happening, move the computers closer together.',
    'Try again at a quieter moment or move the computers closer together.',
    'Try again.',
  ];
  const HINT_ADVICE = [
    '',
    'The signal is too quiet at the receiver: turn the volume up or move the computers closer together.',
    'The signal clips in the receiver\'s microphone: turn the volume down or move the computers further apart.',
    'There is too much background noise: try again at a quieter moment or move the computers closer together.',
  ];

  function onEvent(b) {
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    switch (b[0]) {
      case 1: { // send status
        const [sub, a, x, c] = [b[1], dv.getUint32(2, true), dv.getUint32(6, true), dv.getUint32(10, true)];
        const text = [
          () => 'Checking that the channel is free…',
          () => `Looking for a receiver… (attempt ${a}/${x})`,
          () => `Sending, frame ${a}/${x}${c > 1 ? ' (retransmission)' : ''}`,
          () => 'Waiting for acknowledgement…',
          () => `No acknowledgement heard, asking again (${a}/${x})…`,
          () => `Retrying: the receiver is missing ${a} ${a === 1 ? 'frame' : 'frames'}.`,
          () => 'The other computer is sending right now. Your message goes out right after.',
        ][sub]();
        setTx(text, false);
        emit({ kind: 'tx', sub, a, b: x, c });
        break;
      }
      case 2: { // delivered
        const seconds = dv.getUint32(3, true) / 1000;
        setSending(false);
        setTx(`Delivered (${seconds.toFixed(1)} s)`, true);
        emit({ kind: 'delivered', seconds });
        break;
      }
      case 3: { // failed
        const [reason, hint, hasQuality] = [b[1], b[2], b[3]];
        setSending(false);
        setTx(`Failed: ${FAIL_TEXT[reason] || 'unknown reason.'}`, false);
        const advice = $('tx-advice');
        advice.textContent = HINT_ADVICE[hint] || FAIL_ADVICE[reason] || '';
        advice.hidden = !advice.textContent;
        emit({ kind: 'failed', reason, hint, quality: hasQuality ? { snrDb: dv.getInt8(4), levelDb: dv.getInt8(5), clipped: !!b[6] } : null });
        break;
      }
      case 4:
        setSending(false);
        setTx('Send cancelled.', false);
        emit({ kind: 'cancelled' });
        break;
      case 5: { // receiving started
        const len = dv.getUint32(3, true);
        $('rx-status').textContent = `Receiving a message (${len} bytes)…`;
        $('rx-status').classList.remove('ok');
        const progress = $('rx-progress');
        progress.hidden = false;
        progress.value = 0;
        emit({ kind: 'rxStarted', len });
        break;
      }
      case 6: { // progress
        const [have, of] = [dv.getUint32(1, true), dv.getUint32(5, true)];
        $('rx-status').textContent = `Receiving: ${have}/${of} frames`;
        const progress = $('rx-progress');
        progress.hidden = false;
        progress.max = of;
        progress.value = have;
        emit({ kind: 'rxProgress', have, of });
        break;
      }
      case 7: { // received
        const len = dv.getUint32(3, true);
        const bytes = b.subarray(7, 7 + len);
        app.received = decoder.decode(bytes);
        const el = $('received');
        el.textContent = app.received; // always as plain text
        el.hidden = false;
        $('rx-progress').hidden = true;
        $('copy').hidden = false;
        $('copy-status').textContent = '';
        const status = $('rx-status');
        status.textContent = len === 0 ? 'Received an empty message.' : `Received ${len} bytes.`;
        status.classList.add('ok');
        emit({ kind: 'received', text: app.received, bytes: Array.from(bytes) });
        break;
      }
      case 8:
        $('rx-progress').hidden = true;
        $('rx-status').classList.remove('ok');
        $('rx-status').textContent = b[1] === 1 ? 'The sender cancelled.' : 'Nothing more was heard from the sender: the incomplete message was discarded.';
        emit({ kind: 'rxAborted', cancelled: b[1] === 1 });
        break;
      case 9:
        log(decoder.decode(b.subarray(1)));
        break;
    }
  }

  // ------------------------------------------------------------------ sending

  function setTx(text, ok) {
    const el = $('tx-status');
    el.textContent = text;
    el.classList.toggle('ok', !!ok);
    if (text) log(`status: ${text}`);
    if (ok || !text) $('tx-advice').hidden = true;
  }

  function setSending(on) {
    app.sending = on;
    $('cancel').hidden = !on;
    $('text').readOnly = on;
    updateInput();
  }

  function updateInput() {
    const bytes = encoder.encode($('text').value).length;
    const tooLong = bytes > app.maxMessage;
    $('count').textContent = `${bytes} / ${app.maxMessage} bytes`;
    const warn = $('too-long');
    warn.hidden = !tooLong;
    if (tooLong) {
      warn.textContent = `The message is too long: ${bytes} bytes, the maximum is ${app.maxMessage} bytes. Accented letters and emoji take several bytes each.`;
      $('estimate').textContent = '';
    }
    $('send').disabled = !app.ready || app.sending || tooLong;
    if (app.ready && !tooLong) app.node.port.postMessage({ type: 'estimate', len: bytes });
  }

  function showEstimate(len, seconds) {
    if (encoder.encode($('text').value).length !== len) return;
    $('estimate').textContent = `Transfer takes about ${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)} s`;
  }

  function send(text) {
    if (!app.ready || app.sending) return false;
    const bytes = encoder.encode(text);
    if (bytes.length > app.maxMessage) return false;
    $('tx-advice').hidden = true;
    setSending(true);
    setTx('Starting…', false);
    log(`sending ${bytes.length} bytes`);
    app.node.port.postMessage({ type: 'send', bytes: bytes.buffer }, [bytes.buffer]);
    return true;
  }

  // ------------------------------------------------------------------ signal level

  function updateMeter(rms, peak) {
    const db = rms > 0 ? 20 * Math.log10(rms) : -100;
    const peakDb = peak > 0 ? 20 * Math.log10(peak) : -100;
    const pos = (v) => Math.max(0, Math.min(100, ((v + 80) / 80) * 100));
    app.peakHold = Math.max(peakDb, app.peakHold - 1.5);
    const clipping = peak >= 0.98;
    if (clipping) app.clipSince = performance.now();
    const hot = performance.now() - app.clipSince < 1500;
    $('meter-bar').style.width = `${pos(db)}%`;
    $('meter-bar').classList.toggle('hot', hot);
    $('meter-peak').style.left = `${pos(app.peakHold)}%`;
    $('meter').setAttribute('aria-valuenow', String(Math.round(Math.max(-80, db))));
    $('meter-db').textContent = db <= -99 ? 'no signal' : `${db.toFixed(0)} dB`;
    $('meter-hint').textContent = hot
      ? 'The microphone is clipping: the sound is too loud.'
      : db <= -99
        ? 'No sound at all from the microphone. Check that it is not muted.'
        : '';
    const s = app.stats;
    if (s && $('debug').open) {
      $('stats').textContent = `Preambles ${s.preambles}, good frames ${s.framesOk}, rejected ${s.framesFailed}, sent ${s.framesSent}, corrected bytes ${s.corrected}` + (s.framesOk ? `, latest SNR ${s.snrDb.toFixed(0)} dB` : '');
    }
  }

  // ------------------------------------------------------------------ recording

  function requestRecording() {
    return new Promise((resolve) => {
      const tag = Math.random().toString(36).slice(2);
      app.pendingRecording.set(tag, resolve);
      app.node.port.postMessage({ type: 'dump', tag });
    });
  }

  function wavBytes(samples, sampleRate) {
    const buffer = new ArrayBuffer(44 + samples.length * 2);
    const dv = new DataView(buffer);
    const ascii = (offset, s) => { for (let i = 0; i < s.length; i++) dv.setUint8(offset + i, s.charCodeAt(i)); };
    ascii(0, 'RIFF');
    dv.setUint32(4, 36 + samples.length * 2, true);
    ascii(8, 'WAVEfmt ');
    dv.setUint32(16, 16, true);
    dv.setUint16(20, 1, true);
    dv.setUint16(22, 1, true);
    dv.setUint32(24, sampleRate, true);
    dv.setUint32(28, sampleRate * 2, true);
    dv.setUint16(32, 2, true);
    dv.setUint16(34, 16, true);
    ascii(36, 'data');
    dv.setUint32(40, samples.length * 2, true);
    for (let i = 0; i < samples.length; i++) {
      const v = Math.max(-1, Math.min(1, samples[i]));
      dv.setInt16(44 + i * 2, Math.round(v * 32767), true);
    }
    return buffer;
  }

  async function downloadRecording() {
    const { samples, sampleRate } = await requestRecording();
    const blob = new Blob([wavBytes(samples, sampleRate)], { type: 'audio/wav' });
    const a = document.createElement('a');
    const stamp = new Date().toISOString().replace(/[-:]/g, '').slice(0, 15);
    a.href = URL.createObjectURL(blob);
    a.download = `soundpaste-${stamp}.wav`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 10000);
    log(`recording downloaded: ${(samples.length / sampleRate).toFixed(1)} s, ${sampleRate} Hz`);
  }

  // ------------------------------------------------------------------ events

  $('start').addEventListener('click', start);
  $('text').addEventListener('input', updateInput);
  $('send').addEventListener('click', () => send($('text').value));
  $('cancel').addEventListener('click', () => {
    if (app.node) app.node.port.postMessage({ type: 'cancel' });
  });
  $('copy').addEventListener('click', async () => {
    // Writing to the clipboard needs a user gesture, so it only happens from the button.
    try {
      await navigator.clipboard.writeText(app.received);
      $('copy-status').textContent = 'Copied.';
    } catch (err) {
      $('copy-status').textContent = 'Copying failed. Select the text and copy it by hand.';
    }
  });
  $('download').addEventListener('click', downloadRecording);
  $('clear-log').addEventListener('click', () => {
    logLines.length = 0;
    $('log').textContent = '';
  });
  document.addEventListener('visibilitychange', () => {
    $('hidden-warning').hidden = !(document.hidden && app.ready);
    if (app.ready) log(document.hidden ? 'tab to background' : 'tab visible');
  });

  // Hook for the dev tools. Nothing uses this on the production page,
  // and nothing in this file contacts the network.
  window.soundpaste = {
    start,
    send,
    cancel: () => app.node && app.node.port.postMessage({ type: 'cancel' }),
    onEvent: (fn) => app.listeners.push(fn),
    recording: requestRecording,
    context: () => app.ctx,
    wavBytes,
    reloadWasm: (buffer) => { app.ready = false; loadWasm(buffer); },
    state: () => ({ ready: app.ready, sending: app.sending, sampleRate: app.sampleRate, stats: app.stats, stationNow: app.stationNow }),
    log,
  };
})();
