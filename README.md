# soundpaste

Siirrä lyhyt teksti (linkki, salasana, koodinpätkä) viereiselle koneelle
äänellä. Toinen kone soittaa kaiuttimesta, toinen kuuntelee mikrofonilla ja
kuittaa äänellä takaisin. Palvelinta ei ole: kaikki tapahtuu selaimessa.

- Sivu: https://sairanen.org/soundpaste tai https://heikkisairanen.github.io/soundpaste/ (julkaisu: ks. [Julkaisu](#julkaisu))
- Suunnitelma ja perustelut: [DESIGN.md](DESIGN.md)
- Kokeilupäiväkirja mittaustuloksineen: [EXPERIMENTS.md](EXPERIMENTS.md)
- Alkuperäinen tehtävänanto: [TEHTAVA.md](TEHTAVA.md)

60 merkin linkki siirtyy kuittauksineen noin 2,5 sekunnissa ja 512 tavun
enimmäisviesti noin 16 sekunnissa. Siirto ei ole salattu.

## Rakenne

| hakemisto | sisältö |
|---|---|
| `crates/modem-core` | Ydin, ei riippuvuuksia: signaalinkäsittely (`dsp`), aaltomuodot (`phy`), Reed–Solomon (`rs`), linkkikerros (`link`), asema (`station`), kanavasimulaattori (`sim`), kanavamittaus (`measure`) |
| `crates/cli` | `soundpaste`-komento: koodaus, mittausajot, oikean äänen kehät, kehityspalvelin. Ei riippuvuuksia |
| `crates/wasm` | WebAssembly-kääre: C-rajapinta ilman wasm-bindgeniä |
| `web/` | Julkaistava sivu: `index.html`, `style.css`, `app.js` (pääsäie), `worklet.js` (äänisäie) |
| `dev/` | Testiaseman selainkoodi. Vain kehityspalvelin tarjoilee sen |
| `bench/` | Mittausajojen tulokset (`results/`), perustaso (`baseline.json`), yksittäiset kokeet (`kokeet/`) |
| `corpus/` | Oikeat äänitteet: kanavamittaukset (`kanava/`) ja modeemilähetykset (`lahetys/`). Ei versionhallinnassa: syntyy omilla mittauksilla |

Sama tilakone (`Station`: modeemi + linkkikerros) pyörii selaimessa,
komentorivillä ja simulaattorissa.

## Kääntäminen

Tarvitaan Rust ja wasm-kohde (`rustup target add wasm32-unknown-unknown`).
Ei npm:ää, ei bundleria, ei ulkoisia crateja.

```sh
./rakenna.sh            # kääntää CLI:n ja wasm-moduulin, kokoaa sivun hakemistoon dist/
./rakenna.sh --testit   # ajaa lisäksi testit
cargo test -q           # pelkät testit
```

`dist/` on valmis staattinen sivu, joka toimii missä tahansa alihakemistossa
(suhteelliset polut). Mikrofoni vaatii selaimessa https-yhteyden tai
localhostin.

## Kehityssykli

Periaate: jokainen muutos mitataan yhdellä komennolla, ja suurin osa työstä
tapahtuu hiljaa Macilla. Kehät nopeimmasta hitaimpaan:

### A. Simulaatio (sekunteja, hiljainen)

```sh
cargo test -q
./target/release/soundpaste bench            # koko sessio 15 kanavaprofiililla, noin 35 s
./target/release/soundpaste bench --quick    # nopea tarkistus
./target/release/soundpaste bench --oneway   # pelkkä fyysinen kerros
./target/release/soundpaste compare          # uusin ajo vs. perustaso; paluuarvo 1, jos jokin heikkeni
./target/release/soundpaste baseline         # uusin ajo perustasoksi (vain kun muutos jää voimaan)
./target/release/soundpaste session --profile kaikuisa --snr 3 --len 300   # yksi sessio lokeineen
./target/release/soundpaste sim --profile halli --snr 20 --seed 2          # yksi kehys
```

Pistetaulussa on aina samat mittarit: 60 merkin linkin siirtoaika,
onnistumisosuus, alin toimiva signaali-kohinasuhde, bittivirhesuhde ennen
virheenkorjausta, korjattujen tavujen määrä ja nettonopeus. Tulokset
tallentuvat hakemistoon `bench/results/` commit-tunnisteen kanssa.

### B. Äänitekorpus (sekunteja, hiljainen)

```sh
./target/release/soundpaste corpus             # listaa korpuksen ja purkaa äänitteet nykyisellä vastaanottimella
./target/release/soundpaste corpus --holdout   # mukaan vertailuaineisto
```

Mitatut kanavat (`corpus/kanava/`) ovat automaattisesti mittausajon
profiileina (`m:...`), ja lähetysäänitteet (`corpus/lahetys/`) ajetaan
jokaisen mittausajon yhteydessä. Osa aineistosta on merkitty
vertailuaineistoksi (`"holdout": true`), jota ei käytetä virittämiseen.

Korpus on huoneäänitteitä, joten sitä ei jaeta lähdekoodin mukana. Ilman sitä
mittausajo toimii pelkillä synteettisillä profiileilla, ja `bench/`-hakemiston
tulosten korpusrivit ja `m:`-profiilit jäävät pois. Oman korpuksen saa kohdan C
komennoilla.

### C. Akustinen kehä Macilla (kymmeniä sekunteja, kuuluu ääntä)

Vaatii SoX:n (`rec`, `play`) ja päätteelle mikrofoniluvan.

```sh
./target/release/soundpaste audio-test                      # mikrofonin taso, ei soita mitään
./target/release/soundpaste measure --note "työhuone"        # kanavamittaus pyyhkäisyllä -> corpus/kanava
./target/release/soundpaste acoustic --gains 1.0,0.3,0.1     # modeemilähetys usealla voimakkuudella -> corpus/lahetys
```

Lisää `--sim huone`, niin sama komento ajetaan hiljaa simulaattorin läpi.

### D. Selain samalla Macilla (minuutti)

```sh
./target/release/soundpaste serve &                          # kehityspalvelin, http://localhost:8787/
# avaa selaimessa http://localhost:8787/?asema=mac-selain ja salli mikrofoni
./target/release/soundpaste live --send "teksti"             # CLI lähettää, selain vastaanottaa
./target/release/soundpaste remote laheta "teksti"           # selain lähettää...
./target/release/soundpaste live --once --expect "teksti"    # ...ja CLI vastaanottaa
```

`live` on sama asema kuin selaimessa, mutta ääni kulkee SoX:n kautta.

### E. Kaksi konetta (minuutteja)

PC on passiivinen testiasema: käyttöönoton jälkeen siihen ei tarvitse koskea.

**Kertaluonteinen käyttöönotto PC:llä (Windows, Chrome tai Edge):**

1. Käynnistä Macilla `./target/release/soundpaste serve`. Se tulostaa
   lähiverkon osoitteen muodossa `http://<macin-nimi>.local:8787/`. Salli
   yhteydet, jos macOS:n palomuuri kysyy.
2. Tee PC:n työpöydälle pikakuvake, jonka kohde on (yhdellä rivillä, vaihda
   Macin osoite):

   ```text
   "C:\Program Files\Google\Chrome\Application\chrome.exe"
     --user-data-dir=C:\soundpaste-testi
     --unsafely-treat-insecure-origin-as-secure=http://<macin-nimi>.local:8787
     --autoplay-policy=no-user-gesture-required
     --use-fake-ui-for-media-stream
     "http://<macin-nimi>.local:8787/?testi=vastaanotto&asema=pc"
   ```

   Liput: erillinen selainprofiili (ei vaikuta muuhun selaamiseen), Macin
   http-osoite kohdellaan turvallisena (mikrofoni vaatii sen), ääni sallitaan
   ilman napinpainallusta ja mikrofonilupa hyväksytään automaattisesti.
   Edgellä polku on `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`.
   Jos `.local`-nimi ei toimi, käytä Macin IP-osoitetta ja tee reitittimeen
   kiinteä osoitevaraus.
3. Windowsin ääniasetuksista mikrofonin tehosteet pois (kohinanvaimennus,
   "Voice Clarity", valmistajan tehosteet), ei lepotilaa eikä näytön
   lukitusta, selainikkuna näkyvissä, koneet 0,5–1 m päässä toisistaan.

**Käyttö Macilta:**

```sh
./target/release/soundpaste remote tila                      # onko asema hengissä
./target/release/soundpaste remote tila kaiku                # testitila: vastaanotto | kaiku | majakka [väli s]
./target/release/soundpaste live --send "teksti"             # Mac lähettää, PC vastaanottaa (kaikutilassa lähettää takaisin)
./target/release/soundpaste remote laheta "teksti"           # PC lähettää
./target/release/soundpaste remote keraa 5                   # seuraavat 5 PC:n sessioäänitettä korpukseen
./target/release/soundpaste measure --kaukaa pc              # kanava PC:n kaiuttimesta Macin mikrofoniin
./target/release/soundpaste measure --kauas pc               # kanava Macin kaiuttimesta PC:n mikrofoniin
```

Testiasema tarkistaa kahden sekunnin välein, onko uusi versio: uusi
wasm-moduuli vaihdetaan lennossa (mikrofoni pysyy päällä) ja muuttunut sivu
ladataan uudelleen. Lokit tulostuvat palvelimen päätteeseen ja tallentuvat
hakemistoon `devdata/`, samoin jokaisen session mikrofoniäänite.

Mikään tästä ei päädy julkaistavaan sivuun: `rakenna.sh` kokoaa `dist/`:n
vain `web/`-hakemistosta ja keskeyttää, jos tiedostoista löytyy verkko-osoite
tai kehitystyökalujen jälki. Sivun CSP sallii yhteydet vain omaan hakemistoon.

### Parannussykli

1. Kirjaa hypoteesi `EXPERIMENTS.md`:hen: mitä muutetaan ja minkä mittarin odotetaan paranevan.
2. Tee muutos, aja kehät A ja B (`cargo test`, `bench`, `compare`).
3. Jos tulos on lupaava, aja kehä C ja myöhemmin D ja E.
4. Kirjaa tulos ja johtopäätös, myös epäonnistuneet kokeilut.
5. Jos muutos jää voimaan: `baseline` ja commit.

Uusi aaltomuoto on uusi rivi taulukossa `MFSK_PRESETS`
(`crates/modem-core/src/phy/mod.rs`), ja sitä voi mitata valinnalla `--phy <nimi>`.

## Julkaisu

```sh
./julkaise.sh -n    # kääntää, testaa ja näyttää, mitä lähetettäisiin
./julkaise.sh       # lähettää dist/-hakemiston osoitteeseen sairanen.org/soundpaste
./julkaise-github.sh   # sama sisältö GitHub Pagesiin (haara gh-pages)
```

Tarvitsee tiedoston `.julkaisu` (rivi `ZONER_USER=tunnus`, ei versionhallinnassa)
ja SSH-avaimen Zonerin ohjauspaneelissa. Skripti tarkistaa lopuksi, että
palvelimen tiedostot vastaavat paikallisia ja että wasm tarjoillaan oikealla
MIME-tyypillä.

## Tunnetut rajoitukset

- Vain kaksi laitetta kerrallaan: useampi vastaanottaja kuittaa päällekkäin.
- Ei salausta eikä todennusta.
- Hyvin kaikuisa tila (jälkikaiunta yli noin sekunnin ja koneet kaukana
  toisistaan) ei toimi; kestävämpi tila on suunniteltu mutta ei toteutettu.
- Tarkemmin: DESIGN.md luku 8 ja CLAUDE.md.

## Lisenssi

MIT, katso `LICENSE`.
