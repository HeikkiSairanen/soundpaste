# soundpaste

Send a short text (a link, a note, a code snippet) to the computer next to
you by sound. One computer plays through its speaker, the other listens with
its microphone and acknowledges by sound. There is no server: everything
happens in the browser.

- Page: https://heikkisairanen.github.io/soundpaste/ (publishing: see [Publishing](#publishing))
- Design and rationale: [DESIGN.md](DESIGN.md)
- Experiment log with measurements: [EXPERIMENTS.md](EXPERIMENTS.md)
- Original task specification: [TASK.md](TASK.md)

A 60-character link transfers, acknowledgement included, in about 2.5 seconds
and the 512-byte maximum message in about 16 seconds. The transfer is not
encrypted.

## Layout

| directory | contents |
|---|---|
| `crates/modem-core` | The core, no dependencies: signal processing (`dsp`), waveforms (`phy`), Reed–Solomon (`rs`), link layer (`link`), station (`station`), channel simulator (`sim`), channel measurement (`measure`) |
| `crates/cli` | The `soundpaste` command: encoding, bench runs, real-audio loops, dev server. No dependencies |
| `crates/wasm` | WebAssembly wrapper: a C interface without wasm-bindgen |
| `web/` | The published page: `index.html`, `style.css`, `app.js` (main thread), `worklet.js` (audio thread) |
| `dev/` | Browser code for the test station. Only the dev server serves it |
| `bench/` | Bench run results (`results/`), the baseline (`baseline.json`), one-off experiments (`experiments/`) |
| `corpus/` | Real recordings: channel measurements (`channel/`) and modem transmissions (`tx/`). Not in version control: you create it with your own measurements |

The same state machine (`Station`: modem + link layer) runs in the browser, on
the command line and in the simulator.

## Building

You need Rust and the wasm target (`rustup target add wasm32-unknown-unknown`).
No npm, no bundler, no external crates.

```sh
./build.sh           # builds the CLI and the wasm module, assembles the page into dist/
./build.sh --test    # also runs the tests
cargo test -q        # tests only
```

`dist/` is a finished static page that works in any subdirectory (relative
paths). In the browser the microphone requires https or localhost.

## Development cycle

The principle: every change is measured with one command, and most of the work
happens silently on the Mac. The loops, fastest to slowest:

### A. Simulation (seconds, silent)

```sh
cargo test -q
./target/release/soundpaste bench            # full session over 15 channel profiles, about 35 s
./target/release/soundpaste bench --quick    # quick check
./target/release/soundpaste bench --oneway   # physical layer only
./target/release/soundpaste compare          # latest run vs. baseline; exit code 1 if anything regressed
./target/release/soundpaste baseline         # make the latest run the baseline (only when the change stays)
./target/release/soundpaste session --profile reverberant --snr 3 --len 300   # one session with its log
./target/release/soundpaste sim --profile hall --snr 20 --seed 2              # one frame
```

The scoreboard always has the same metrics: transfer time of a 60-character
link, success rate, lowest working signal-to-noise ratio, bit error rate before
error correction, number of corrected bytes and net data rate. Results are
saved in `bench/results/` together with the commit id.

### B. Recording corpus (seconds, silent)

```sh
./target/release/soundpaste corpus             # lists the corpus and decodes the recordings with the current receiver
./target/release/soundpaste corpus --holdout   # include the holdout set
```

Measured channels (`corpus/channel/`) automatically become bench profiles
(`m:...`), and the transmission recordings (`corpus/tx/`) are run with every
bench run. Part of the material is marked as the holdout set
(`"holdout": true`), which is not used for tuning.

The corpus consists of room recordings, so it is not distributed with the
source. Without it the bench run works on the synthetic profiles alone, and the
corpus rows and `m:` profiles in the results under `bench/` are absent. You get
a corpus of your own with the commands in section C.

### C. Acoustic loop on the Mac (tens of seconds, audible)

Requires SoX (`rec`, `play`) and microphone permission for the terminal.

```sh
./target/release/soundpaste audio-test                      # microphone level, plays nothing
./target/release/soundpaste measure --note "study"           # channel measurement with a sweep -> corpus/channel
./target/release/soundpaste acoustic --gains 1.0,0.3,0.1     # modem transmission at several levels -> corpus/tx
```

Add `--sim room` and the same command runs silently through the simulator.

### D. Browser on the same Mac (a minute)

```sh
./target/release/soundpaste serve &                          # dev server, http://localhost:8787/
# open http://localhost:8787/?station=mac-browser in the browser and allow the microphone
./target/release/soundpaste live --send "text"               # the CLI sends, the browser receives
./target/release/soundpaste remote send "text"               # the browser sends...
./target/release/soundpaste live --once --expect "text"      # ...and the CLI receives
```

`live` is the same station as in the browser, but the audio goes through SoX.

### E. Two computers (minutes)

The PC is a passive test station: once set up it needs no touching.

**One-time setup on the PC (Windows, Chrome or Edge):**

1. On the Mac, start `./target/release/soundpaste serve`. It prints the local
   network address in the form `http://<mac-name>.local:8787/`. Allow
   connections if the macOS firewall asks.
2. Create a shortcut on the PC desktop whose target is (on one line, with the
   Mac's address filled in):

   ```text
   "C:\Program Files\Google\Chrome\Application\chrome.exe"
     --user-data-dir=C:\soundpaste-test
     --unsafely-treat-insecure-origin-as-secure=http://<mac-name>.local:8787
     --autoplay-policy=no-user-gesture-required
     --use-fake-ui-for-media-stream
     "http://<mac-name>.local:8787/?test=receive&station=pc"
   ```

   The flags: a separate browser profile (does not affect other browsing), the
   Mac's http address is treated as secure (the microphone requires that),
   audio is allowed without a click and the microphone permission is accepted
   automatically. For Edge the path is
   `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`.
   If the `.local` name does not work, use the Mac's IP address and reserve a
   fixed address for it in the router.
3. In the Windows sound settings turn microphone effects off (noise
   suppression, "Voice Clarity", vendor effects); no sleep and no screen lock;
   keep the browser window visible and the computers 0.5–1 m apart.

**Use from the Mac:**

```sh
./target/release/soundpaste remote mode                      # is the station alive
./target/release/soundpaste remote mode echo                 # test mode: receive | echo | beacon [interval s]
./target/release/soundpaste live --send "text"               # the Mac sends, the PC receives (in echo mode it sends it back)
./target/release/soundpaste remote send "text"               # the PC sends
./target/release/soundpaste remote collect 5                 # the next 5 session recordings from the PC go into the corpus
./target/release/soundpaste measure --from-remote pc         # channel from the PC's speaker to the Mac's microphone
./target/release/soundpaste measure --to-remote pc           # channel from the Mac's speaker to the PC's microphone
```

The test station checks every two seconds for a new version: a new wasm module
is swapped in live (the microphone stays on) and a changed page is reloaded.
Logs are printed in the server's terminal and saved under `devdata/`, as is
the microphone recording of every session.

None of this ends up in the published page: `build.sh` assembles `dist/` from
the `web/` directory only and aborts if the files contain a network address or
a trace of the dev tooling. The page's CSP allows connections to its own
directory only.

### Improvement cycle

1. Write the hypothesis in `EXPERIMENTS.md`: what changes and which metric is expected to improve.
2. Make the change, run loops A and B (`cargo test`, `bench`, `compare`).
3. If the result looks promising, run loop C and later D and E.
4. Record the result and the conclusion, failed experiments included.
5. If the change stays: `baseline` and commit.

A new waveform is a new row in the `MFSK_PRESETS` table
(`crates/modem-core/src/phy/mod.rs`), and it can be measured with `--phy <name>`.

## Publishing

```sh
./publish-github.sh   # builds the page and pushes dist/ to GitHub Pages (branch gh-pages)
```

`dist/` is an ordinary static directory, so it can be copied to any HTTPS
server. The server must serve `.wasm` with the MIME type `application/wasm`.

## Known limitations

- Only two devices at a time: several receivers would acknowledge on top of each other.
- No encryption and no authentication: any device within earshot can receive the
  message. Do not send passwords or other secrets with it.
- A very reverberant space (reverberation time over about a second with the
  computers far apart) does not work; a more robust mode is designed but not
  implemented.
- More detail: DESIGN.md chapter 8 and CLAUDE.md.

## License

MIT, see `LICENSE`.
