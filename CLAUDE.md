# soundpaste

Äänimodeemi selaimeen: kaksi vierekkäistä konetta siirtää lyhyttä tekstiä
äänellä. Rust-ydin (ei riippuvuuksia) käännetään natiiviksi CLI:ksi ja
WebAssemblyksi. Tehtävänanto ja työjärjestys: `TEHTAVA.md`. Suunnitteluvalinnat:
`DESIGN.md`. Kokeilut: `EXPERIMENTS.md`.

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
./target/release/soundpaste decode x.wav --expect "teksti"   # näyttää bittivirheiden paikat

# Oikea ääni (kysy lupa ensin!). --sim <profiili> ajaa saman hiljaa simulaattorilla.
./target/release/soundpaste audio-test          # mikrofonin taso, ei soita mitään
./target/release/soundpaste measure --gain 0.3 --note "..."   # kanavamittaus -> corpus/kanava
./target/release/soundpaste acoustic --gains 1.0,0.3,0.1      # modeemilähetys -> corpus/lahetys
./target/release/soundpaste corpus [--holdout]  # korpuksen listaus ja regressioajo (kehä B)
./target/release/soundpaste bench --holdout     # mukaan vertailuaineisto (vain vaiheiden lopussa)
./target/release/soundpaste explore             # modulaatiovaihtoehtojen vertailu (noin 3 min)
```

Älä putkita `measure`- tai `acoustic`-komentoa `head`:iin: katkennut putki
kaataa ohjelman ennen tallennusta.

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

- Vaiheet 1–2 valmiit: karkea v0, mittaristo, kanavamittaukset ja korpuksen alku.
  Perustaso: `bench/baseline.json` (v0-bfsk, ks. `EXPERIMENTS.md` kohdat 0–1).
- Vaihe 3: `DESIGN.md` on kirjoitettu ja **odottaa käyttäjän kommentteja**.
  Varsinaista modeemia (vaihe 4) ei aloiteta ennen niitä.
- Valmiina vaihetta 4 varten: `rs.rs` (Reed–Solomon pyyhkiytymineen) ja
  `explore.rs`, jonka sävelristikko ja MFSK-ilmaisu ovat uuden aaltomuodon pohja.

## Seuraavaksi (vaihe 4, kun suunnitelma on hyväksytty)

1. Uusi aaltomuoto `PhyKind::V1…`: 16 × 4-FSK, hyppy 4 ryhmää, 40+10 ms,
   pyyhkäisyalkumerkki + CFAR-ilmaisu, otsake ja Reed–Solomon (DESIGN.md luku 4).
2. Kokeilut yksi kerrallaan päiväkirjaan: 16×40+10 ms vs. 8×20+5 ms, tavujen
   sijoittelu (aika vs. taajuus), pyyhkiytymäkynnys, pariteetin määrä,
   huippujen leikkaus, alkumerkin pituus ja kynnys.
3. Akustinen kehä uudella aaltomuodolla (kysy lupa äänelle).
