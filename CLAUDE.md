# soundpaste

Äänimodeemi selaimeen: kaksi vierekkäistä konetta siirtää lyhyttä tekstiä
äänellä. Rust-ydin (ei riippuvuuksia) käännetään natiiviksi CLI:ksi ja
WebAssemblyksi. Tehtävänanto ja työjärjestys: `TEHTAVA.md`. Suunnitteluvalinnat:
`DESIGN.md` (kirjoitetaan vaiheessa 3). Kokeilut: `EXPERIMENTS.md`.

Dokumentit, käyttöliittymä ja koodin kommentit ovat suomeksi, tunnisteet englanniksi.

## Rakenne

- `crates/modem-core`: ydin, nolla riippuvuutta. `dsp` (FFT, liukuva
  taajuusilmaisin, konvoluutio, näytteenottotaajuuden muunnos), `phy`
  (aaltomuodot, `PhyKind`), `sim` (kanavasimulaattori), `crc`, `wav`, `rng`.
- `crates/cli`: `soundpaste`-komento. Ei riippuvuuksia (oma JSON ja argumentit).
- `bench/results/`: mittausajot JSON-muodossa, `bench/baseline.json`: perustaso.
- `corpus/`: oikeat äänitteet (kanavamittaukset ja modeemilähetykset).

## Kehityssyklin komennot

```sh
cargo test -q                        # yksikkötestit (kehä A)
cargo build --release -q
./target/release/soundpaste bench    # mittausajo + pistetaulu, tallentaa bench/results/
./target/release/soundpaste bench --quick --no-save   # nopea tarkistus (4 toistoa)
./target/release/soundpaste compare  # uusin ajo vs. perustaso, paluuarvo 1 jos heikennyksiä
./target/release/soundpaste baseline # uusin ajo perustasoksi (vain kun muutos jää voimaan)
./target/release/soundpaste sim --profile kaikuisa --snr 10 --seed 3 -o /tmp/rx.wav   # yksi siirto
./target/release/soundpaste encode -o x.wav "teksti" && ./target/release/soundpaste decode x.wav
```

## Parannussykli

1. Kirjaa hypoteesi `EXPERIMENTS.md`:hen ennen muutosta.
2. Tee muutos, aja `cargo test` ja `soundpaste bench`, sitten `soundpaste compare`.
3. Kirjaa tulos ja johtopäätös, myös epäonnistuneet kokeilut.
4. Jos muutos jää voimaan: `soundpaste baseline`, commit (mukaan tulostiedosto ja baseline).

Älä viritä yhden huoneen tai laiteparin mukaan äläkä säädä testejä menemään läpi.

## Sovitut pelisäännöt

- Kysy käyttäjältä ennen kuin soitat ääntä ensimmäisen kerran kussakin
  työrupeamassa. Äänenvoimakkuus maltillisena.
- Kysy ennen kuin asennat mitään tai lisäät riippuvuuden (myös kehitystyökaluihin).
- Julkaisusta (sairanen.org/soundpaste) sovitaan erikseen. Kysy ensin.
- Ei wasm-bindgeniä, npm:ää eikä bundleria.

## Nykytila

Vaihe 1 valmis (karkea v0 + mittaristo). Perustaso: ks. `EXPERIMENTS.md` kohta 0.
