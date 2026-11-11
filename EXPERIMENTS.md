# Experiment log

For every experiment the hypothesis, the change, the measurement result and the
conclusion are recorded, also when the experiment failed. Newest at the bottom.

The metrics come from the command `soundpaste bench` (see CLAUDE.md). SNR is the
power ratio of signal to noise at the microphone over the whole band 0..fs/2.
"Min SNR" is the lowest point of the 3 dB grid at and above which at least 90 %
of transfers succeed.

The recording corpus (`corpus/`) referred to below is local-only: it consists of
room recordings and is not in the repository.

## 0. Baseline: v0-bfsk (2026-11-11)

**Purpose.** Get loop A running with the simplest possible modem. A good modem
is not the goal.

**Implementation.** 2-FSK (2000/3000 Hz), 100 baud, an alternating preamble and
a 16-bit sync word, CRC-16, no error correction. One-way, one frame.

**Result** (`bench/results/20261111T075213Z-no-commit-v0-bfsk.json`, 12 repetitions/point):

| metric | value |
|---|---|
| transfer time, 60-character link (one frame, no acks) | 5.54 s |
| success in nominal conditions (SNR >= 12 dB) | 84.5 % |
| min SNR, clean channel | -9 dB |
| min SNR, room (RT60 0.35 s) | +21 dB |
| min SNR, reverberant (RT60 0.8 s) | does not reach 90 % at any level |
| BER before correction, nominal conditions | 7.5e-3 |
| net rate | 10.8 bytes/s |

**Conclusion.**
- Noise is not v0's problem: on a clean channel it works down to -9 dB,
  because the 10 ms symbol collects power from a narrow band.
- Echo is the problem. In the room, success stays at about 90 % regardless of
  the signal level (the errors do not decrease as SNR grows), and in the
  reverberant room nothing gets through. The echo of the previous symbol lands
  on the same frequency as the current symbol, and with two frequencies there
  is nowhere to dodge.
- A single bit error in the 504-bit frame brings down the whole frame, because
  there is no error correction.
- Next experiments (stage 4): more frequencies and changing frequency between
  symbols against echo, Reed-Solomon, better synchronisation.

## 1. Real audio on the Mac: channel measurement and v0 from speaker to microphone (2026-11-11)

**Purpose.** Get loops B and C running and measure the channel before the
design decisions. Device: MacBook Pro (MacBookPro17,1), its own speakers and
its own microphone, system volume 44 %, home office.

**Channel measurement** (`soundpaste measure`, sweep 100 Hz – 20 kHz, three
amplitudes 1.0 / 0.3 / 0.1; the quietest one is holdout material):

| quantity | value | significance |
|---|---|---|
| gain 1–8 kHz | -25…-33 dB, flat ±4 dB | usable band |
| gain 10–20 kHz | -37…-42 dB | about 12 dB weaker, but works up to 20 kHz |
| background noise / 50 Hz | -78 dBFS @ 200 Hz, -98 @ 1 kHz, -105 @ 2–8 kHz | the noise is at low frequencies |
| direct sound / echo | 11–12 dB | the own microphone is right next to the speaker |
| delay spread | 3–4 ms, coherence bandwidth about 50–70 Hz | the channel is frequency-selective even within one machine |
| energy > 10 / 20 / 40 ms after the direct sound | -14 / -20 / -27 dB | intersymbol interference |
| own echo at the noise floor (-55 dB) | 62 ms | dimensioning of the guard time |
| second harmonic distortion | -40 dB (amplitude 1.0), in one measurement -24 dB | limits dense constellations |

The RT60 estimate (0.05–0.17 s) is not reliable: the echo tail drowns in the
background noise before it has decayed 25 dB, because the direct sound is so
dominant. The measurement describes the path of the own echo, not the channel
heard by another machine. The other machine is estimated with the derived
profile `:far` (direct sound -14 dB).

**v0 over real audio** (`soundpaste acoustic`, five levels):

| amplitude | 1.0 | 0.3 | 0.1 | 0.03 | 0.01 |
|---|---|---|---|---|---|
| result | ok | ok | ok | ok | not detected |

**Observations.**
- On the first attempt three out of four failed, and the error was every time
  in the last bit of the frame: SoX's `play` cuts the sound off at the end.
  The fix is in the measurement tool (silence at both ends of the playback),
  but the lesson also concerns the modem: the last symbol of a frame must not
  be the last thing the speaker plays.
- The signal level at the microphone is low (-33 dBFS at full amplitude), so
  clipping is not a problem on this device, but level-independent detection is
  essential.
- With the measured channel and the measured noise, v0 still works at an SNR
  of -15 dB, because the noise is outside the band. Whole-band SNR is thus a
  poor metric for a real room, and the choice of band matters a lot.
- With the derived `:far` profile v0 stays at 50–60 % regardless of level: the
  echo brings it down, as in the synthetic rooms.

**Baseline** updated: `bench/results/20261111T081049Z-b1d2c55-dirty-v0-bfsk.json`
(measured channels included, corpus 4/5).

## 2. v1: multitone MFSK, tone hopping, chirp preamble, Reed–Solomon (2026-11-11)

**Hypothesis.** The waveform of DESIGN.md (16 × 4-FSK, hopping over 4 groups,
40 + 10 ms, chirp + relative threshold, RS with parity 1/2) shortens the
60-character frame to about 1.5 seconds and works in the reverberant room,
where v0 does not work at all.

**Result** (`bench/results/20261111T083553Z-d9cbeba-dirty-v1.json`):

| metric | v0 | v1 |
|---|---|---|
| transfer time, 60-character frame | 5.54 s | 1.44 s |
| success in nominal conditions | 86.3 % | 94.7 % (100 % without the new hall profile) |
| min SNR, room | +21 dB | -6 dB |
| min SNR, reverberant (RT60 0.8 s) | does not reach | -3 dB |
| net rate | 10.8 bytes/s | 41.7 bytes/s |
| corrected bytes / frame in nominal conditions | – | 0.7 (capacity 16) |
| real audio on the Mac, amplitudes 1.0…0.01 | 4/5 | 5/5 |

The hypothesis held. There is a lot of margin in nominal conditions: less than
5 % of the error correction is used.

**A bug along the way: the preamble triggered on a sidelobe.** The first
version failed on the measured channels exactly when there was the least noise
(SNR 24–30 dB: 42–83 %). The cause: when the background is nearly silent, the
relative threshold is already exceeded by a sidelobe of the chirp's
autocorrelation tens of milliseconds before the main peak, and the peak search
looked only 15 ms ahead. The fix: from the threshold crossing, the strongest
peak is searched up to one preamble length ahead. After this the measured
channels are at 100 % at all levels.
The lesson: a "too good" signal is a test case of its own.

**New profiles**, because the old ones no longer distinguished the versions:
`thump` (three impacts during the frame), `speech` (0.5 s of speech-like
interference) and `hall` (RT60 1.2 s, echo 5 dB stronger than the direct
sound).

**Variants** (same instrumentation, min SNR / success):

| variant | reverberant | hall (SNR >= 12 dB) | other | decision |
|---|---|---|---|---|
| v1 (base, erasures on) | -3 dB | 8–33 % | | **chosen** |
| `v1-noera`: no erasures | 0 dB | | otherwise the same | rejected: erasures are a free 3 dB at the edge |
| `v1-8x20`: 8 subchannels, 20 + 5 ms | -3 dB | 0–8 % | same rate | rejected: a long symbol withstands a long echo better |
| `v1-g2`: hopping over 2 groups | +27 dB | 0 % | half the band | rejected: confirms the result of the comparison experiment |
| `v1-time`: byte = consecutive symbols | 0 dB | | thump the same | rejected: no benefit, slightly worse in reverberant |
| `v1-p3`: parity 1/3 | 0 dB | 0–25 % | 1.29 s; speech -3 dB | rejected for now: 10 % faster but 3 dB more sensitive |

**Where the limit lies.**
- Noise: about -6 dB whole-band SNR (-9 dB on a clean channel).
- Echo: RT60 0.8 s and direct/echo 0 dB works. `hall` (RT60 1.2 s, direct
  sound 5 dB weaker than the echo) does not work: the preamble and the header
  get through, but the payload errors exceed the Reed–Solomon capacity (e.g.
  18 to correct when the limit is about 16). Retransmission does not help,
  because the channel does not change. This is the case that the robust mode
  of DESIGN.md section 7 (longer symbols, more parity) would be meant for.
- Thumps and speech-like interference do not bring down a frame in simulation
  even at the -6 dB level. The profiles may be too easy; real speech and real
  thumps are tested in loop E.
- False alarms: a frequency sweep (channel measurement) and v0's sounds
  trigger the preamble once or twice per recording, but the header check
  rejects them in 0.2 seconds. White noise (20 s) does not trigger it even
  once.

**Tested:** simulation (15 profiles) and real audio from the Mac's speaker to
its own microphone (5 recordings in the corpus). Not yet between two machines.

## 3. Link layer: burst or per-frame ack, handshake or inline start (2026-11-11)

**Measurement method.** The bench run moved to session mode: two stations, a
separate room response for each direction, the leak of the own speaker into
the own microphone (four times stronger than the other station), randomly
drawn audio latencies (output 20–250 ms, input 10–150 ms), different sample
rates and clock error according to the profile. Success requires that the
sender got the confirmation and the receiver showed exactly the right message
exactly once.

**Hypothesis 1.** A burst with a combined ack is, for a long message, about a
third faster than per-frame acks (DESIGN.md: 15.5 s vs. 23 s).

| 512 bytes, 6 seeds | burst | 3 frames/ack | per-frame |
|---|---|---|---|
| transfer time | 15.4–16.1 s | 17.1–18.6 s | 21.5–24.8 s |

The same result with the profiles `room` and `reverberant`, SNR 20 dB and 0 dB
(no retransmissions in any run, so the time depends only on the audio
latencies). The hypothesis held, and the calculation was on target.
**Decision: burst.** This has not been measured on a channel where frames are
lost often; there a shorter burst could win, because a lost ack request costs
less.

**Hypothesis 2.** Sending a short message along with the start frame (inline
start) shortens the 60-character link to about 2.6 seconds without weakening
success.

| 60-character link, 15 profiles | handshake | inline start |
|---|---|---|
| transfer time, median in nominal conditions | 4.24 s | 2.82 s |
| success in nominal conditions | 93.6 % | 93.9 % |
| min SNR, room / reverberant | -6 / 0 dB | -6 / 0 dB |
| no receiver | about 8–11 s | about 8–11 s |

The hypothesis held. The fear was that a missing receiver would be discovered
more slowly, because the start frame is longer. The solution: only the first
start carries the message, and the retries are short. **Decision: inline start
as the default.** Baseline updated:
`bench/results/20261111T085404Z-0cf87ca-dirty-v1.json`.

**Findings during implementation.**
- The receiver sent the ack twice when the preamble of the ack request (POLL)
  was interpreted as a starting data burst. Fixed: a non-data frame resets the
  burst observation. Found with a frame-level test.
- An exactly simultaneous send is not always resolved: the random waits must
  differ by more than the preamble detection delay (about 0.2 s). The backoff
  interval was widened from 0.3–1.2 seconds to 0.2–2.2 seconds, whereby more
  than 93 % are resolved within three attempts. Recorded as a known
  limitation.
- The transfer time estimate (DESIGN.md section 6) matched the measured one to
  within a few percent.

**Tested:** in simulation only (frame level: 33 tests; audio level: 8 session
tests and the bench run). The link layer has not yet been run over real audio.

## 4. Loop D: CLI and browser on the same Mac over real audio (2026-11-11)

**Purpose.** Verify the wasm wrapper, the AudioWorklet glue and the link layer
over real audio before the second machine. The Mac's CLI (`soundpaste live`,
audio via SoX) and the page in Chrome share the same speaker and microphone.

| test | result |
|---|---|
| CLI -> browser, 60-character link (inline start) | delivered in 2.59 s, no bytes to correct |
| browser -> CLI, 43 bytes with non-ASCII letters and emoji | delivered in 2.2 s (browser clock) |
| CLI -> browser, 300 bytes (handshake + burst of 5 frames) | delivered in 9.96 s, 5 corrected bytes |
| swapping the wasm module on the fly twice | the microphone stayed on, the transfer worked after the swap (1.74 s) |

**Observations.**
- Everything worked on the first attempt. The simulation's estimates (2.8 s
  and about 10 s) were right; real audio was slightly faster, because the
  output latency reported by Chrome is only about 5 ms.
- Chrome turned the speech processing off as requested (echo cancellation,
  noise suppression and automatic gain: false).
- The own echo was recognised at both ends, even though the two stations share
  the same speaker.
- The SNR measured by the browser was 23–30 dB and the level -39 dBFS at a
  system volume of 44 %.

**Scope.** This is still the speaker and microphone of a single device. The
channel between two machines (loop E) has not yet been run.

## 5. Loop E, first two-machine run: false alarms and a quiet return direction (2026-11-11)

**Setup.** The published page (sairanen.org/soundpaste) in the PC's browser, a
command-line station on the Mac (`soundpaste live`) that recorded for four
minutes. The user sent messages from the PC. Recording: `devdata/diag-pc.wav`
(not in version control), three clips of it in the corpus
(`...-pc-mac-1..3-sessio-v1`).

**A misstep.** I interpreted the first "delivered" results as the PC's acks,
but the acks came from the Mac's own browser, where the page was also open.
This was revealed by the levels in the recording: the ack was heard at exactly
the same level (-39 dBFS) as the own transmission. The lesson: in a
two-machine test, identifying the responder requires the level or a log, not
just the ack.

**Observations from real data.**

| observation | figure |
|---|---|
| PC -> Mac: start frames decoded | all, SNR 17–27 dB, level -36…-44 dBFS |
| Mac -> PC: the ack was at first not heard | the PC repeated the same start 3 times; fixed itself when the volume was raised |
| duplicate recognition | worked: the message was shown once on the Mac although the start arrived three times |
| false preamble detections | 246 in four minutes (in clusters, keyboard typing) |
| needless ack transmissions because of false detections | 6 during one unfinished session |

**Hypothesis.** A click exceeds the relative power threshold because it is
preceded by silence, but it does not look like a chirp. The check "how large a
part of the band energy of a preamble-length window is explained by the chirp"
(pure chirp 1, a click close to 0) removes the false alarms without weakening
sensitivity.

| threshold (`chirp_fraction`) | false alarms in the recording | real frames (44) | min SNR in simulation |
|---|---|---|---|
| 0 (no check) | 246 | 44 | baseline |
| 0.03 | 110 | 44 | – |
| 0.06 | 2 | 44 | the same in all profiles |
| 0.08 (chosen) | 0 | 44 | the same |
| 0.12 | 0 | 44 | the same in all profiles |

The hypothesis held. In simulation the threshold had no cost in any profile.
The chosen 0.08 leaves room in both directions, but it has been tuned on the
basis of a single recording: the limit for a weak real signal in echo is
unmeasured over real audio.

**Other changes.**
- Link layer: a mere preamble detection no longer triggers the silence-based
  ack. Only a frame whose header decoded counts as a sign of an ongoing burst.
- Transmit level +5 dB (RMS 0.25 -> 0.45, clip limit 0.5 -> 0.9), because the
  return direction was too quiet. This does not replace raising the volume,
  but it moves the limit.
- Rejected frames are now shown in the log with their levels (troubleshooting
  view).

**Result.** The bench run is unchanged (2.81 s, 93.9 %, no regressions),
corpus 6/6 with this waveform. Baseline updated.

**Open.** There are no logs or recordings from the PC's end (the published
page does not send them), so the level of the Mac -> PC direction, the PC's
microphone processing and the channel response are unmeasured. They require
the test station (README, loop E).

## 6. Translation to English (no functional change)

The project (code comments, CLI output, UI, docs) was translated from Finnish
to English. Channel profiles were renamed (`huone` -> `room`, `kaikuisa` ->
`reverberant`, `halli` -> `hall`, and so on), and the names in earlier result
files under `bench/results/` were translated in place so that `compare` keeps
working across the change.

The bench seeds are hashed from the profile name, so the renamed profiles draw
different noise. With 6 runs per point the numbers moved within that noise:
transfer time 2.81 -> 2.85 s, success rate 93.9 -> 94.2 %, and `clean` at
-9 dB went from passing to 5/6 (83 %), which `compare` reports as a regression
of the minimum SNR from -9 to -6 dB. No modem code changed (the diff in
`modem-core` is comments and string literals only), so this is taken as seed
noise and the baseline was updated. The labels in the stored modulation
comparison outputs in `bench/experiments/` were translated in place; their
numbers are the original ones.
