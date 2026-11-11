# soundpaste: design

Status: **accepted as the starting point 2026-11-11** (the robust mode is left out of the first version, see section 7). Based on the
measurements of stages 1–2 (see `EXPERIMENTS.md`). The physical-layer parameters
are initial values that the improvement cycle of stage 4 confirms or refutes.

Contents:
1. [Decisions in brief](#1-decisions-in-brief)
2. [The measured channel](#2-the-measured-channel)
3. [Choice of modulation](#3-choice-of-modulation)
4. [Physical layer](#4-physical-layer)
5. [Link layer](#5-link-layer)
6. [Time estimates](#6-time-estimates)
7. [Extensibility](#7-extensibility)
8. [Known limitations](#8-known-limitations)
9. [What has been measured and what has not](#9-what-has-been-measured-and-what-has-not)
10. [Questions and decisions](#10-questions-and-decisions)

## 1. Decisions in brief

| matter | proposal | main justification |
|---|---|---|
| modulation | 16 parallel 4-FSK subchannels, energy detection | withstands echo without phase tracking, 8 times the rate of a single tone |
| echo rejection | tone hopping: consecutive symbols use a different tone group (4 groups) | error rate in the reverberant room 7.8 % -> 0.5 % |
| band | 1000–7400 Hz, 256 tones at 25 Hz spacing | measured response flat over 1–8 kHz, noise 20–27 dB quieter than below 500 Hz |
| symbol | 50 ms (40 ms detection window + 10 ms guard time), 4 bytes/symbol | raw rate 640 bit/s |
| synchronisation | frequency sweep (chirp) as the preamble, matched filter | sharp timing in echo, independent of level and sample rate |
| error correction | Reed–Solomon GF(256), parity half the amount of data, erasures in use | corrects 16 % of bytes as errors or 33 % as erasures |
| acknowledgement | data frames as a burst, one combined ack (bitmap) | for a maximum-length message about 15 s vs. 23 s with per-frame acks |
| own echo | the receiver runs all the time; the own frame is recognised from its content and the timeouts are anchored to it | does not depend on the browser's unknown latency |
| 60-character link | measured in simulation 2.8 s (inline start, default) or 4.2 s (separate handshake) | see section 6 |

## 2. The measured channel

Measurement: MacBook Pro, own speakers -> own microphone, home office,
system volume 44 %, three sweeps (100 Hz – 20 kHz).
Recordings and derived impulse responses: `corpus/channel/` (the corpus is
local-only, not in the repository).

| quantity | measured | consequence for the design |
|---|---|---|
| frequency response | flat ±4 dB over 1–8 kHz; 10–20 kHz about 12 dB weaker but usable | band 1–7.4 kHz; a nearly inaudible mode at 18–20 kHz is possible later |
| background noise in a 50 Hz band | -78 dBFS @ 200 Hz, -98 @ 1 kHz, -105 @ 2–8 kHz | frequencies below 1 kHz are not used; whole-band SNR underestimates a real room |
| SNR of a single tone (full amplitude, 50 Hz) | 62–80 dB at the own microphone | there are tens of decibels of headroom against noise: the power can be split over several tones |
| delay spread | 3–4 ms, coherence bandwidth 50–70 Hz | the band (6.4 kHz) is about 100 times the coherence bandwidth: different tones fade independently of each other |
| energy later than 10 / 20 / 40 ms | -14 / -20 / -27 dB | a short symbol without hopping suffers from the echo of the previous symbol |
| own echo at the noise floor | 62 ms (with 55 dB of dynamic range) | a response delay of 150 ms is enough in this room |
| second harmonic distortion | -40 dB, in one measurement -24 dB | dense constellations (QAM) do not work with small speakers; enough for FSK |
| signal level at the microphone | -33 dBFS at full amplitude | detection must be level-independent |

The crude v0 modem (2-FSK, 100 baud) over real audio: 4/5 transmissions
succeeded; the quietest (amplitude 0.01) went undetected. In simulation v0
breaks down in echo: in the reverberant room nothing gets through.

**Limitation:** the measurement describes the path from the own speaker to the
own microphone (direct sound 11–12 dB stronger than the echo). Another machine
a metre away hears the direct sound an estimated 14 dB weaker and the echo
about the same. This is estimated with the derived profile `:far` and with
synthetic rooms until the PC test station (stage 7) brings a real two-machine
measurement.

## 3. Choice of modulation

Comparison experiment (`soundpaste explore`, result: `bench/experiments/20261111-modulation-comparison.txt`):
with each alternative, random symbols were sent through the channel profiles
and the raw bit error rate was measured with ideal synchronisation, without
error correction. The same total power for all. The figures are bit error
percentages.

| alternative | bit/s | band | clean 0 dB | room 0 dB | reverberant 20 dB | reverberant 5 dB | measured:far 20 dB |
|---|---|---|---|---|---|---|---|
| 2-FSK, 10 ms (like v0) | 100 | 0.2 kHz | 0.00 | 0.00 | 14.3 | 5.3 | 0.00 |
| 16-FSK, single tone, 40+10 ms | 80 | 0.4 kHz | 0.00 | 0.00 | 5.1 | 5.0 | 1.35 |
| the same + hopping (4 groups) | 80 | 1.6 kHz | 0.00 | 0.00 | 0.42 | 0.10 | 0.00 |
| 16 × 4-FSK, no hopping | 640 | 1.6 kHz | 0.00 | 0.22 | 7.8 | 8.4 | 0.50 |
| 16 × 4-FSK, hopping over 2 groups | 640 | 3.2 kHz | 0.00 | 0.03 | 2.7 | 2.7 | 0.09 |
| **16 × 4-FSK, hopping over 4 groups** | **640** | **6.4 kHz** | **0.00** | **0.04** | **0.47** | **1.09** | **0.04** |
| 8 × 4-FSK, 20+5 ms, hopping over 4 groups | 640 | 6.4 kHz | 0.00 | 0.01 | 0.45 | 0.53 | 0.15 |
| 8 × 16-FSK, hopping over 2 groups | 640 | 6.4 kHz | 0.00 | 0.00 | 1.40 | 2.04 | 0.06 |
| DQPSK, 256 tones (OFDM), 40+10 ms | 10,240 | 6.4 kHz | 9.2 | 13.6 | 20.1 | 23.4 | 4.1 |
| DQPSK, 512 tones, 80+20 ms | 10,240 | 6.4 kHz | 8.5 | 12.1 | 14.7 | 18.5 | 1.3 |

Profiles: "room" RT60 0.35 s and direct/echo 8 dB, "reverberant" RT60 0.8 s and
direct/echo 0 dB, "measured:far" the measured response of this room with the
direct sound attenuated by 14 dB. SNR is against white noise over the whole band (0–24 kHz).

### What the experiment tells

**Echo is the enemy, not noise.** All FSK alternatives are error-free on the
clean channel even at 0 dB SNR, but in the reverberant room there are 5–14 %
errors, and making the signal stronger does not help (20 dB and 5 dB give the
same result). The echo of the previous symbol is still ringing when the next
symbol is detected.

**Tone hopping removes most of the effect of echo.** When consecutive symbols
use a different tone group, the echo of the previous symbol does not land on
the candidate tones of the current symbol. The same frequency comes back into
use only after four symbols (200 ms). With four groups the error rate drops
from 7.8 % to 0.5 %; with two groups to 2.7 %. The price is bandwidth, and
according to the measurement it is available.

**Parallel tones give the rate.** One 16-FSK tone carries 80 bits per second,
which would make a 60-character link a transfer of more than 10 seconds.
16 parallel 4-FSK subchannels carry 640 bit/s with the same echo robustness.
The price is power per tone (-24 dB when the peak level is limited), but the
measured tone SNR is 62–80 dB at the own microphone, so there is headroom.
4-FSK is the most efficient choice per unit of band: bits per tone is
log2(M)/M, which is at its largest when M is 2 or 4.

**Phase modulation (PSK/OFDM) does not suit a first version that emphasises
reliability.** DQPSK would carry 16 times more bits, but the error rate is
15–23 % in the reverberant room and 9–14 % already in plain noise. With ideal
error correction one would still get 2–3 kbit/s out of it, but it would
require a low-rate code based on soft decisions (a convolutional or LDPC
code), channel estimation or longer symbols, and it is sensitive to speaker
distortion (measured -24…-40 dB) and to the difference between the devices'
clocks. Single-carrier wideband PSK would in addition need an equaliser,
because the band is 100 times the coherence bandwidth. OFDM is a natural basis
for a later fast mode, not for the first version.

**Chirp (frequency sweep) for carrying data** would spread every symbol over
the whole band and withstand narrowband interference, but the processing gain
it brings against noise is useless when noise is not the limiting factor. For
synchronisation, on the other hand, it is the best alternative (see 4.3).

### The choice

**16 × 4-FSK, tone hopping over four groups, 40 + 10 ms.** An almost equally
good candidate is 8 × 4-FSK with 20 + 5 ms symbols (the same rate and band,
half the tones at once, but a tighter timing requirement). Stage 4 compares
these two with the whole frame, synchronisation and error correction included.

The peak-to-RMS ratio (PAPR) is about 13 dB with 16 tones and about 12 dB with
8 tones. The speaker limits the peak level, so parallel tones cost power.
Clipping experiment: see [section 9](#9-what-has-been-measured-and-what-has-not).

## 4. Physical layer

### 4.1 Tone grid and symbol

- Tones: f_i = 1000 Hz + i · 25 Hz, i = 0…255 (1000–7375 Hz).
- Tone spacing 25 Hz = 1 / 40 ms: the tones are mutually orthogonal in the
  40 ms detection window.
- 16 subchannels, each with 16 tones: 4 values × 4 hop groups.
  The tone of subchannel s, value v (0–3) and symbol t is
  `i = 16·s + 4·v + (t mod 4)` ("interleaved": the candidates are 100 Hz
  apart, and the tones of the previous symbols fall between them).
- A symbol lasts 50 ms: 10 ms of guard time and a 40 ms detection window. The
  guard time absorbs the timing error and the early reflections (measured:
  energy more than 10 ms late is -14 dB). The tones rise and fall smoothly
  within the guard time.
- Every symbol carries 16 × 2 = 32 bits, i.e. 4 bytes.
- Every tone has a fixed pseudorandom initial phase, so that the 16 tones do
  not add up into a single spike.

For every symbol the receiver computes the energies of the candidate tones
(a Goertzel-type correlation over a window of exactly 40 ms) and picks the
strongest in each subchannel. The window length is defined in seconds, so the
same works at any sample rate (48 kHz: 1920 samples, 44.1 kHz: 1764 samples).
FFT is not used for detection, because its length would have to be fitted to a
power of two.

If the difference between the two strongest candidates of a subchannel is
small, the byte is marked as an erasure. Reed–Solomon corrects twice as many
erasures as errors.

### 4.2 Placement of bytes into symbols

Every subchannel carries its own byte stream: a byte takes four consecutive
symbols in one subchannel. Justification: frequency-selective fading is
constant for the duration of a frame (the devices do not move), so a dead
subchannel ruins 1/16 of the bytes and not a piece of every byte. A short
wideband interference (a thump), on the other hand, hits all subchannels at
once. Stage 4 measures this and the alternative placement (byte = four
subchannels in one symbol) with the thump and echo profiles.

### 4.3 Preamble and synchronisation

The preamble is a frequency sweep 1000 -> 7400 Hz (about 60 ms). The receiver
correlates the input signal against the chirp template (a matched filter,
computed block by block with FFT).

- The bandwidth of the chirp (6.4 kHz) gives a timing accuracy of about
  0.2 ms, while the symbol guard time is 10 ms.
- Echo shows up in the correlation as separate later peaks and does not spread
  the main peak. The timing is taken from the first strong peak (the direct
  sound).
- Narrowband interference or a fade ruins only a part of the chirp.
- The detection threshold is relative: the peak is compared with the
  correlation's own background level just before the peak (CFAR). This way the
  threshold depends neither on the signal level nor on out-of-band noise.
- The template is computed at the receiver's own sample rate, so the
  44.1/48 kHz difference has no effect. A clock error of 100 ppm shifts the
  timing by 0.3 ms over a 3-second frame, which fits within the guard time.

The lesson of v0: a 20 ms tail follows the frame, so that the last symbol is
not the last thing the speaker plays.

### 4.4 Frame

```text
| preamble 60 ms | pause 10 ms | header 3 symbols | payload N symbols | tail 20 ms |
```

**Header** (always in the base mode, 12 bytes = 3 symbols):

| byte | content |
|---|---|
| 0 | bits 7–5: protocol version (now 1), bits 4–0: mode identifier (0 = base mode) |
| 1 | payload length L in bytes (0–160) |
| 2 | CRC-8 of bytes 0–1 |
| 3–11 | Reed–Solomon parity (9 bytes: corrects 4 erroneous bytes out of 12) |

The header is coded more heavily than the data, because losing it wastes the
whole frame. The mode identifier tells which modulation the payload was sent
with. No attempt is made to decode an unknown version or mode. Faster modes
can thus use the same preamble and header.

**Payload** (with the modulation of the mode):

| bytes | content |
|---|---|
| 0 … L-1 | link-layer frame (see 5.1) |
| L, L+1 | CRC-16 of bytes 0…L-1 |
| the rest | Reed–Solomon parity, P bytes |

P = ceil((L + 2) / 2), at least 4, rounded up so that L + 2 + P is divisible
by four (the symbol is filled; the extra parity is free).
The codeword is at most 255 bytes, so L is at most 160. CRC-16 is included
because when the capacity is exceeded, Reed–Solomon can on rare occasions
"correct" to a wrong codeword.

Frame duration: 90 ms (preamble, pause, tail) + 150 ms (header) +
50 ms × (L + 2 + P) / 4.

## 5. Link layer

The link layer is a state machine that knows nothing about sound. It is given
events (frame received, preamble detected, own transmission ended, clock) and
it returns actions (send a frame, show a status). Time is supplied from
outside, so the tests are deterministic. The same state machine runs in the
browser, in the CLI and in the simulator.

### 5.1 Frame types byte by byte

In all frames: byte 0 = type (upper nibble) and flags (lower), bytes 1–2 =
message identifier (random 16-bit, chosen by the sender).

| type | name | direction | content from byte 3 onwards | L |
|---|---|---|---|---|
| 1 | START | A -> B | message length (2), number of frames (1), frame data size (1), message CRC-32 (4). Flag 1: the whole message follows at the end of the frame (inline start) | 11–75 |
| 2 | START_ACK | B -> A | signal quality: SNR dB (1), level dBFS (1), flags (1: bit 0 = clipping) | 6 |
| 3 | DATA | A -> B | sequence number (1), at most 64 bytes of data. Flag 0: last of the burst | 4–68 |
| 4 | ACK | B -> A | quality (3), bitmap of received frames (1). Flag 0: start over | 7 |
| 5 | DONE | B -> A | message CRC-32 (4) | 7 |
| 6 | POLL | A -> B | no content: "send your ack again" | 3 |
| 7 | CANCEL | A -> B | no content | 3 |

Flag 3 (0x08) is reserved for marking encrypted content.

The maximum message length is 512 bytes (8 data frames of 64 bytes each). An
empty message is allowed: there are 0 frames and B answers the start directly
with a DONE frame.

### 5.2 Half-duplex operation and own echo

A computer hears its own speaker. The receiver is not muted for the duration
of a transmission; instead it also decodes the own transmission, and the link
layer makes use of it:

- **Recognition:** a received frame that is byte for byte the same as the one
  last sent by oneself is the own echo. The roles guarantee that the other
  side does not send the same: A sends only START/DATA/POLL/CANCEL and B only
  START_ACK/ACK/DONE.
- **Anchoring:** the end of the own frame on the microphone's timeline is the
  exact moment when the sound really stopped. The wait for the reply starts
  from it, so the browser's unknown output and input latency (up to hundreds
  of milliseconds) does not affect the timeout. If the own frame is not heard
  (e.g. headphones), the anchor is the end of the transmission + 0.5 s.
- **Guard time:** the responder waits 150 ms from the end of the received
  frame before sending. The measured own echo is at the noise floor at 62 ms;
  in a more reverberant room the relative threshold of the preamble and tone
  hopping tolerate the rest.

### 5.3 State machine

```text
                 send()                 START_ACK                 all acknowledged
  IDLE ────────────────► STARTING ───────────────► SENDING ◄──┐
   ▲  ▲                  (START)                   (DATA burst) │ ACK: some missing
   │  │                     │ 3 attempts                │        │
   │  │                     ▼ without a reply           ▼        │
   │  │              "no receiver"               WAITING ────────┘
   │  │                                          (ACK/DONE)
   │  └──────────── DONE: "Delivered" ◄─────────────┘
   │
   │  START                       DATA...                    all assembled, CRC-32 ok
  IDLE ────────► RECEIVING ────────────────► RECEIVING ────────────────► IDLE
        START_ACK              ACK at the end                  DONE, show the message
                               of the burst
```

States of the sender (A): IDLE, CHANNEL LISTEN, STARTING, SENDING, WAITING.
States of the receiver (B): IDLE, RECEIVING. The roles are symmetric: either
one can start, and the roles are determined per message.

### 5.4 Timeouts

All times are anchored to the end of a detected signal (the own or the other
side's), not to the own clock at the moment of sending.

| name | value | anchor | purpose |
|---|---|---|---|
| response delay | 150 ms | end of the received frame | guard time for the own and the room's echo |
| reply timeout | 1.5 s | end of the own frame at the microphone | covers the response delay and the audio latencies of both devices |
| extension of the wait | to the end of the frame | preamble detected during the wait | the reply is coming even though it started late |
| start attempts | 3 | | then "no receiver found" (in simulation about 8–11 s) |
| POLL attempts | 3 | | an ack or the final ack was lost |
| retransmission rounds | 6 | | then "transfer failed" with reasons |
| receive timeout | 6 s | the latest detected end of signal | the sender disappeared: the partial message is discarded |
| channel listen | 200 ms of silence | | before the START frame |
| random backoff | 0.2–2.2 s | | channel busy or the start collided |
| memory of completed messages | 8 identifiers, 60 s | | duplicate recognition |

B sends an ack when (a) it receives a DATA frame with the "last of the burst"
flag, or (b) it has detected preambles since its previous ack and the channel
has been silent for 400 ms. Case (b) covers the situation where the last frame
of the burst was corrupted.

### 5.5 Normal session

1. Both computers listen (IDLE). The preamble detector runs all the time.
2. A's user presses Send. The text is UTF-8 bytes. A draws a random
   16-bit identifier, computes the CRC-32 and splits the message into 64-byte chunks.
3. A listens to the channel for 200 ms. If a preamble has been detected or the
   reception of a frame is in progress, A waits a random 0.3–1.2 s and listens again.
4. A sends the START frame and hears it itself (the anchor).
5. B receives the START frame, allocates memory, shows "receiving" and
   after 150 ms sends a START_ACK frame containing the signal quality it
   measured.
6. A receives the START_ACK frame and sends all data frames one after another
   (50 ms between frames). The last one has the "last of the burst" flag.
7. B stores the intact frames and shows the progress. At the end of the burst:
   - all assembled and the CRC-32 matches: B sends a DONE frame, shows the
     text and the Copy button and returns to idle
   - some missing: B sends an ACK frame whose bitmap tells which were received
8. A receives the ACK frame and sends only the missing ones (a new round), or
   receives the DONE frame, compares the CRC-32 and shows "Delivered".

### 5.6 Exceptional situations

| situation | what happens |
|---|---|
| a data frame is lost or corrupted | The frame's CRC-16 (and Reed–Solomon) rejects it. The frame is missing from B's bitmap, and A resends it in the next round. At most 6 rounds. |
| the last frame of the burst is lost | B does not see the flag, but has detected preambles and acknowledges when the channel has been silent for 400 ms. |
| an ack (ACK) is lost or corrupted | A's reply timeout (1.5 s) expires. A sends a POLL frame and B repeats its ack. After three unanswered POLL frames the transfer fails. |
| the start ack (START_ACK) is lost | A sends the START frame again. B recognises the identifier and answers again without starting over. |
| the final ack (DONE) is lost | A sends a POLL frame (or the START/DATA frame again). B finds the identifier in the memory of completed messages, sends the DONE frame again and does not show the message a second time. |
| both press Send at the same time | If the transmissions overlap, neither gets a START_ACK frame. Both wait a random time (0.3–1.2 s) and listen to the channel, so one gets there first. The one that receives a foreign START frame while waiting for its own start becomes the receiver and sends its own message right after the reception ends. If both hear each other's START frame, the smaller identifier keeps the sender role. |
| the sender disappears in the middle of a message | B detects no frames of the session for 6 seconds from the last end of signal. B discards the partial message, tells the user and returns to idle. |
| the user cancels the transmission | A stops the sound immediately, sends one CANCEL frame and returns to idle. B discards the partial message. If the CANCEL is lost, B's 6 s timeout takes care of it. |
| there is no receiver | Three START attempts without a reply, then "no receiver found" in about 6 seconds. No data is sent. |
| a loud interfering sound in the middle of a transfer | Some of the frames are corrupted: Reed–Solomon corrects a short interference (a thump hits 1–2 symbols), a longer one (speech) brings down frames, which are resent. The ack can also be lost (POLL). If the rounds run out, A shows the reason based on the quality reported by B. |
| non-ASCII letters, emoji | The content is UTF-8 bytes that the protocol does not interpret. The split into chunks may fall in the middle of a character, because the message is assembled before interpretation. The length limit is counted in bytes, and the user interface shows it. |
| empty message | START (length 0, 0 frames) -> DONE. |
| maximum-length message | 512 bytes = 8 data frames in one burst. |
| a corrupted frame passes the checks | The frame's CRC-16 and the whole message's CRC-32 together: if the assembled message does not match, B sends an ACK frame with the "start over" flag. |
| a foreign message in the middle of the own session | Frames whose identifier is not that of the ongoing session are ignored. |

The reason for failure shown to the user is derived as follows: no reply at
all -> "no receiver found"; the other side reports clipping -> "signal is
clipping, lower the volume"; reported SNR low and level low -> "signal too
quiet, raise the volume or move the computers closer"; SNR low and level
normal -> "too much background noise".

### 5.7 Burst or per-frame ack

Every change of direction costs the response delay (150 ms), the devices'
audio latencies (estimate 200 ms, measured in loop D) and the ack frame
(390 ms): in total about 0.75 s per ack and another 0.35 s for the return
direction.

| message | per-frame ack | burst + combined ack |
|---|---|---|
| 60 bytes (1 data frame) | the same | the same |
| 512 bytes (8 data frames) | about 23 s | about 15.5 s |

For a one-frame message the methods are identical. For a long message the
burst is a third faster, and selective retransmission with the bitmap handles
a bad channel. The burst size is a link-layer parameter (1 = per-frame ack),
so the bench run of stage 5 compares both on simulated channels before the
final choice.

### 5.8 Inline start (decided on the basis of measurements)

If the message fits in one frame (at most 64 bytes), it travels along with
the first START frame and B answers directly with a DONE frame.
If the start is not answered, the retries are short START frames without the
message (the ordinary handshake), so that a missing receiver is discovered as
quickly as before. If the inline start was delivered but the DONE was lost, B
recognises the identifier of the new start and acknowledges again.

Measured in simulation (15 profiles, 60-character link, median in nominal conditions):

| method | transfer time | success | min SNR, room |
|---|---|---|---|
| separate handshake | 4.24 s | 93.6 % | -6 dB |
| inline start | 2.82 s | 93.9 % | -6 dB |

The missing percent of success come from the `hall` profile, which does not
work with either method. Inline start is the default (`LinkConfig::inline_start`).

## 6. Time estimates

**Measured in simulation** (`soundpaste bench`, `soundpaste session`): 60-character
link 2.8 s with inline start and 4.2 s with the handshake; 512 bytes 15.4–16.1 s
as a burst, 21.5–24.8 s with per-frame acks and 17.1–18.6 s with bursts of
three frames. A missing receiver is discovered in 8–11 seconds.
The calculations below were made before the implementation, and they matched
the measurements to within a few percent.

Frame durations with the proposed parameters (50 ms/symbol, 4 bytes/symbol):

| frame | L | codeword | symbols | duration |
|---|---|---|---|---|
| START | 11 | 20 bytes | 5 | 0.49 s |
| START_ACK | 5 | 12 | 3 | 0.39 s |
| DATA, 60 bytes | 64 | 100 | 25 | 1.49 s |
| DATA, 64 bytes | 68 | 108 | 27 | 1.59 s |
| ACK | 6 | 12 | 3 | 0.39 s |
| DONE | 7 | 16 | 4 | 0.44 s |

**60-character link, everything succeeds on the first try:**

| phase | duration |
|---|---|
| channel listen | 0.20 s |
| START | 0.49 s |
| change of direction (150 ms + audio latencies about 200 ms) | 0.35 s |
| START_ACK | 0.39 s |
| change of direction | 0.35 s |
| DATA | 1.49 s |
| change of direction | 0.35 s |
| DONE | 0.44 s |
| **total** | **about 4.1 s** |

The net rate for the whole session is about 15 bytes/s (v0: 10.8 bytes/s
without acks and without working in echo). The range depending on the audio
latencies is an estimated 3.8–5 s.

**Maximum-length message (512 bytes):** start 1.8 s + 8 data frames
(8 × 1.64 s = 13.1 s) + change of direction and DONE 0.8 s = **about 15.5 s**.
One retransmission round for one frame adds about 2.7 s.

## 7. Extensibility

These are not implemented now, but the structure does not rule them out:

- **Faster mode:** the mode identifier of the header (5 bits) selects the
  payload modulation. START_ACK tells the receiver's signal quality, and A can
  choose a faster mode for the data frames. The header and the acks stay in
  the base mode.
- **Nearly inaudible mode (18–20 kHz):** the measured response there is about
  12 dB weaker but usable. The mode needs its own preamble (a chirp on a
  different band), i.e. a second detector is added alongside in the receiver.
  `PhyKind` and the tone-grid parameters (lowest frequency, tone spacing,
  number of subchannels) are made for this.
- **Robust mode for heavy noise:** the first version has no slower fallback
  mode (decision 2026-11-11: we try without and add it if practice requires).
  Adding it is possible thanks to three structural choices:
  1. All parameters of the waveform (number of tones and subchannels, symbol
     length, share of parity, preamble) are in one structure (`MfskParams`),
     and every parameter set is its own `PhyKind` value. The robust mode is a
     new row in the table, for example 8 subchannels, 80 ms symbols and as
     much parity as data.
  2. Because the start and ack frames must hold up at least as well as the
     data, the robust mode needs its own preamble (e.g. a falling chirp), from
     which the receiver also recognises the modulation of the header. The
     receiver can run several detectors in parallel; the same mechanism serves
     the nearly inaudible mode.
  3. The link layer gets the mode along with every frame and gives the mode
     for every frame to be sent, so it can switch modes on the basis of the
     quality report or failed rounds.
- **Rate adaptation:** the quality bytes of the START_ACK and ACK frames are
  already in the protocol.
- **Encryption:** frame flag 3 is reserved. Encryption would apply to the
  content of the DATA frames and to the CRC-32 of the START frame, not to the
  frame structure.
- **Versioning:** the three version bits of the header. An unknown version is
  silently ignored.

## 8. Known limitations

- **Only two devices.** If there are several receivers within earshot, all of
  them answer the START frame at the same time and the acks collide. The
  sender sees this as a missing receiver or as a failed transfer. A solution
  would require a receiver identifier and a randomised response delay, and it
  is not done in the first version.
- **No encryption and no authentication.** Anyone within earshot can receive
  the message or send their own. The user interface says so, and received text
  is always shown as plain text.
- **An exactly simultaneous send is not always resolved.** If both press Send
  within less than about 0.2 seconds of each other, the starts collide and
  each retries after a random time (0.2–2.2 s). If the drawn times again fall
  within less than 0.2 s of each other (the preamble detection delay), the
  collision repeats. In the frame-level test more than 93 % of cases are
  resolved within three attempts; the rest end on both sides with the notice
  "no receiver found". If the interval between the presses is more than 0.3 s,
  the situation is always resolved.
- **Very reverberant space.** When the reverberation time is more than about a
  second and the echo is stronger than the direct sound (the `hall` profile),
  the payload errors exceed the capacity of the error correction and
  retransmission does not help.
- **Identifier collision.** The 16-bit identifier can come out the same as a
  recently received one (1/65536 per message for each of the 8 slots of the
  memory). B would then take the new message for a duplicate. The memory
  therefore also compares the CRC-32 and the length.
- **Moving devices.** The 25 Hz tone spacing tolerates only a little Doppler
  shift (walking speed at a frequency of 7 kHz is about 20 Hz). The devices
  are assumed to be stationary.
- **Audibility.** The signal is an audible, wideband chirping sound.

## 9. What has been measured and what has not

| claim | basis |
|---|---|
| choice of band, noise spectrum, distortion, duration of the own echo | measured over real audio on the Mac (one device, one room) |
| benefit of tone hopping, 4-FSK vs. 16-FSK, weakness of PSK in echo | simulation: synthetic rooms and the measured impulse response |
| the channel heard by the other machine | **estimate** (direct sound -14 dB), not a measurement |
| audio latency of the devices (200 ms), duration of a change of direction | **assumption**, measured in loops D and E |
| frame durations and the total time of 4.1 s | calculated from the parameters, not yet implemented |
| preamble detection and timing | not yet measured (the comparison experiment used ideal timing) |
| Reed–Solomon code rate, placement of bytes | initial value, stage 4 measures |

**Peak clipping.** The comparison experiment was repeated so that the peaks of
the transmit signal were clipped 6 dB above the RMS value (the limited peak
level of a speaker), whereby the PAPR drops from 13 dB to 6 dB
(`bench/experiments/20261111-modulation-comparison-clipped.txt`). Clipping did
not measurably weaken any FSK alternative: the error rate of the proposed
16 × 4-FSK in the reverberant room was 0.49 % (without clipping 0.47 %).
The peaks of the transmit signal can therefore be limited and the average
power raised by about 7 dB, so the power cost of parallel tones is in practice
about 5 dB and not 12 dB. This is a simulation result with an ideal clipper;
the behaviour of a real speaker is measured in loop C.

## 10. Questions and decisions

Status 2026-11-11: questions 1–4 received no separate answer, so the
implementation follows the proposals: (1) inline start is a parameter and, on
the basis of the measurements, on by default (section 5.8), (2) band
1–7.4 kHz, (3) maximum length 512 bytes, (4) in a simultaneous send the
message that yielded is sent automatically. The robust mode for heavy noise
was left out (section 7). All four are one-line changes (`LinkConfig`, `MfskParams`).

The original questions:

1. **A short message in the start frame?** The task specification proposes a
   separate handshake (START -> START_ACK -> DATA -> DONE), and the design
   follows it. If the message fits in one frame, it could be sent along with
   the START frame, in which case a 60-character link takes about 2.6 s
   instead of 4.1 s. The price: when there is no receiver, the failure is
   discovered more slowly (about 9 s instead of 6 s), and no quality report is
   obtained before the data. I propose making this a link-layer parameter and
   choosing the default on the basis of the stage 5 measurements. Is that OK?
2. **Band 1–7.4 kHz.** The signal sounds like a chirping of about 0.5–3
   seconds. A narrower band (e.g. 1.5–4.7 kHz) would halve the rate. Does the
   character of the sound matter at this stage?
3. **Maximum length 512 bytes** (about 15 s). Is that enough, or do you want
   more (1024 bytes would take about 30 s)?
4. **Is some exceptional situation handled differently from what you would
   like?** In particular the simultaneous send: the loser's message is sent
   automatically after the reception without a new button press.
