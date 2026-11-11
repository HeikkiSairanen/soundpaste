// soundpaste: pääsäikeen liima.
//
// Hoitaa vain selainrajapinnat: mikrofonin (getUserMedia), äänen
// (AudioContext, AudioWorklet), leikepöydän ja näkymän. Modeemi ja protokolla
// ovat wasm-moduulissa, jota ajetaan äänisäikeessä (worklet.js).
//
// Vastaanotettu teksti on vierasta syötettä: se näytetään aina textContentilla,
// ei koskaan HTML:nä, eikä linkkejä avata.

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
    listeners: [], // kehitystyökalujen kuuntelijat (dev.js), tuotannossa tyhjä
    pendingRecording: new Map(),
    peakHold: -80,
    clipSince: 0,
  };

  // ------------------------------------------------------------------ loki

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

  // ------------------------------------------------------------------ käynnistys

  async function start() {
    const button = $('start');
    const status = $('start-status');
    button.disabled = true;
    try {
      if (!window.isSecureContext || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
        throw new Error('Mikrofoni vaatii suojatun yhteyden (https) ja selaimen, joka tukee äänen kaappausta.');
      }
      if (!window.AudioWorkletNode || !window.WebAssembly) {
        throw new Error('Selain ei tue AudioWorkletia tai WebAssemblya.');
      }
      status.textContent = 'Pyydetään mikrofonia…';
      // Puheelle tehdyt käsittelyt rikkovat modeemisignaalin, joten ne pyydetään pois.
      app.stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
        video: false,
      });
      const track = app.stream.getAudioTracks()[0];
      const settings = track.getSettings ? track.getSettings() : {};
      log(`mikrofoni: ${track.label || 'nimetön'}; kaiunpoisto=${settings.echoCancellation}, kohinanvaimennus=${settings.noiseSuppression}, automaattinen taso=${settings.autoGainControl}`);
      if (settings.echoCancellation || settings.noiseSuppression || settings.autoGainControl) {
        log('VAROITUS: selain ei kytkenyt puheenkäsittelyä pois. Siirto voi epäonnistua.');
      }

      status.textContent = 'Ladataan modeemia…';
      // Näytteenottotaajuutta ei oleteta: selain valitsee laitteen taajuuden ja
      // modeemi lukee sen AudioContextista.
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
      app.node.onprocessorerror = () => fatal('Äänenkäsittely kaatui. Lataa sivu uudelleen.');
      loadWasm(wasm);
      app.ctx.createMediaStreamSource(app.stream).connect(app.node);
      app.node.connect(app.ctx.destination);
      await app.ctx.resume();
      app.sampleRate = app.ctx.sampleRate;
      const outLatency = ((app.ctx.baseLatency || 0) + (app.ctx.outputLatency || 0)) * 1000;
      log(`ääni: ${app.ctx.sampleRate} Hz, lähtöviive noin ${outLatency.toFixed(0)} ms`);
      track.addEventListener('ended', () => fatal('Mikrofoni katkesi. Lataa sivu uudelleen.'));
      app.ctx.addEventListener('statechange', () => {
        log(`äänen tila: ${app.ctx.state}`);
        if (app.ctx.state === 'suspended') app.ctx.resume().catch(() => {});
      });
    } catch (err) {
      button.disabled = false;
      const denied = err && (err.name === 'NotAllowedError' || err.name === 'SecurityError');
      const missing = err && err.name === 'NotFoundError';
      status.textContent = denied
        ? 'Mikrofonin käyttöä ei sallittu. Salli mikrofoni selaimen osoitepalkista ja paina Aloita uudelleen.'
        : missing
          ? 'Mikrofonia ei löytynyt.'
          : `Käynnistys epäonnistui: ${err && err.message ? err.message : err}`;
      log(`käynnistysvirhe: ${err && err.name ? err.name + ': ' : ''}${err && err.message ? err.message : err}`);
    }
  }

  async function fetchWasm() {
    const response = await fetch('./soundpaste.wasm', { cache: 'no-cache' });
    if (!response.ok) throw new Error(`modeemin lataus epäonnistui (${response.status})`);
    return response.arrayBuffer();
  }

  function loadWasm(buffer) {
    const seed = crypto.getRandomValues(new Uint32Array(2));
    const flags = params.get('kattely') === '1' ? 1 : 0;
    app.node.port.postMessage({ type: 'load', wasm: buffer, seed: [seed[0], seed[1]], flags }, [buffer]);
  }

  function fatal(message) {
    $('start-section').hidden = false;
    $('start').hidden = true;
    $('start-status').textContent = message;
    log(message);
  }

  // ------------------------------------------------------------------ viestit äänisäikeeltä

  function onWorkletMessage(m) {
    switch (m.type) {
      case 'ready':
        app.ready = true;
        app.maxMessage = m.maxMessage;
        app.sending = false;
        $('start-section').hidden = true;
        for (const id of ['meter-section', 'send-section', 'receive-section']) $(id).hidden = false;
        $('download').disabled = false;
        log(`modeemi valmis: ${m.sampleRate} Hz, viestin enimmäispituus ${m.maxMessage} tavua`);
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
          setTx(m.code === 1 ? 'Edellinen lähetys on vielä kesken.' : 'Viesti on liian pitkä.', false);
        }
        break;
      case 'recording': {
        const resolve = app.pendingRecording.get(m.tag);
        app.pendingRecording.delete(m.tag);
        if (resolve) resolve({ samples: new Float32Array(m.samples), sampleRate: m.sampleRate });
        break;
      }
      case 'error':
        log(`virhe äänisäikeessä: ${m.message}`);
        if (!app.ready) fatal(`Modeemin käynnistys epäonnistui: ${m.message}`);
        break;
    }
  }

  const FAIL_TEXT = [
    'Vastaanottajaa ei löytynyt.',
    'Vastaanottaja lakkasi vastaamasta kesken siirron.',
    'Siirto ei onnistunut uudelleenyrityksistä huolimatta.',
    'Vastaanottaja kuittasi eri sisällön kuin lähetettiin.',
  ];
  const FAIL_ADVICE = [
    'Varmista, että toisella koneella tämä sivu on auki ja Aloita on painettu, että koneet ovat lähekkäin ja että äänet ovat päällä. Kokeile lisätä äänenvoimakkuutta.',
    'Kokeile uudelleen. Jos ongelma toistuu, siirrä koneet lähemmäs toisiaan.',
    'Kokeile uudelleen hiljaisempana hetkenä tai siirrä koneet lähemmäs toisiaan.',
    'Kokeile uudelleen.',
  ];
  const HINT_ADVICE = [
    '',
    'Signaali on vastaanottajalla liian hiljainen: lisää äänenvoimakkuutta tai siirrä koneet lähemmäs toisiaan.',
    'Signaali leikkautuu vastaanottajan mikrofonissa: pienennä äänenvoimakkuutta tai siirrä koneet kauemmas toisistaan.',
    'Taustamelua on liikaa: yritä uudelleen hiljaisempana hetkenä tai siirrä koneet lähemmäs toisiaan.',
  ];

  function onEvent(b) {
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    switch (b[0]) {
      case 1: { // lähetyksen tila
        const [sub, a, x, c] = [b[1], dv.getUint32(2, true), dv.getUint32(6, true), dv.getUint32(10, true)];
        const text = [
          () => 'Kuunnellaan, onko kanava vapaa…',
          () => `Etsitään vastaanottajaa… (yritys ${a}/${x})`,
          () => `Lähetetään, kehys ${a}/${x}${c > 1 ? ' (uudelleenlähetys)' : ''}`,
          () => 'Odotetaan kuittausta…',
          () => `Kuittausta ei kuulunut, pyydetään uudelleen (${a}/${x})…`,
          () => `Yritetään uudelleen: vastaanottajalta puuttuu ${a} ${a === 1 ? 'kehys' : 'kehystä'}.`,
          () => 'Toinen kone lähettää juuri. Oma viesti lähtee heti sen jälkeen.',
        ][sub]();
        setTx(text, false);
        emit({ kind: 'tx', sub, a, b: x, c });
        break;
      }
      case 2: { // perillä
        const seconds = dv.getUint32(3, true) / 1000;
        setSending(false);
        setTx(`Perillä (${seconds.toFixed(1).replace('.', ',')} s)`, true);
        emit({ kind: 'delivered', seconds });
        break;
      }
      case 3: { // epäonnistui
        const [reason, hint, hasQuality] = [b[1], b[2], b[3]];
        setSending(false);
        setTx(`Epäonnistui: ${FAIL_TEXT[reason] || 'tuntematon syy.'}`, false);
        const advice = $('tx-advice');
        advice.textContent = HINT_ADVICE[hint] || FAIL_ADVICE[reason] || '';
        advice.hidden = !advice.textContent;
        emit({ kind: 'failed', reason, hint, quality: hasQuality ? { snrDb: dv.getInt8(4), levelDb: dv.getInt8(5), clipped: !!b[6] } : null });
        break;
      }
      case 4:
        setSending(false);
        setTx('Lähetys peruutettu.', false);
        emit({ kind: 'cancelled' });
        break;
      case 5: { // vastaanotto alkoi
        const len = dv.getUint32(3, true);
        $('rx-status').textContent = `Vastaanotetaan viestiä (${len} tavua)…`;
        $('rx-status').classList.remove('ok');
        const progress = $('rx-progress');
        progress.hidden = false;
        progress.value = 0;
        emit({ kind: 'rxStarted', len });
        break;
      }
      case 6: { // edistyminen
        const [have, of] = [dv.getUint32(1, true), dv.getUint32(5, true)];
        $('rx-status').textContent = `Vastaanotetaan: ${have}/${of} kehystä`;
        const progress = $('rx-progress');
        progress.hidden = false;
        progress.max = of;
        progress.value = have;
        emit({ kind: 'rxProgress', have, of });
        break;
      }
      case 7: { // vastaanotettu
        const len = dv.getUint32(3, true);
        const bytes = b.subarray(7, 7 + len);
        app.received = decoder.decode(bytes);
        const el = $('received');
        el.textContent = app.received; // aina pelkkänä tekstinä
        el.hidden = false;
        $('rx-progress').hidden = true;
        $('copy').hidden = false;
        $('copy-status').textContent = '';
        const status = $('rx-status');
        status.textContent = len === 0 ? 'Vastaanotettu tyhjä viesti.' : `Vastaanotettu ${len} tavua.`;
        status.classList.add('ok');
        emit({ kind: 'received', text: app.received, bytes: Array.from(bytes) });
        break;
      }
      case 8:
        $('rx-progress').hidden = true;
        $('rx-status').classList.remove('ok');
        $('rx-status').textContent = b[1] === 1 ? 'Lähettäjä peruutti lähetyksen.' : 'Lähettäjästä ei kuulunut enää: keskeneräinen viesti hylättiin.';
        emit({ kind: 'rxAborted', cancelled: b[1] === 1 });
        break;
      case 9:
        log(decoder.decode(b.subarray(1)));
        break;
    }
  }

  // ------------------------------------------------------------------ lähetys

  function setTx(text, ok) {
    const el = $('tx-status');
    el.textContent = text;
    el.classList.toggle('ok', !!ok);
    if (text) log(`tila: ${text}`);
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
    $('count').textContent = `${bytes} / ${app.maxMessage} tavua`;
    const warn = $('too-long');
    warn.hidden = !tooLong;
    if (tooLong) {
      warn.textContent = `Viesti on liian pitkä: ${bytes} tavua, enimmäispituus on ${app.maxMessage} tavua. Ääkköset ja emojit vievät useamman tavun.`;
      $('estimate').textContent = '';
    }
    $('send').disabled = !app.ready || app.sending || tooLong;
    if (app.ready && !tooLong) app.node.port.postMessage({ type: 'estimate', len: bytes });
  }

  function showEstimate(len, seconds) {
    if (encoder.encode($('text').value).length !== len) return;
    $('estimate').textContent = `Siirto kestää noin ${seconds < 10 ? seconds.toFixed(1).replace('.', ',') : Math.round(seconds)} s`;
  }

  function send(text) {
    if (!app.ready || app.sending) return false;
    const bytes = encoder.encode(text);
    if (bytes.length > app.maxMessage) return false;
    $('tx-advice').hidden = true;
    setSending(true);
    setTx('Aloitetaan…', false);
    log(`lähetetään ${bytes.length} tavua`);
    app.node.port.postMessage({ type: 'send', bytes: bytes.buffer }, [bytes.buffer]);
    return true;
  }

  // ------------------------------------------------------------------ signaalitaso

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
    $('meter-db').textContent = db <= -99 ? 'ei signaalia' : `${db.toFixed(0)} dB`;
    $('meter-hint').textContent = hot
      ? 'Mikrofoni leikkaa: ääni on liian voimakas.'
      : db <= -99
        ? 'Mikrofonista ei tule lainkaan ääntä. Tarkista, ettei mikrofoni ole mykistetty.'
        : '';
    const s = app.stats;
    if (s && $('debug').open) {
      $('stats').textContent = `Alkumerkkejä ${s.preambles}, ehjiä kehyksiä ${s.framesOk}, hylättyjä ${s.framesFailed}, lähetettyjä ${s.framesSent}, korjattuja tavuja ${s.corrected}` + (s.framesOk ? `, viimeisin SNR ${s.snrDb.toFixed(0)} dB` : '');
    }
  }

  // ------------------------------------------------------------------ äänite

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
    log(`äänite ladattu: ${(samples.length / sampleRate).toFixed(1)} s, ${sampleRate} Hz`);
  }

  // ------------------------------------------------------------------ tapahtumat

  $('start').addEventListener('click', start);
  $('text').addEventListener('input', updateInput);
  $('send').addEventListener('click', () => send($('text').value));
  $('cancel').addEventListener('click', () => {
    if (app.node) app.node.port.postMessage({ type: 'cancel' });
  });
  $('copy').addEventListener('click', async () => {
    // Leikepöydälle kirjoittaminen vaatii käyttäjän toiminnon, joten se tehdään vain napista.
    try {
      await navigator.clipboard.writeText(app.received);
      $('copy-status').textContent = 'Kopioitu.';
    } catch (err) {
      $('copy-status').textContent = 'Kopiointi ei onnistunut. Valitse teksti ja kopioi käsin.';
    }
  });
  $('download').addEventListener('click', downloadRecording);
  $('clear-log').addEventListener('click', () => {
    logLines.length = 0;
    $('log').textContent = '';
  });
  document.addEventListener('visibilitychange', () => {
    $('hidden-warning').hidden = !(document.hidden && app.ready);
    if (app.ready) log(document.hidden ? 'välilehti taustalle' : 'välilehti näkyviin');
  });

  // Kehitystyökalujen kiinnityskohta. Tuotantosivulla tätä ei käytä mikään,
  // eikä mikään tässä tiedostossa ota yhteyttä verkkoon.
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
