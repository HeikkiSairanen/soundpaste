// soundpaste: testiaseman koodi. VAIN KEHITYSKÄYTTÖÖN.
//
// Kehityspalvelin (soundpaste serve) lisää tämän sivulle. Julkaistavassa
// versiossa tätä tiedostoa ei ole, eikä tuotantosivu ota yhteyttä mihinkään.
//
// Testiasema on passiivinen: käyttöönoton jälkeen siihen ei tarvitse koskea.
// - Tila URL-parametrista ?testi=vastaanotto | kaiku | majakka (&vali=20), nimi &asema=pc
// - Tarkistaa kahden sekunnin välein, onko uusi versio: uusi wasm vaihdetaan
//   lennossa (mikrofoni pysyy päällä), muuttunut sivu ladataan uudelleen.
// - Lähettää lokin ja jokaisen session mikrofoniäänitteen kehityspalvelimelle.
// - Hakee komennot palvelimelta (soundpaste remote ...).

(() => {
  'use strict';
  const sp = window.soundpaste;
  const params = new URLSearchParams(location.search);
  const station = (params.get('asema') || 'asema').replace(/[^a-z0-9-]/gi, '').toLowerCase() || 'asema';
  let mode = params.get('testi') || 'vastaanotto';
  let beaconSeconds = Math.max(8, Number(params.get('vali')) || 20);
  const BEACON_TEXT = 'https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd';
  const device = navigator.userAgent.replace(/^Mozilla\/5\.0 /, '').slice(0, 120);

  // Näkyvä merkki siitä, että tämä on testiasema.
  const banner = document.createElement('p');
  banner.className = 'notice warning';
  document.querySelector('main').prepend(banner);
  const showBanner = () => {
    banner.textContent = `TESTIASEMA "${station}", tila: ${mode}${mode === 'majakka' ? ` (${beaconSeconds} s välein)` : ''}. Lokit ja mikrofoniäänitteet lähetetään kehityspalvelimelle.`;
  };
  showBanner();

  // ---------------------------------------------------------------- loki palvelimelle
  let pendingLog = [];
  const post = (path, body, type) => fetch(path, { method: 'POST', body, headers: type ? { 'Content-Type': type } : undefined }).catch(() => {});
  setInterval(() => {
    if (!pendingLog.length) return;
    const text = pendingLog.join('\n') + '\n';
    pendingLog = [];
    post(`./dev/log?asema=${station}`, text, 'text/plain');
  }, 2000);

  // ---------------------------------------------------------------- äänitteet
  let sessionStart = 0; // aseman kello (ms), jolloin sessio alkoi
  let lastSentText = '';

  async function uploadRecording(kind, query, seconds) {
    const { samples, sampleRate } = await sp.recording();
    const keep = Math.min(samples.length, Math.round(seconds * sampleRate));
    const wav = sp.wavBytes(samples.subarray(samples.length - keep), sampleRate);
    const q = new URLSearchParams({ asema: station, tyyppi: kind, laite: device, ...query });
    await post(`./dev/aanite?${q}`, wav, 'audio/wav');
  }

  function sessionSeconds() {
    const now = sp.state().stationNow;
    return Math.min(44, Math.max(4, (now - sessionStart) / 1000 + 3));
  }

  // ---------------------------------------------------------------- tapahtumat
  let beaconTimer = null;
  function scheduleBeacon() {
    clearTimeout(beaconTimer);
    if (mode !== 'majakka') return;
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
        // Äänite talteen hetken päästä, jotta loppukuittauskin on mukana.
        setTimeout(() => uploadRecording('sessio', { teksti: e.text, ok: '1', huom: `tila ${mode}, vastaanotettu` }, sessionSeconds()), 1200);
        if (mode === 'kaiku') {
          // Kaikutila: sama teksti takaisin uutena viestinä.
          setTimeout(() => {
            lastSentText = e.text;
            sp.send(e.text);
          }, 2500);
        }
        break;
      case 'rxAborted':
        setTimeout(() => uploadRecording('sessio', { teksti: '', ok: '0', huom: `tila ${mode}, vastaanotto keskeytyi` }, sessionSeconds()), 500);
        break;
      case 'failed':
        setTimeout(() => uploadRecording('sessio', { teksti: '', ok: '0', huom: `tila ${mode}, lähetys epäonnistui (syy ${e.reason})` }, sessionSeconds()), 500);
        break;
    }
  });

  // ---------------------------------------------------------------- versiontarkistus
  let version = null;
  async function checkVersion() {
    try {
      const v = await (await fetch('./dev/version', { cache: 'no-store' })).json();
      if (!version) {
        version = v;
        return;
      }
      if (v.page !== version.page) {
        sp.log('sivu on muuttunut, ladataan uudelleen');
        setTimeout(() => location.reload(), 300);
        return;
      }
      if (v.wasm !== version.wasm && sp.state().ready && !sp.state().sending) {
        version = v;
        const buffer = await (await fetch('./soundpaste.wasm', { cache: 'no-store' })).arrayBuffer();
        sp.log(`uusi modeemi (${buffer.byteLength} tavua) vaihdetaan lennossa`);
        sp.reloadWasm(buffer);
      }
    } catch (err) {
      // Palvelin ei ole päällä: yritetään myöhemmin uudelleen.
    }
  }
  setInterval(checkVersion, 2000);
  checkVersion();

  // ---------------------------------------------------------------- komennot
  // Kanavamittauksen pyyhkäisy: sama signaali kuin komennolla soundpaste measure.
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
    sp.log(`soitetaan pyyhkäisy, amplitudi ${amplitude}`);
  }

  let commandSeq = 0;
  async function pollCommands() {
    try {
      const state = `${mode}${sp.state().ready ? '' : ' (ei käynnissä)'}`;
      const r = await (await fetch(`./dev/commands?after=${commandSeq}&asema=${station}&tila=${encodeURIComponent(state)}`, { cache: 'no-store' })).json();
      commandSeq = r.seq;
      for (const line of r.commands) runCommand(line);
    } catch (err) {
      // ei yhteyttä
    }
  }
  setInterval(pollCommands, 1000);

  function runCommand(line) {
    const [cmd, ...rest] = line.trim().split(/\s+/);
    sp.log(`komento: ${line}`);
    if (cmd === 'tila' && rest[0]) {
      mode = rest[0];
      if (rest[1]) beaconSeconds = Math.max(8, Number(rest[1]) || beaconSeconds);
      showBanner();
      scheduleBeacon();
    } else if (cmd === 'laheta') {
      lastSentText = line.trim().slice(cmd.length + 1);
      sp.send(lastSentText);
    } else if (cmd === 'peruuta') {
      sp.cancel();
    } else if (cmd === 'pyyhkaisy') {
      playSweep(Number(rest[0]) || 0.3);
    } else if (cmd === 'aanita') {
      // Äänitä N sekuntia tästä eteenpäin ja lähetä (esim. Macin soittama pyyhkäisy).
      const seconds = Math.min(40, Number(rest[0]) || 8);
      const kind = rest[1] || 'muu';
      const gain = rest[2] || '0.3';
      setTimeout(() => uploadRecording(kind, { gain, huom: rest.slice(3).join(' ') }, seconds), seconds * 1000);
    } else if (cmd === 'lataa') {
      location.reload();
    }
  }

  // ---------------------------------------------------------------- automaattinen käynnistys
  // Testiaseman selain käynnistetään lipuilla, jotka sallivat äänen ja mikrofonin
  // ilman napinpainallusta (ks. README). Jos ne puuttuvat, Aloita-nappi toimii.
  window.addEventListener('load', () => setTimeout(() => sp.start(), 300));
})();
