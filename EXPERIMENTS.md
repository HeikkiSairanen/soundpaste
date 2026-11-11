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

## 2. v1: moniääni-MFSK, sävelhyppy, pyyhkäisyalkumerkki, Reed–Solomon (2026-11-11)

**Hypoteesi.** DESIGN.md:n aaltomuoto (16 × 4-FSK, hyppy 4 ryhmää, 40 + 10 ms,
pyyhkäisy + suhteellinen kynnys, RS pariteetilla 1/2) lyhentää 60 merkin
kehyksen noin 1,5 sekuntiin ja toimii kaikuisassa huoneessa, jossa v0 ei
toimi lainkaan.

**Tulos** (`bench/results/20261111T083553Z-d9cbeba-muutoksia-v1.json`):

| mittari | v0 | v1 |
|---|---|---|
| siirtoaika, 60 merkin kehys | 5,54 s | 1,44 s |
| onnistuminen nimellisoloissa | 86,3 % | 94,7 % (100 % ilman uutta halli-profiilia) |
| alin SNR, huone | +21 dB | -6 dB |
| alin SNR, kaikuisa (RT60 0,8 s) | ei yllä | -3 dB |
| nettonopeus | 10,8 tavua/s | 41,7 tavua/s |
| korjatut tavut / kehys nimellisoloissa | – | 0,7 (kapasiteetti 16) |
| oikea ääni Macilla, amplitudit 1,0…0,01 | 4/5 | 5/5 |

Hypoteesi piti. Marginaalia on nimellisoloissa paljon: virheenkorjauksesta
käytetään alle 5 %.

**Vika matkan varrella: alkumerkki laukesi sivukeilasta.** Ensimmäinen
versio epäonnistui mitatuilla kanavilla juuri silloin, kun kohinaa oli
vähiten (SNR 24–30 dB: 42–83 %). Syy: kun tausta on lähes äänetön,
suhteellinen kynnys ylittyy jo pyyhkäisyn autokorrelaation sivukeilasta
kymmeniä millisekunteja ennen päähuippua, ja huipun haku katsoi vain 15 ms
eteenpäin. Korjaus: kynnyksen ylityksestä haetaan voimakkain huippu yhden
alkumerkin mitan päähän asti. Tämän jälkeen mitatut kanavat 100 % kaikilla tasoilla.
Opetus: "liian hyvä" signaali on oma testitapauksensa.

**Uudet profiilit**, koska vanhat eivät enää erotelleet versioita: `kolahdus`
(kolme iskua kehyksen aikana), `puhe` (0,5 s puheen kaltaista häiriötä) ja
`halli` (RT60 1,2 s, kaiku 5 dB suoraa ääntä voimakkaampi).

**Muunnelmat** (sama mittaristo, alin SNR / onnistuminen):

| muunnelma | kaikuisa | halli (SNR >= 12 dB) | muuta | päätös |
|---|---|---|---|---|
| v1 (perus, pyyhkiytymät päällä) | -3 dB | 8–33 % | | **valittu** |
| `v1-noera`: ei pyyhkiytymiä | 0 dB | | muuten sama | hylätty: pyyhkiytymät ovat ilmainen 3 dB reunalla |
| `v1-8x20`: 8 alikanavaa, 20 + 5 ms | -3 dB | 0–8 % | sama nopeus | hylätty: pitkä symboli kestää pitkän kaiun paremmin |
| `v1-g2`: hyppy 2 ryhmää | +27 dB | 0 % | puolet kaistasta | hylätty: vahvistaa vertailukokeen tuloksen |
| `v1-time`: tavu = peräkkäiset symbolit | 0 dB | | kolahdus sama | hylätty: ei hyötyä, kaikuisassa hieman huonompi |
| `v1-p3`: pariteetti 1/3 | 0 dB | 0–25 % | 1,29 s; puhe -3 dB | hylätty toistaiseksi: 10 % nopeampi mutta 3 dB herkempi |

**Missä raja kulkee.**
- Kohina: noin -6 dB koko kaistan SNR:llä (puhtaalla kanavalla -9 dB).
- Kaiku: RT60 0,8 s ja suora/kaiku 0 dB toimii. `halli` (RT60 1,2 s, suora
  ääni 5 dB kaikua heikompi) ei toimi: alkumerkki ja otsake menevät läpi,
  mutta hyötykuorman virheet ylittävät Reed–Solomonin kapasiteetin (esim.
  18 korjattavaa, kun raja on noin 16). Uudelleenlähetys ei auta, koska kanava
  ei muutu. Tämä on se tapaus, johon DESIGN.md:n luvun 7 kestävä tila
  (pidemmät symbolit, enemmän pariteettia) olisi tarkoitettu.
- Kolahdukset ja puheen kaltainen häiriö eivät simulaatiossa kaada kehystä
  vielä -6 dB:n tasollakaan. Profiilit voivat olla liian helppoja; oikea puhe
  ja oikeat kolahdukset testataan kehässä E.
- Väärät hälytykset: taajuuspyyhkäisy (kanavamittaus) ja v0:n äänet
  laukaisevat alkumerkin kerran pari per äänite, mutta otsakkeen tarkistus
  hylkää ne 0,2 sekunnissa. Valkoinen kohina (20 s) ei laukaise kertaakaan.

**Testattu:** simulaatio (15 profiilia) ja oikea ääni Macin kaiuttimesta omaan
mikrofoniin (5 äänitettä korpuksessa). Ei vielä kahden koneen välillä.

## 3. Linkkikerros: ryöppy vai kehyskohtainen kuittaus, kättely vai pikalähetys (2026-11-11)

**Mittaustapa.** Mittausajo siirtyi sessiotilaan: kaksi asemaa, kummallekin
suunnalle oma huonevaste, oman kaiuttimen vuoto omaan mikrofoniin (neljä
kertaa toista asemaa voimakkaampana), arvotut ääniviiveet (lähtö 20–250 ms,
tulo 10–150 ms), eri näytteenottotaajuudet ja kellovirhe profiilin mukaan.
Onnistuminen vaatii, että lähettäjä sai vahvistuksen ja vastaanottaja näytti
täsmälleen oikean viestin täsmälleen kerran.

**Hypoteesi 1.** Ryöppy ja koontikuittaus on pitkällä viestillä noin
kolmanneksen nopeampi kuin kehyskohtainen kuittaus (DESIGN.md: 15,5 s vs. 23 s).

| 512 tavua, 6 siementä | ryöppy | 3 kehystä/kuittaus | kehyskohtainen |
|---|---|---|---|
| siirtoaika | 15,4–16,1 s | 17,1–18,6 s | 21,5–24,8 s |

Sama tulos profiileilla `huone` ja `kaikuisa`, SNR 20 dB ja 0 dB (ei
uudelleenlähetyksiä yhdessäkään ajossa, joten aika riippuu vain
ääniviiveistä). Hypoteesi piti, ja laskelma osui kohdalleen. **Päätös: ryöppy.**
Tätä ei ole mitattu kanavalla, jossa kehyksiä katoaa usein; siellä lyhyempi
ryöppy voisi voittaa, koska kadonnut kuittauspyyntö maksaa vähemmän.

**Hypoteesi 2.** Lyhyen viestin lähettäminen aloituskehyksen mukana
(pikalähetys) lyhentää 60 merkin linkin noin 2,6 sekuntiin heikentämättä
onnistumista.

| 60 merkin linkki, 15 profiilia | kättely | pikalähetys |
|---|---|---|
| siirtoaika, mediaani nimellisoloissa | 4,24 s | 2,82 s |
| onnistuminen nimellisoloissa | 93,6 % | 93,9 % |
| alin SNR, huone / kaikuisa | -6 / 0 dB | -6 / 0 dB |
| vastaanottajaa ei ole | noin 8–11 s | noin 8–11 s |

Hypoteesi piti. Pelkona oli, että puuttuva vastaanottaja selviäisi
hitaammin, koska aloituskehys on pidempi. Ratkaisu: vain ensimmäinen
aloitus kantaa viestin, ja uudet yritykset ovat lyhyitä. **Päätös:
pikalähetys oletukseksi.** Perustaso päivitetty:
`bench/results/20261111T085404Z-0cf87ca-muutoksia-v1.json`.

**Löydöksiä toteutuksen aikana.**
- Vastaanottaja lähetti kuittauksen kahdesti, kun kuittauspyynnön (POLL)
  alkumerkki tulkittiin alkavaksi dataryöpyksi. Korjattu: muu kuin datakehys
  nollaa ryöppyhavainnon. Löytyi kehystason testillä.
- Täsmälleen yhtäaikainen lähetys ei ratkea aina: satunnaisen odotuksen
  pitää erota yli alkumerkin havaitsemisviiveen (noin 0,2 s). Odotusväli
  levennettiin 0,3–1,2 sekunnista 0,2–2,2 sekuntiin, jolloin yli 93 %
  ratkeaa kolmella yrityksellä. Kirjattu tunnetuksi rajoitukseksi.
- Siirtoaika-arvio (DESIGN.md luku 6) osui mitattuun muutaman prosentin tarkkuudella.

**Testattu:** vain simulaatiossa (kehystaso: 33 testiä; äänitaso: 8 sessiotestiä
ja mittausajo). Linkkikerrosta ei ole vielä ajettu oikealla äänellä.

## 4. Kehä D: CLI ja selain samalla Macilla oikealla äänellä (2026-11-11)

**Tarkoitus.** Varmistaa wasm-kääre, AudioWorklet-liima ja linkkikerros
oikealla äänellä ennen toista konetta. Macin CLI (`soundpaste live`, ääni
SoX:n kautta) ja Chromen sivu jakavat saman kaiuttimen ja mikrofonin.

| testi | tulos |
|---|---|
| CLI -> selain, 60 merkin linkki (pikalähetys) | perillä 2,59 s, ei korjattavia tavuja |
| selain -> CLI, 43 tavua ääkkösin ja emojein | perillä 2,2 s (selaimen kello) |
| CLI -> selain, 300 tavua (kättely + 5 kehyksen ryöppy) | perillä 9,96 s, 5 korjattua tavua |
| wasm-moduulin vaihto lennossa kahdesti | mikrofoni pysyi päällä, siirto toimi vaihdon jälkeen (1,74 s) |

**Havainnot.**
- Kaikki toimi ensimmäisellä yrityksellä. Simulaation arviot (2,8 s ja noin
  10 s) osuivat oikeaan; oikea ääni oli hieman nopeampi, koska Chromen
  ilmoittama lähtöviive on vain noin 5 ms.
- Chrome kytki puheenkäsittelyn pois pyydetysti (kaiunpoisto,
  kohinanvaimennus ja automaattinen taso: false).
- Oma kaiku tunnistettiin molemmissa päissä, vaikka kaksi asemaa jakaa saman
  kaiuttimen.
- Selaimen mittaama SNR oli 23–30 dB ja taso -39 dBFS järjestelmän
  äänenvoimakkuudella 44 %.

**Rajaus.** Tämä on edelleen yhden laitteen kaiutin ja mikrofoni. Kahden
koneen välistä kanavaa (kehä E) ei ole vielä ajettu.
