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

## 7. Ultrasonic mode: the same waveform on 17.6–20.0 kHz (2026-11-17)

**Motivation.** The user asked for a mode that uses only the highest pitches,
switchable from the page, so that a transfer is (nearly) silent to people.

**What the existing data says.** The channel measurements of entry 1
(MacBook Pro, own speaker -> own microphone) were re-analysed above 15 kHz
from the impulse responses. Relative to the current band (1–7.4 kHz):
15–19.5 kHz is -8 to -12 dB and flat within ±2 dB, 19.5–20 kHz -12 dB. The
measurement sweep stops at 20 kHz, so nothing is known above that. Background
noise up there is much lower than in the speech band, so the per-tone SNR of
the third-octave band at 20 kHz (62 dB) is better than at 1 kHz (40 dB).

**Choice.** 20 kHz is the hard upper limit anyway: a 44.1 kHz codec's
anti-alias filter passes only up to about 0.45·fs = 19.8–20 kHz. The band is
2.4 kHz wide: 96 tones at the same 25 Hz spacing = K 6 × M 4 × G 4, from 17.6
to 20.0 kHz. Everything else (symbol timing, chirp, Reed–Solomon) is the v1
waveform, so ultrasonic is one more `MfskParams` preset (`v1-us`). K drops
from 16 to 6, so the raw rate drops to 6/16 = 38 % of v1.

**Hypothesis.** (1) In simulation `v1-us` reaches the same SNR limits as v1
in the room profiles (the simulator's noise is white or pink, so the
narrower band neither helps nor hurts much at the same in-band SNR), and a
60-character transfer takes about 2–2.5 × as long. (2) The byte layout
matters more in ultrasonic than in v1: the real response near the band edge
(19.5–20 kHz, and above 19.8 kHz on 44.1 kHz devices) can kill a whole
subchannel. With `Freq` one dead subchannel out of 6 touches most bytes;
with `Time` it ruins only a sixth of them, which Reed–Solomon with erasures
covers. Expect `Time` to be the right layout for ultrasonic.

**Finding on the way: clipping is audible.** The first version kept v1's
level (RMS 0.45, peaks clipped at 0.9). Hard clipping a sum of high tones
spreads distortion over the whole spectrum: averaged over the frame, the
energy below 12 kHz was only 27 dB under the signal (worst 20 ms window
-22 dB), i.e. an audible hiss and crackle, and at 44.1 kHz the products also
alias. Six tones peak at most at 6/√3 ≈ 3.5 × RMS, so `v1-us` sends at RMS
0.26 and never clips. After that the energy below 12 kHz is -92 dB and
12–15 kHz -82 dB (worst window) relative to the signal. What is left at
15–17 kHz (worst window -52 dB) is the 2 ms edges of the chirp. The level is
4.8 dB below v1, which the channel's 13 dB better per-tone SNR up there
covers on the measured Mac.

**Result, simulation (session bench, 6 runs/point, not saved).**

| | v1 (baseline) | v1-us (`Time`) | v1-us-freq |
|---|---|---|---|
| 60-char link | 2.85 s | 5.95 s | 5.95 s |
| success, nominal | 94.2 % | 95.0 % | 95.8 % |
| min SNR, room | -6 dB | -6 dB | -6 dB |
| measured Mac profiles | -9 dB | -9 dB | -9 dB |
| net rate | 21.1 B/s | 10.1 B/s | 10.1 B/s |

Hypothesis (1) held: same SNR limits, 2.1 × the transfer time.

**Result, band edge.** Encoded at 44.1 kHz, brick-wall low-pass, white noise
at the given SNR in the modem band, 8 seeds, file decode:

| cutoff | layout | 6 dB | 3 dB | 0 dB | -3 dB |
|---|---|---|---|---|---|
| none | Time | 8/8 | 8/8 | 8/8 | 8/8 |
| none | Freq | 8/8 | 8/8 | 8/8 | 8/8 |
| 19.8 kHz | Time | 8/8 | 8/8 | 8/8 | 6/8 |
| 19.8 kHz | Freq | 0/8 | 0/8 | 0/8 | 0/8 |
| 19.5 kHz | Time | 8/8 | 8/8 | 8/8 | 5/8 |
| 19.5 kHz | Freq | 0/8 | 0/8 | 0/8 | 0/8 |

Hypothesis (2) held decisively: with the `Freq` layout a single lost
subchannel loses every frame (the header too), with `Time` it costs a few dB
at most. `v1-us` uses `Time`; `v1-us-freq` stays as a comparison preset.

**Other checks.** Duplex tests: a 512-byte ultrasonic message between a
48 kHz and a 44.1 kHz station with 100 ppm clock error is delivered (the
link layer's timeouts did not need changes), and a normal-mode station does
not even detect an ultrasonic preamble (the bands do not overlap). v1 bench
unchanged, 0 regressions.

**In the page.** A switch ("Ultrasonic") above Start, remembered in the
browser, `?mode=ultrasonic|normal` overrides. Switching rebuilds the station
in the running worklet (flag bit 1 of `sp_new`); it is disabled during a send
or receive. Below 44 kHz the station falls back to normal mode and the page
says so.

**Open.** Nothing over real audio yet. The Mac's response is measured only
up to 20 kHz and only speaker -> own microphone at 0 m; at a distance and on
the PC the band edge is unknown, and many PC microphones (and any that the
browser resamples to 16 kHz) will not pass 17.6 kHz at all. Next step:
`soundpaste acoustic --phy v1-us` on the Mac (with permission), then the two
computers. Whether the 2 ms chirp edges are audible as a tick also needs ears.

**First real-audio attempt (void).** `acoustic --phy v1-us` at gains 1.0–0.01
detected nothing, and a `v1` control at 0.3 failed as well. The cause was
macOS output being muted (volume 55, `output muted:true`), not the waveform.
The recordings and the channel measurement made in that state were taken out
of the corpus. One useful by-product: in the November sweep recording the
sweep stands 47–58 dB above the background all the way to 19.8 kHz, so this
Mac's speaker and microphone do pass the ultrasonic band.

**Real audio on the Mac (loop B, output unmuted).** MacBook Pro, own speaker
-> own microphone, system volume 55, same study as entry 1:

| gain | v1-us | v1 (today) | v1 (2026-11-11) |
|---|---|---|---|
| 1.0 | ok, 36.9 dB | | |
| 0.3 | ok, 36.0 dB | ok, 28.1 dB | |
| 0.1 | ok, 34.2 dB | | ok, 27.9 dB |
| 0.03 | ok, 25.3 dB | | ok, 26.0 dB |
| 0.01 | ok, 23.2 dB | | |

5/5, no corrected bytes at any gain. At the same gain, one minute apart, v1-us
had 8 dB more SNR than v1. A fresh channel measurement shows why: the room was
noisier than in November in the speech band (2 kHz in the silence before the
sweep: 68 dB vs 37 dB), while 17–20 kHz was as quiet as before. The speaker
-> microphone response at 17–19.5 kHz matched November's within a few dB
(-5 to -10 dB relative to 1–7.4 kHz, 19.5–20 kHz -14 dB).

**Still open.** This is one machine at 0 m talking to itself. Two computers
(the PC's speaker and microphone above 17.6 kHz) and distance are unmeasured,
and so is audibility (the chirp edges, anything a speaker's nonlinearity
folds down) — that needs ears.

## 8. Band ladder: negotiate in the normal band, send on the highest band that works (2026-11-17)

**Motivation.** The user asked for transfers to start in the normal band and
then test how high the pair of computers can go, replacing the manual switch
of entry 7. Two machines differ: a PC microphone may stop at 16 kHz, a
Bluetooth or voice-processed path at 8 kHz.

**Design** (DESIGN.md 5.9). Four bands: `v1` 1–7.4 kHz and three rungs with
the `v1-us` waveform (K 6, `Time` layout, no clipping) at 12.0–14.4 (`v1-b12`),
14.8–17.2 (`v1-b15`) and 17.6–20.0 kHz (`v1-us`). Every station listens on all
bands at once. The first START goes out on all bands simultaneously (copies
aligned to end together, each at 1/√4 of its level); the receiver answers
START_ACK on every band where it heard the START, with a per-band SNR report.
The sender picks the highest band that both directions decoded with an SNR
estimate of at least 12 dB, sends the data there, and the receiver answers in
the band it was addressed in. Fallbacks: a retried START is band 0 only at full
level; if polls on a high band go unanswered, the sender drops to band 0.
Inline start is off when negotiating (it would carry the message in band 0).

Simultaneous rather than sequential probes: the same information in the time
of the longest copy, and the 6 dB level split is a built-in margin (a band that
decodes at -6 dB has room at full level).

**Threshold.** `sim --profile room`, v1-us: decodes at a demodulator SNR
estimate of about 10 dB and fails at 7.5 dB (the estimate runs about 20 dB
above the bench's whole-band SNR and saturates around 26 dB in a room). A
probe heard at >= 12 dB at -6 dB level means roughly 8 dB of margin at full
level.

**Cost.** One demodulator takes 0.3 s of CPU per 60 s of audio natively
(0.5 %); four in parallel stay small in wasm as well.

**Hypothesis.** (1) In the flat simulator the ladder always ends on the top
band, and the 60-char link takes about 8 s (START and START_ACK on the K 6
bands are about 1.2 s and 0.9 s, the data frame about 4 s), against 5.95 s for
manual ultrasonic with inline start and 2.85 s for v1. (2) Success in nominal
conditions is no worse than v1's. (3) With a receiver path cut off at 16 kHz
or at 8 kHz, the ladder settles on the 12–14.4 kHz band or the normal band
respectively, and the message gets through.

**Result, simulation.** Session bench `--phy ladder` (6 runs/point, saved as
`bench/results/20261117T114147Z-7bb5040-dirty-ladder.json`):

| | v1 (baseline) | ladder |
|---|---|---|
| 60-char link | 2.85 s | 8.64 s |
| success, nominal | 94.2 % | 96.8 % |
| min SNR, room | -6 dB | -6 dB |
| measured Mac profiles (incl. today's) | -9 dB (today's: -6 / +3 dB) | -9 dB |

Hypotheses (1) and (2) held: 8.6 s against the predicted 8 s, success no
worse. In the flat simulator the choice is all or nothing: `room` stays on band
0 up to a bench SNR of 0 dB and climbs to band 3 from 3 dB up (`reverberant`:
3 and 6 dB). On band 0 the transfer is faster, 5–6 s. The middle rungs only
matter on frequency-limited paths: duplex tests with the receiver's
microphone cut at 14.6 kHz and at 9 kHz end on band 1 and band 0, and A's
microphone cut at 17.4 kHz ends on band 2 (hypothesis 3 held). The link
tests cover the threshold, a peer without the ladder, the merge of START
copies, the band-0-only retry and the fallback after silence on a high band.

**Result, real audio (loop D, Mac CLI <-> browser on the same Mac).** Both
directions chose band 3 and delivered: CLI -> browser 7.4 s and 9.8 s,
browser -> CLI 7.3 s. Reports (dB per band 0/1/2/3): the browser heard the
CLI's START at 16/19/27/30, the CLI heard the browser's at 26/20/28/31. The
session recording decodes in the corpus.

**Unexpected: a third station.** In every run a START_ACK without a report
arrived on band 3 alone, 300 ms before the real peer's copies. 300 ms is
exactly how much shorter a 6-byte START_ACK is than the 11-byte one with a
report on a K 6 band: it was an older page (entry 7's switch, ultrasonic on)
on another device within earshot, answering the band 3 probe. The sender chose
the band on its START_ACK alone, since it came first; here that was harmless
(the peer's own report would have chosen band 3 too), and a wrong choice
would end in the fallback to band 0. Frames have no station address; see
DESIGN.md 5.9.

**Open.** Two different computers (the point of the middle rungs) are
untested. The 12 dB threshold puts the top band about 12 dB above where `v1-us`
alone still decodes in the bench; real data may justify lowering it. The
inline start is gone, so short messages take about 8.6 s instead of 2.85 s.
