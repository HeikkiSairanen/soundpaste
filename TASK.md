# Task: soundpaste, an acoustic modem for the browser (Rust + WebAssembly)

This is an English translation of the original Finnish task specification.

The original task specification as given (2026-11-11). The page is published at
https://sairanen.org/soundpaste.

## Goal
Build a static web page with which two adjacent computers transfer short
text (a link, a password, a code snippet) to each other using sound: one
computer plays through its speaker, the other listens with its microphone.
The connection is bidirectional: the receiver acknowledges back by sound
that the message arrived intact, and the sender sees the confirmation.
There is no server; everything happens in the browser. The page is
published at sairanen.org/soundpaste.

Build this like a project that can grow into something bigger: a clear
architecture, good test coverage and code that is easy to build on. The
transfer problem itself (modulation, synchronisation, error correction,
acknowledgements) is the core of the work and its most interesting part. I
want to understand the solutions, so justify the design choices and do not
hide them behind libraries.

The way of working is iterative: first build the instrumentation and a
crude version that works end to end, then improve the modem measurably. I
work mostly on a Mac, so the development cycle has to run mainly on it.

## Constraints
- The core in Rust, compiled to the target wasm32-unknown-unknown.
- As few external dependencies as possible. The goal is zero crates in the
  core: implement FFT or Goertzel, CRC, Reed-Solomon (GF(256)) and WAV
  reading and writing yourself. In development tools a dependency is easier
  to accept, but still ask first and justify it.
- No wasm-bindgen, no npm, no bundler. Export the functions directly
  (`#[no_mangle] extern "C"`) and write a small hand-made JS glue that
  handles only the browser APIs (getUserMedia, AudioContext, AudioWorklet,
  clipboard).
- The end result is a folder of static files (index.html, .js, .wasm).
  Use relative paths so that the page works in a subdirectory.
- The first version is limited to two devices. The case of several
  receivers (overlapping acks) may remain unsolved, but record it in
  DESIGN.md as a known limitation.

## Architecture
1. `modem-core` (a Rust library, no browser dependencies), in two layers:
   - physical layer: bytes -> f32 samples and f32 samples -> bytes.
     The receiver works in a streaming fashion, i.e. it is fed blocks of
     samples as they arrive.
   - link layer: frames, acks and retransmissions as a state machine that
     knows nothing about sound and whose time is supplied from outside, so
     that it can be tested deterministically.
2. A native CLI from the same core: wav encoding and decoding, running the
   whole link layer over real audio, and the measurement commands of the
   development cycle.
3. The web part: a wasm wrapper and a minimal user interface. The same
   state machine runs in the browser and in the CLI.
4. A development server for the Mac (for development use only, not part of
   the published page): serves the page to the local network and receives
   the test station's logs and recordings.

## Normal session
This is an initial proposal for how the data flows. You may propose
something better in DESIGN.md, but there must be an answer to every
situation below.

1. Both computers open the page and press Start. Each is idle and
   listening. The roles are symmetric: either one can send.
2. A enters the text and presses Send. The text is encoded as UTF-8 bytes,
   a random identifier and a checksum of the whole message are created for
   the message, and it is split into numbered frames.
3. Before sending, A listens that the channel is free. If it is not, it
   waits a random time and tries again.
4. A sends a short start frame: the message identifier, the length and the
   number of frames. B answers with a short ack that contains the signal
   quality at the receiving end. If no answer arrives after a few attempts,
   A stops immediately and reports that no receiver was found.
5. A sends the data frames. B shows the progress.
6. B acknowledges which frames arrived intact. A resends only the missing
   ones. This repeats for a limited number of rounds.
7. When all frames have been delivered, B assembles the message, verifies
   the checksum of the whole message and sends a final ack that includes
   the checksum.
8. A shows "Delivered". B shows the text and a Copy button. Both return to
   idle.

To be considered in the design:
- Every change of direction costs a preamble and an echo guard time.
  Therefore compare per-frame acks with sending as a burst with one
  combined ack, and choose on the basis of measurements.
- The connection is half-duplex. A computer hears its own speaker, so the
  receiver must not interpret its own transmission or its echo as an
  incoming message. Dimension the guard time according to the measured
  echo.
- The browser's audio input and output latency varies and can be hundreds
  of milliseconds. Anchor the timeouts to the end of the detected signal
  and dimension them generously.
- Ack frames must be short and at least as robust as data frames.
- The frame header has a protocol version, a frame type and a mode
  identifier, so that faster modes and encryption can be added later
  without breaking compatibility.

Exceptional situations that must be handled and tested:
- a data frame is lost or corrupted
- an ack is lost or corrupted
- the final ack is lost: A resends, and B must not show the same message
  twice but recognise the identifier and acknowledge again
- both press Send at the same time
- the sender disappears in the middle of a message: B gives up the partial
  message on a timeout and returns to idle
- the user cancels the transmission midway
- there is no receiver at all
- there is a loud interfering sound in the room in the middle of a
  transfer (speech, a thump)
- content: non-ASCII letters (ä, ö), emoji, an empty message and a
  maximum-length message

## Physical layer
The final first version emphasises reliability, not speed:
- audible frequency range, MFSK or a similar simple and robust modulation
- a preamble for synchronisation, a frame with a length and a CRC
- Reed-Solomon error correction
- maximum message length a few hundred bytes

A later stage (do not implement yet, but do not rule it out in the
structure): a faster mode, a nearly inaudible mode at around 18–20 kHz,
rate adaptation based on the signal quality reported by the receiver, and
content encryption.

## Development cycle
The principle: every change is measurable with one command, and most of the
iteration happens silently and on the Mac alone.

### Metrics
Every bench run produces the same scoreboard:
- main metric: how long the transfer of a link of about 60 characters takes
  including acks, and what fraction of transfers succeeds
- robustness: the lowest signal-to-noise ratio at which the success rate
  stays high
- margin: the bit error rate before error correction and the number of
  corrected symbols, to see how close to the limit we are
- net rate in bytes per second

The results are stored in machine-readable form (e.g. JSON) in the
directory `bench/results/` together with the git commit identifier, and a
separate command compares a run with the previous baseline and highlights
regressions.

### Loops from fastest to slowest
A. Simulation (seconds, silent). `cargo test` and a bench run through a
   simulated channel with fixed seeds. The channel models: background
   noise at different signal-to-noise ratios, attenuation and clipping,
   echo, a sample rate difference (44.1 kHz vs 48 kHz plus a small clock
   error), a random initial delay and a varying delay between the
   directions. A bidirectional session has two modem instances, a separate
   channel for each direction and in addition the leak of one's own
   transmission into one's own microphone. Each exceptional situation of
   the Normal session section has its own test.

B. Recording corpus (seconds, silent). A directory of real recordings with
   metadata (device, volume, distance, protocol version, expected
   content). The corpus has two kinds of material:
   - channel measurements: sweeps, noise and plain room background sound.
     From these the room impulse response and the real background noise
     are derived and fed into the loop A simulation. This way a new
     modulation can be tried against a real room without playing sound.
     These do not go stale when the modulation changes.
   - recordings of modem transmissions: regression tests for the receiver
     for as long as the waveform stays the same. Mark the protocol version
     so that stale ones are recognised.
   Keep part of the corpus and of the measured channels aside as a holdout
   set that is not used for tuning.

C. Acoustic loop on the Mac (tens of seconds, audible). One command plays
   a transmission through the Mac's speaker, records it simultaneously
   with the microphone, decodes it and adds the recording to the corpus.
   Repeat at several volumes.
   - Use the system's ready-made command-line tools for playback and
     recording, and first find out what the Mac has. Ask before you
     install anything. Using the microphone may require a one-time
     permission for the terminal from me.
   - Ask me before you play sound for the first time in each work session,
     and keep the volume moderate.

D. Browser on the same Mac (a minute). The Mac's CLI and the page running
   in the Mac's browser talk to each other. This tests the wasm wrapper
   and the JS glue without a second computer.

E. Two computers (minutes). A PC is placed next to the Mac as a passive
   test station that does not need to be touched after setup:
   - The Mac's development server serves the page to the local network.
   - The PC's page is in test mode (a URL parameter) and checks regularly
     whether a new version is available. A new wasm module is swapped in
     on the fly without reloading the page, so that the microphone and
     audio stay on and no new button press is needed.
   - The PC's page sends the session logs and its microphone recordings to
     the Mac's development server, and they are added to the corpus.
   - Test modes: receive mode (the normal protocol), echo mode (sends the
     message it received back as a new message) and beacon mode (sends a
     known test message at regular intervals).
   - The microphone requires a secure context in the browser, and a
     local-network http address is not one. Find out the options and
     propose to me the simplest one-time setup for the PC.
   - None of this may end up in the published version: the production page
     does not contact anything.
   Publication to sairanen.org/soundpaste is agreed separately, so ask me
   about it.

### Improvement cycle
1. Record the hypothesis: what is changed and which metric is expected to
   improve.
2. Make the change and run loops A and B.
3. If the result is promising, run loop C and later D and E.
4. Compare with the baseline. Record the result and the conclusion, also
   when the experiment failed.
5. If the change stays, update the baseline and commit.

Do not tune parameters for only one room and one device pair. Report
honestly where the limit lies instead of adjusting the tests to pass.

### Continuity between work sessions
- `EXPERIMENTS.md`: the experiment log, with the hypothesis, the change,
  the measurement result and the conclusion of every experiment.
- `CLAUDE.md`: the commands of the development cycle, the current baseline
  and the next ideas worth trying, so that a new session can continue
  right away.

## DESIGN.md
Write `DESIGN.md` only once the crude first version and the channel
measurements exist (see Work order), so that the choices are based on
measured data. In it you
- briefly compare modulation alternatives (e.g. MFSK, PSK, OFDM,
  chirp-based synchronisation) and justify the choice with measurements
- describe the frame structure byte by byte
- describe the link-layer state machine and the timeouts, and go through
  the normal session and every exceptional situation
- estimate how long the transfer of a typical link and of a maximum-length
  message takes including acks

Show it to me and wait for my comments before you continue to the actual
modem.

## Browser-side pitfalls
- Request the microphone with echoCancellation, noiseSuppression and
  autoGainControl turned off. They are made for speech and break the modem
  signal.
- Do not assume the sample rate; read it from the AudioContext.
- Audio playback and the microphone require a user button press and HTTPS
  (or localhost).
- Microphone reading and signal processing in an AudioWorklet. Find out
  how a tab left in the background affects things, and warn the user if
  necessary.
- Writing to the clipboard requires a user action, so copying happens from
  a button.
- Received text is untrusted input, because anyone within earshot can
  send. Always show it as plain text (textContent), never as HTML, and do
  not open links automatically.

## User interface
In Finnish and pared down.
- The Start button enables the microphone and audio on both computers.
- A text field and a Send button. Before sending, show an estimate of the
  transfer duration and block a too-long message with a clear notice.
- For the sender, the transfer status: searching for the receiver, sending
  (frame n/m), waiting for the ack, retrying, delivered or failed. A
  Cancel button for the duration of the transmission.
- For the receiver, the progress, the finished text and a Copy button.
- On failure, state the reason and practical advice: no receiver found,
  signal too quiet, signal clipping or too much background noise.
- A continuous signal level display that shows whether the computer hears
  anything at all.
- A notice that the transfer is not encrypted and that nearby devices can
  receive it.
- A troubleshooting view: the session event log with timestamps and the
  option to download the microphone recording of the latest session as a
  WAV file, so that a failed transfer can be added to the corpus and
  replayed with the CLI.
- No analytics, no external fonts and no CDN downloads.

## Work order
1. A crude end-to-end version and the instrumentation: the simplest
   possible one-way modulation without error correction, the CLI, a
   simulated channel, the scoreboard and result storage. The goal is to
   get loop A running, not a good modem.
2. The acoustic loop on the Mac: playback and recording, channel
   measurement, the start of the corpus and feeding the measured channel
   into the simulation (loops B and C).
3. DESIGN.md based on the measurements, then a stop for my comments.
4. The actual physical layer with the improvement cycle.
5. The link layer: session, acks and exceptional situations with tests.
6. The wasm wrapper, the web page and the test modes (loop D).
7. The development server and the PC test station (loop E).
8. README: build instructions, use of the development cycle and
   publication.

At the end of each stage, report the scoreboard and what has been tested
only in simulation, what with real audio on the Mac and what between two
computers.
