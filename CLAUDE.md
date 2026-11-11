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
- `corpus/`: oikeat äänitteet (kanavamittaukset ja modeemilähetykset). Vain paikallisesti,
  ei versionhallinnassa (käyttäjän päätös: huoneäänitteitä ei julkaista).

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

# Selain ja kaksi konetta (README.md, kehät D ja E)
./rakenna.sh                                    # CLI + wasm + dist/
./target/release/soundpaste serve               # kehityspalvelin, http://localhost:8787/
./target/release/soundpaste live --send "x"     # asema oikealla äänellä (kysy lupa äänelle)
./target/release/soundpaste remote tila         # testiasemien tila; remote laheta/tila kaiku/keraa N
```

Älä putkita `measure`-, `acoustic`- tai `bench`-komentoa `head`:iin (ohjaa tiedostoon): katkennut putki
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

- Vaiheet 1–8 on toteutettu; kehä E (PC-testiasema) ja julkaisu odottavat käyttäjää.
  Kehä D (CLI + selain samalla Macilla) on ajettu oikealla äänellä.
- Oletusaaltomuoto `v1` (16 × 4-FSK, sävelhyppy, pyyhkäisyalkumerkki,
  Reed–Solomon), linkkikerros (`link/`), asema (`station.rs`) ja kaksisuuntainen
  simulaatio (`sim/duplex.rs`).
- Perustaso `bench/baseline.json`: sessiotila, 60 merkin linkki 2,82 s, onnistuminen
  93,9 % nimellisoloissa (puuttuva osa on `halli`-profiili, joka ei toimi), alin SNR
  huoneessa -6 dB.
- `soundpaste bench` ajaa oletuksena koko session (noin 35 s). `--oneway` mittaa
  pelkän fyysisen kerroksen (noin 18 s), `--quick` on nopea tarkistus.
- Tehdyt valinnat ja perusteet: `DESIGN.md` (luvut 5.8 ja 10) ja `EXPERIMENTS.md`.

## Tunnetut rajat (rehellisesti)

- `halli` (RT60 1,2 s, kaiku suoraa ääntä voimakkaampi) ei toimi. Korjaus olisi
  DESIGN.md:n luvun 7 kestävä tila; käyttäjä päätti jättää sen pois, kunnes käytäntö vaatii.
- Kahden koneen välillä on ajettu yksi istunto (PC:n selain -> Mac toimii, EXPERIMENTS.md kohta 5).
  Mac -> PC vaati kovan äänenvoimakkuuden. PC:n päästä ei ole lokeja eikä kanavamittausta.
- Täsmälleen yhtäaikainen lähetys (alle 0,2 s) ratkeaa noin 93 %:ssa tapauksista.

## Seuraavaksi kokeilemisen arvoista

- Oikean kahden koneen kanavan mittaus (PC-testiasema) ja sen lisääminen profiileiksi.
- Pariteetti 1/3 (`v1-p3`): 10 % nopeampi, 3 dB herkempi. Päätös oikean datan perusteella.
- Alkumerkin havaitsemisviiveen (noin 150 ms) lyhentäminen: pienentäisi törmäysikkunaa.
- Häiriöprofiilit (`kolahdus`, `puhe`) ovat simulaatiossa liian helppoja: oikeat äänitteet tilalle.
