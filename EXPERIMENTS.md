# Kokeilupäiväkirja

Jokaisesta kokeilusta kirjataan hypoteesi, muutos, mittaustulos ja johtopäätös,
myös silloin kun kokeilu epäonnistui. Uusin alimpana.

Mittarit tulevat komennosta `soundpaste bench` (ks. CLAUDE.md). SNR on
signaalin ja kohinan tehosuhde mikrofonissa koko kaistalla 0..fs/2. "Alin SNR"
on pienin 3 dB:n ruudukon piste, jolla ja jonka yläpuolella vähintään 90 %
siirroista onnistuu.

## 0. Perustaso: v0-bfsk (2026-11-11)

**Tarkoitus.** Saada kehä A pyörimään yksinkertaisimmalla mahdollisella
modeemilla. Ei tavoitella hyvää modeemia.

**Toteutus.** 2-FSK (2000/3000 Hz), 100 baudia, vuorotteleva alkumerkki ja
16-bittinen tahdistussana, CRC-16, ei virheenkorjausta. Yksisuuntainen, yksi
kehys.

**Tulos** (`bench/results/20261111T075213Z-ei-commitia-v0-bfsk.json`, 12 toistoa/piste):

| mittari | arvo |
|---|---|
| siirtoaika, 60 merkin linkki (yksi kehys, ei kuittauksia) | 5,54 s |
| onnistuminen nimellisoloissa (SNR >= 12 dB) | 84,5 % |
| alin SNR, puhdas kanava | -9 dB |
| alin SNR, huone (RT60 0,35 s) | +21 dB |
| alin SNR, kaikuisa (RT60 0,8 s) | ei yllä 90 %:iin millään tasolla |
| BER ennen korjausta, nimellisolot | 7,5e-3 |
| nettonopeus | 10,8 tavua/s |

**Johtopäätös.**
- Kohina ei ole v0:n ongelma: puhtaalla kanavalla se toimii -9 dB:iin asti,
  koska 10 ms:n symboli kerää tehoa kapealta kaistalta.
- Kaiku on ongelma. Huoneessa onnistuminen jää noin 90 %:iin signaalin
  voimakkuudesta riippumatta (virheet eivät vähene, kun SNR kasvaa), ja
  kaikuisassa huoneessa mikään ei mene läpi. Edellisen symbolin kaiku osuu
  samalle taajuudelle kuin nykyinen symboli, eikä kahdella taajuudella ole
  mihin väistää.
- Yksikin bittivirhe 504 bitin kehyksessä kaataa koko kehyksen, koska
  virheenkorjausta ei ole.
- Seuraavat kokeilut (vaihe 4): useampi taajuus ja taajuuden vaihto symbolien
  välillä kaikua vastaan, Reed-Solomon, parempi tahdistus.

## 1. Oikea ääni Macilla: kanavamittaus ja v0 kaiuttimesta mikrofoniin (2026-11-11)

**Tarkoitus.** Saada kehät B ja C pyörimään ja mitata kanava ennen
suunnittelupäätöksiä. Laite: MacBook Pro (MacBookPro17,1), omat kaiuttimet ja
oma mikrofoni, järjestelmän äänenvoimakkuus 44 %, työhuone.

**Kanavamittaus** (`soundpaste measure`, pyyhkäisy 100 Hz – 20 kHz, kolme
amplitudia 1,0 / 0,3 / 0,1; hiljaisin on vertailuaineistoa):

| suure | arvo | merkitys |
|---|---|---|
| vahvistus 1–8 kHz | -25…-33 dB, tasainen ±4 dB | käyttökelpoinen kaista |
| vahvistus 10–20 kHz | -37…-42 dB | noin 12 dB heikompi, mutta toimii 20 kHz:iin asti |
| taustamelu / 50 Hz | -78 dBFS @ 200 Hz, -98 @ 1 kHz, -105 @ 2–8 kHz | melu on matalilla taajuuksilla |
| suora ääni / kaiku | 11–12 dB | oma mikrofoni on aivan kaiuttimen vieressä |
| viivehaje | 3–4 ms, koherenssikaista noin 50–70 Hz | kanava on taajuusvalikoiva jo omassa koneessa |
| energia > 10 / 20 / 40 ms suoran äänen jälkeen | -14 / -20 / -27 dB | symbolien välinen häiriö |
| oma kaiku kohinapohjassa (-55 dB) | 62 ms | suoja-ajan mitoitus |
| toinen harmoninen särö | -40 dB (amplitudi 1,0), yhdessä mittauksessa -24 dB | rajoittaa tiheitä konstellaatioita |

RT60-arvio (0,05–0,17 s) ei ole luotettava: kaiun häntä hukkuu taustameluun
ennen kuin se on vaimennut 25 dB, koska suora ääni on niin hallitseva.
Mittaus kuvaa oman kaiun polkua, ei toisen koneen kuulemaa kanavaa. Toista
konetta arvioidaan johdetulla profiililla `:kaukana` (suora ääni -14 dB).

**v0 oikealla äänellä** (`soundpaste acoustic`, viisi voimakkuutta):

| amplitudi | 1,0 | 0,3 | 0,1 | 0,03 | 0,01 |
|---|---|---|---|---|---|
| tulos | ok | ok | ok | ok | ei havaittu |

**Havainnot.**
- Ensimmäisellä yrityksellä kolme neljästä epäonnistui, ja virhe oli joka
  kerta kehyksen viimeisessä bitissä: SoX:n `play` katkaisee äänen lopusta.
  Korjaus on mittaustyökalussa (hiljaisuutta toiston molempiin päihin), mutta
  opetus koskee myös modeemia: kehyksen viimeinen symboli ei saa olla
  viimeinen asia, jonka kaiutin soittaa.
- Signaalin taso mikrofonissa on matala (-33 dBFS täydellä amplitudilla),
  eli leikkautuminen ei ole tällä laitteella ongelma, mutta tasosta
  riippumaton ilmaisu on välttämätön.
- Mitatulla kanavalla ja mitatulla melulla v0 toimii vielä -15 dB:n
  SNR:llä, koska melu on kaistan ulkopuolella. Koko kaistan SNR on siis
  huono mittari oikealle huoneelle, ja kaistan valinta ratkaisee paljon.
- Johdetulla `:kaukana`-profiililla v0 jää 50–60 %:iin tasosta
  riippumatta: kaiku kaataa sen, kuten synteettisissäkin huoneissa.

**Perustaso** päivitetty: `bench/results/20261111T081049Z-b1d2c55-muutoksia-v0-bfsk.json`
(mukana mitatut kanavat, korpus 4/5).
