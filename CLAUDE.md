# soundpaste

An audio modem for the browser: two computers next to each other transfer short
text by sound. A Rust core (no dependencies) is compiled to a native CLI and to
WebAssembly. Task specification and work order: `TASK.md`. Design decisions:
`DESIGN.md`. Experiments: `EXPERIMENTS.md`.

Docs, UI, code comments and identifiers are in English.

## Layout

- `crates/modem-core`: the core, zero dependencies. `dsp` (FFT, sliding
  frequency detector, convolution, sample-rate conversion), `phy`
  (waveforms, `PhyKind`), `sim` (channel simulator), `crc`, `wav`, `rng`.
- `crates/cli`: the `soundpaste` command. No dependencies (own JSON and argument parsing).
- `bench/results/`: bench runs as JSON, `bench/baseline.json`: the baseline.
- `corpus/`: real recordings (channel measurements and modem transmissions). Local only,
  not in version control (the user's decision: room recordings are not published).
- The page is published on GitHub Pages with `publish-github.sh`
  (https://heikkisairanen.github.io/soundpaste/). GitHub Pages is the only publishing
  target for now.

## Development-cycle commands

```sh
cargo test -q                        # unit tests (loop A)
cargo build --release -q
./target/release/soundpaste bench    # bench run + scoreboard, saves to bench/results/
./target/release/soundpaste bench --quick --no-save   # quick check (4 repetitions)
./target/release/soundpaste compare  # latest run vs. baseline, exit code 1 on regressions
./target/release/soundpaste baseline # make the latest run the baseline (only when the change stays)
./target/release/soundpaste sim --profile reverberant --snr 10 --seed 3 -o /tmp/rx.wav   # one transfer
./target/release/soundpaste encode -o x.wav "text" && ./target/release/soundpaste decode x.wav
./target/release/soundpaste decode x.wav --expect "text"   # shows where the bit errors are

# Real audio (ask permission first!). --sim <profile> runs the same silently in the simulator.
./target/release/soundpaste audio-test          # microphone level, plays nothing
./target/release/soundpaste measure --gain 0.3 --note "..."   # channel measurement -> corpus/channel
./target/release/soundpaste acoustic --gains 1.0,0.3,0.1      # modem transmission -> corpus/tx
./target/release/soundpaste corpus [--holdout]  # corpus listing and regression run (loop B)
./target/release/soundpaste bench --holdout     # include the holdout set (only at the end of a stage)
./target/release/soundpaste explore             # comparison of modulation alternatives (about 3 min)

# Browser and two computers (README.md, loops D and E)
./build.sh                                      # CLI + wasm + dist/
./target/release/soundpaste serve               # dev server, http://localhost:8787/
./target/release/soundpaste live --send "x"     # a station on real audio (ask permission for sound)
./target/release/soundpaste remote mode         # test-station status; remote send / mode echo / collect N
```

Do not pipe `measure`, `acoustic` or `bench` into `head` (redirect to a file): a broken pipe
crashes the program before it saves.

## Improvement cycle

1. Write the hypothesis in `EXPERIMENTS.md` before the change.
2. Make the change, run `cargo test` and `soundpaste bench`, then `soundpaste compare`.
3. Record the result and the conclusion, failed experiments included.
4. If the change stays: `soundpaste baseline`, commit (include the result file and the baseline).

Do not tune for a single room or device pair, and do not adjust tests to make them pass.

## Agreed rules

- Ask the user before playing sound for the first time in each work session.
  Keep the volume moderate.
- Ask before installing anything or adding a dependency (dev tooling included).
- Publishing anywhere other than GitHub Pages is agreed separately. Ask first.
- No wasm-bindgen, no npm, no bundler.

## Current state

- Stages 1–8 are implemented. The page is published on GitHub Pages. Loop D (CLI + browser on the same Mac) has been run on real audio; loop E
  has been run once with the PC on the published page (no PC test station yet).
- Default waveform `v1` (16 × 4-FSK, tone hopping, chirp preamble,
  Reed–Solomon), link layer (`link/`), station (`station.rs`) and two-way
  simulation (`sim/duplex.rs`).
- Baseline `bench/baseline.json`: session mode, 60-character link 2.85 s, success
  94.2 % in nominal conditions (the missing part is the `hall` profile, which does not
  work), lowest SNR in a room -6 dB.
- `soundpaste bench` runs the full session by default (about 35 s). `--oneway` measures
  the physical layer alone (about 18 s), `--quick` is a quick check.
- Decisions made and their reasons: `DESIGN.md` (chapters 5.8 and 10) and `EXPERIMENTS.md`.

## Known limits (honestly)

- `hall` (RT60 1.2 s, reverberation louder than the direct sound) does not work. The fix
  would be the robust mode of DESIGN.md chapter 7; the user decided to leave it out until
  practice demands it.
- One session has been run between two computers (PC browser -> Mac works, EXPERIMENTS.md entry 5).
  Mac -> PC needed a loud volume. There are no logs and no channel measurement from the PC end.
- Exactly simultaneous sends (within 0.2 s) resolve in about 93 % of cases.

## Worth trying next

- Measuring a real two-computer channel (PC test station) and adding it as profiles.
- Parity 1/3 (`v1-p3`): 10 % faster, 3 dB more sensitive to noise. Decide on real data.
- Shortening the preamble detection delay (about 150 ms): it would shrink the collision window.
- The interference profiles (`thump`, `speech`) are too easy in simulation: replace them with real recordings.
