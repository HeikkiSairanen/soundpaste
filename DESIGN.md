# soundpaste: suunnitelma

Tila: **ehdotus, odottaa kommentteja** (vaihe 3). Perustuu vaiheiden 1–2
mittauksiin (ks. `EXPERIMENTS.md`). Fyysisen kerroksen parametrit ovat
lähtöarvoja, jotka vaiheen 4 parannussykli vahvistaa tai kumoaa.

Sisältö:
1. [Päätökset lyhyesti](#1-päätökset-lyhyesti)
2. [Mitattu kanava](#2-mitattu-kanava)
3. [Modulaation valinta](#3-modulaation-valinta)
4. [Fyysinen kerros](#4-fyysinen-kerros)
5. [Linkkikerros](#5-linkkikerros)
6. [Aika-arviot](#6-aika-arviot)
7. [Laajennettavuus](#7-laajennettavuus)
8. [Tunnetut rajoitukset](#8-tunnetut-rajoitukset)
9. [Mitä on mitattu ja mitä ei](#9-mitä-on-mitattu-ja-mitä-ei)
10. [Kysymykset sinulle](#10-kysymykset-sinulle)

## 1. Päätökset lyhyesti

| asia | ehdotus | tärkein peruste |
|---|---|---|
| modulaatio | 16 rinnakkaista 4-FSK-alikanavaa, energiailmaisu | kestää kaiun ilman vaiheen seurantaa, 8-kertainen nopeus yhteen säveleen verrattuna |
| kaiun torjunta | sävelhyppy: peräkkäiset symbolit käyttävät eri sävelryhmää (4 ryhmää) | virhesuhde kaikuisassa huoneessa 7,8 % -> 0,5 % |
| kaista | 1000–7400 Hz, 256 säveltä 25 Hz:n välein | mitattu vaste tasainen 1–8 kHz, melu 20–27 dB hiljaisempaa kuin alle 500 Hz |
| symboli | 50 ms (40 ms ilmaisuikkuna + 10 ms suoja-aika), 4 tavua/symboli | raaka nopeus 640 bit/s |
| tahdistus | taajuuspyyhkäisy (chirp) alkumerkkinä, sovitettu suodin | terävä ajoitus kaiussa, riippumaton tasosta ja näytteenottotaajuudesta |
| virheenkorjaus | Reed–Solomon GF(256), pariteettia puolet datan määrästä, pyyhkiytymät käytössä | korjaa 16 % tavuista virheinä tai 33 % pyyhkiytyminä |
| kuittaus | datakehykset ryöppynä, yksi koontikuittaus (bittikartta) | enimmäisviestillä noin 15 s vs. 23 s kehyskohtaisella kuittauksella |
| oma kaiku | vastaanotin käy koko ajan; oma kehys tunnistetaan sisällöstä ja siitä ankkuroidaan aikakatkaisut | ei riipu selaimen tuntemattomasta viiveestä |
| 60 merkin linkki | noin 4,1 s kättelyineen (2,6 s, jos lyhyt viesti kulkee aloituskehyksessä) | ks. luku 6 |

## 2. Mitattu kanava

Mittaus: MacBook Pro, omat kaiuttimet -> oma mikrofoni, työhuone,
järjestelmän äänenvoimakkuus 44 %, kolme pyyhkäisyä (100 Hz – 20 kHz).
Äänitteet ja johdetut impulssivasteet: `corpus/kanava/`.

| suure | mitattu | seuraus suunnittelulle |
|---|---|---|
| taajuusvaste | tasainen ±4 dB välillä 1–8 kHz; 10–20 kHz noin 12 dB heikompi mutta käyttökelpoinen | kaista 1–7,4 kHz; lähes kuulumaton tila 18–20 kHz on mahdollinen myöhemmin |
| taustamelu 50 Hz:n kaistalla | -78 dBFS @ 200 Hz, -98 @ 1 kHz, -105 @ 2–8 kHz | ei käytetä alle 1 kHz:n taajuuksia; koko kaistan SNR aliarvioi oikean huoneen |
| yhden sävelen SNR (täysi amplitudi, 50 Hz) | 62–80 dB omassa mikrofonissa | kohinaan on varaa kymmeniä desibelejä: teho voidaan jakaa usealle sävelelle |
| viivehaje | 3–4 ms, koherenssikaista 50–70 Hz | kaista (6,4 kHz) on noin 100 kertaa koherenssikaista: eri sävelet häipyvät toisistaan riippumatta |
| energia myöhemmin kuin 10 / 20 / 40 ms | -14 / -20 / -27 dB | lyhyt symboli ilman hyppyä kärsii edellisen symbolin kaiusta |
| oma kaiku kohinapohjassa | 62 ms (55 dB dynamiikalla) | vastausviive 150 ms riittää tässä huoneessa |
| toinen harmoninen särö | -40 dB, yhdessä mittauksessa -24 dB | tiheät konstellaatiot (QAM) eivät toimi pienillä kaiuttimilla; FSK:lle riittää |
| signaalin taso mikrofonissa | -33 dBFS täydellä amplitudilla | ilmaisun pitää olla tasosta riippumaton |

Karkea v0-modeemi (2-FSK, 100 baudia) oikealla äänellä: 4/5 lähetystä
onnistui; hiljaisin (amplitudi 0,01) jäi havaitsematta. Simulaatiossa v0
kaatuu kaikuun: kaikuisassa huoneessa mikään ei mene läpi.

**Rajoitus:** mittaus kuvaa polkua omasta kaiuttimesta omaan mikrofoniin
(suora ääni 11–12 dB kaikua voimakkaampi). Toinen kone metrin päässä kuulee
suoran äänen arviolta 14 dB heikompana ja kaiun suunnilleen samana.
Tätä arvioidaan johdetulla profiililla `:kaukana` ja synteettisillä
huoneilla, kunnes PC-testiasema (vaihe 7) tuo oikean kahden koneen mittauksen.

## 3. Modulaation valinta

Vertailukoe (`soundpaste explore`, tulos: `bench/kokeet/20261111-modulaatiovertailu.txt`):
jokaisella vaihtoehdolla lähetettiin satunnaisia symboleita kanavaprofiilien
läpi ja mitattiin raaka bittivirhesuhde ihanteellisella tahdistuksella, ilman
virheenkorjausta. Sama kokonaisteho kaikilla. Luvut ovat bittivirheprosentteja.

| vaihtoehto | bit/s | kaista | puhdas 0 dB | huone 0 dB | kaikuisa 20 dB | kaikuisa 5 dB | mitattu:kaukana 20 dB |
|---|---|---|---|---|---|---|---|
| 2-FSK, 10 ms (kuten v0) | 100 | 0,2 kHz | 0,00 | 0,00 | 14,3 | 5,3 | 0,00 |
| 16-FSK, yksi sävel, 40+10 ms | 80 | 0,4 kHz | 0,00 | 0,00 | 5,1 | 5,0 | 1,35 |
| sama + hyppy (4 ryhmää) | 80 | 1,6 kHz | 0,00 | 0,00 | 0,42 | 0,10 | 0,00 |
| 16 × 4-FSK, ei hyppyä | 640 | 1,6 kHz | 0,00 | 0,22 | 7,8 | 8,4 | 0,50 |
| 16 × 4-FSK, hyppy 2 ryhmää | 640 | 3,2 kHz | 0,00 | 0,03 | 2,7 | 2,7 | 0,09 |
| **16 × 4-FSK, hyppy 4 ryhmää** | **640** | **6,4 kHz** | **0,00** | **0,04** | **0,47** | **1,09** | **0,04** |
| 8 × 4-FSK, 20+5 ms, hyppy 4 ryhmää | 640 | 6,4 kHz | 0,00 | 0,01 | 0,45 | 0,53 | 0,15 |
| 8 × 16-FSK, hyppy 2 ryhmää | 640 | 6,4 kHz | 0,00 | 0,00 | 1,40 | 2,04 | 0,06 |
| DQPSK, 256 säveltä (OFDM), 40+10 ms | 10 240 | 6,4 kHz | 9,2 | 13,6 | 20,1 | 23,4 | 4,1 |
| DQPSK, 512 säveltä, 80+20 ms | 10 240 | 6,4 kHz | 8,5 | 12,1 | 14,7 | 18,5 | 1,3 |

Profiilit: "huone" RT60 0,35 s ja suora/kaiku 8 dB, "kaikuisa" RT60 0,8 s ja
suora/kaiku 0 dB, "mitattu:kaukana" tämän huoneen mitattu vaste, jossa suoraa
ääntä on vaimennettu 14 dB. SNR on koko kaistan (0–24 kHz) valkoista kohinaa vastaan.

### Mitä koe kertoo

**Kaiku on vihollinen, ei kohina.** Kaikki FSK-vaihtoehdot ovat virheettömiä
puhtaalla kanavalla vielä 0 dB:n SNR:llä, mutta kaikuisassa huoneessa
virheitä tulee 5–14 %, eikä signaalin voimistaminen auta (20 dB ja 5 dB
antavat saman tuloksen). Edellisen symbolin kaiku soi yhä, kun seuraava
symboli ilmaistaan.

**Sävelhyppy poistaa suurimman osan kaiun vaikutuksesta.** Kun peräkkäiset
symbolit käyttävät eri sävelryhmää, edellisen symbolin kaiku ei osu nykyisen
symbolin ehdokassäveliin. Sama taajuus tulee uudelleen käyttöön vasta neljän
symbolin (200 ms) päästä. Neljällä ryhmällä virhesuhde putoaa 7,8 %:sta
0,5 %:iin; kahdella ryhmällä 2,7 %:iin. Hinta on kaistanleveys, ja sitä on
mittauksen mukaan käytettävissä.

**Rinnakkaiset sävelet antavat nopeuden.** Yksi 16-FSK-sävel siirtää 80
bittiä sekunnissa, mikä tekisi 60 merkin linkistä yli 10 sekunnin siirron.
16 rinnakkaista 4-FSK-alikanavaa siirtää 640 bit/s samalla
kaikukestävyydellä. Hinta on teho per sävel (-24 dB, kun huipputaso on
rajattu), mutta mitattu sävelen SNR on 62–80 dB omassa mikrofonissa, joten
varaa on. 4-FSK on tehokkain valinta kaistaa kohden: bittejä per sävel on
log2(M)/M, joka on suurimmillaan, kun M on 2 tai 4.

**Vaihemodulaatio (PSK/OFDM) ei sovi luotettavuutta painottavaan ensimmäiseen
versioon.** DQPSK siirtäisi 16 kertaa enemmän bittejä, mutta virhesuhde on
15–23 % kaikuisassa huoneessa ja 9–14 % jo pelkässä kohinassa. Ihanteellisella
virheenkorjauksella siitä saisi silti 2–3 kbit/s, mutta se vaatisi pehmeään
päätökseen perustuvan matalan koodisuhteen koodin (konvoluutio- tai
LDPC-koodi), kanavan estimoinnin tai pidemmät symbolit, ja se on herkkä
kaiuttimen särölle (mitattu -24…-40 dB) ja laitteiden kellojen erolle.
Yksikantoaaltoinen laajakaistainen PSK tarvitsisi lisäksi ekvalisaattorin,
koska kaista on 100 kertaa koherenssikaista. OFDM on luonteva pohja
myöhemmälle nopealle tilalle, ei ensimmäiselle versiolle.

**Chirp (taajuuspyyhkäisy) datan siirtoon** levittäisi jokaisen symbolin koko
kaistalle ja kestäisi kapeakaistaista häiriötä, mutta sen tuoma
käsittelyvahvistus kohinaa vastaan on turha, kun kohina ei ole rajoittava
tekijä. Tahdistukseen se sen sijaan on paras vaihtoehto (ks. 4.3).

### Valinta

**16 × 4-FSK, sävelhyppy neljällä ryhmällä, 40 + 10 ms.** Lähes yhtä hyvä
ehdokas on 8 × 4-FSK 20 + 5 ms:n symboleilla (sama nopeus ja kaista,
puolet sävelistä yhtä aikaa, mutta tiukempi ajoitusvaatimus). Vaihe 4 vertaa
näitä kahta koko kehyksellä, tahdistus ja virheenkorjaus mukana.

Huippu- ja tehollisarvon suhde (PAPR) on 16 sävelellä noin 13 dB ja 8
sävelellä noin 12 dB. Kaiutin rajaa huipputason, joten rinnakkaiset sävelet
maksavat tehoa. Leikkauskoe: ks. [luku 9](#9-mitä-on-mitattu-ja-mitä-ei).

## 4. Fyysinen kerros

### 4.1 Sävelristikko ja symboli

- Sävelet: f_i = 1000 Hz + i · 25 Hz, i = 0…255 (1000–7375 Hz).
- Sävelväli 25 Hz = 1 / 40 ms: sävelet ovat keskenään ortogonaalisia 40 ms:n
  ilmaisuikkunassa.
- 16 alikanavaa, kussakin 16 säveltä: 4 arvoa × 4 hyppyryhmää.
  Alikanavan s, arvon v (0–3) ja symbolin t sävel on
  `i = 16·s + 4·v + (t mod 4)` ("lomitettu": ehdokkaat ovat 100 Hz:n välein,
  ja edellisten symbolien sävelet jäävät niiden väliin).
- Symboli kestää 50 ms: 10 ms suoja-aikaa ja 40 ms ilmaisuikkuna. Suoja-aika
  nielee ajoitusvirheen ja varhaiset heijastukset (mitattu: energia yli 10 ms
  myöhässä on -14 dB). Sävelet nousevat ja laskevat pehmeästi suoja-ajan sisällä.
- Jokainen symboli kantaa 16 × 2 = 32 bittiä eli 4 tavua.
- Jokaisella sävelellä on kiinteä näennäissatunnainen alkuvaihe, jotta 16
  säveltä eivät summaudu yhdeksi piikiksi.

Vastaanotin laskee jokaisesta symbolista ehdokassävelten energiat
(Goertzel-tyyppinen korrelaatio täsmälleen 40 ms:n ikkunassa) ja valitsee
kussakin alikanavassa voimakkaimman. Ikkunan pituus määritellään sekunteina,
joten sama toimii millä tahansa näytteenottotaajuudella (48 kHz: 1920
näytettä, 44,1 kHz: 1764 näytettä). FFT:tä ei käytetä ilmaisuun, koska sen
pituus pitäisi sovittaa kahden potenssiin.

Jos alikanavan kahden voimakkaimman ehdokkaan ero on pieni, tavu merkitään
pyyhkiytymäksi. Reed–Solomon korjaa pyyhkiytymiä kaksi kertaa niin monta kuin
virheitä.

### 4.2 Tavujen sijoittelu symboleihin

Jokainen alikanava kuljettaa omaa tavuvirtaansa: tavu vie neljä peräkkäistä
symbolia yhdessä alikanavassa. Peruste: taajuusvalikoiva häipyminen on kehyksen
ajan pysyvää (laitteet eivät liiku), joten kuollut alikanava pilaa 1/16
tavuista eikä palaa jokaisesta tavusta. Lyhyt laajakaistainen häiriö
(kolahdus) osuu sen sijaan kaikkiin alikanaviin kerralla. Vaihe 4 mittaa
tämän ja vaihtoehtoisen sijoittelun (tavu = neljä alikanavaa yhdessä symbolissa)
kolahdus- ja kaikuprofiileilla.

### 4.3 Alkumerkki ja tahdistus

Alkumerkki on taajuuspyyhkäisy 1000 -> 7400 Hz (noin 60 ms). Vastaanotin
korreloi tulosignaalia pyyhkäisyn mallia vastaan (sovitettu suodin,
laskettuna lohkoittain FFT:llä).

- Pyyhkäisyn kaistanleveys (6,4 kHz) antaa ajoitustarkkuudeksi noin 0,2 ms,
  kun symbolin suoja-aika on 10 ms.
- Kaiku näkyy korrelaatiossa erillisinä myöhempinä huippuina, eikä levitä
  päähuippua. Ajoitus otetaan ensimmäisestä vahvasta huipusta (suora ääni).
- Kapeakaistainen häiriö tai häipymä pilaa vain osan pyyhkäisystä.
- Havaintokynnys on suhteellinen: huippua verrataan korrelaation omaan
  taustatasoon juuri ennen huippua (CFAR). Näin kynnys ei riipu signaalin
  tasosta eikä kaistan ulkopuolisesta melusta.
- Malli lasketaan vastaanottimen omalla näytteenottotaajuudella, joten
  44,1/48 kHz:n ero ei vaikuta. 100 ppm:n kellovirhe siirtää ajoitusta
  3 sekunnin kehyksessä 0,3 ms, mikä mahtuu suoja-aikaan.

v0:n opetus: kehyksen perään tulee 20 ms:n häntä, jotta viimeinen symboli ei
ole viimeinen asia, jonka kaiutin soittaa.

### 4.4 Kehys

```text
| alkumerkki 60 ms | tauko 10 ms | otsake 3 symbolia | hyötykuorma N symbolia | häntä 20 ms |
```

**Otsake** (aina perustilassa, 12 tavua = 3 symbolia):

| tavu | sisältö |
|---|---|
| 0 | bitit 7–5: protokollaversio (nyt 1), bitit 4–0: tilatunniste (0 = perustila) |
| 1 | hyötykuorman pituus L tavuina (0–160) |
| 2 | CRC-8 tavuista 0–1 |
| 3–11 | Reed–Solomon-pariteetti (9 tavua: korjaa 4 virheellistä tavua 12:sta) |

Otsake on koodattu raskaammin kuin data, koska sen menetys hukkaa koko
kehyksen. Tilatunniste kertoo, millä modulaatiolla hyötykuorma on lähetetty.
Tuntematonta versiota tai tilaa ei yritetä purkaa. Nopeammat tilat voivat
näin käyttää samaa alkumerkkiä ja otsaketta.

**Hyötykuorma** (tilan mukaisella modulaatiolla):

| tavut | sisältö |
|---|---|
| 0 … L-1 | linkkikerroksen kehys (ks. 5.1) |
| L, L+1 | CRC-16 tavuista 0…L-1 |
| loput | Reed–Solomon-pariteetti P tavua |

P = ceil((L + 2) / 2), vähintään 4, pyöristettynä ylös niin, että L + 2 + P on
neljällä jaollinen (symboli täyttyy; ylimääräinen pariteetti on ilmaista).
Koodisana on enintään 255 tavua, joten L on enintään 160. CRC-16 on mukana,
koska kapasiteetin ylittyessä Reed–Solomon voi harvoin "korjata" väärään
koodisanaan.

Kehyksen kesto: 90 ms (alkumerkki, tauko, häntä) + 150 ms (otsake) +
50 ms × (L + 2 + P) / 4.

## 5. Linkkikerros

Linkkikerros on tilakone, joka ei tiedä äänestä mitään. Sille annetaan
tapahtumia (kehys vastaanotettu, alkumerkki havaittu, oma lähetys loppui,
kello) ja se palauttaa toimia (lähetä kehys, näytä tila). Aika annetaan
ulkoa, joten testit ovat deterministisiä. Sama tilakone pyörii selaimessa,
CLI:ssä ja simulaattorissa.

### 5.1 Kehystyypit tavu tavulta

Kaikissa kehyksissä: tavu 0 = tyyppi (ylempi puolitavu) ja liput (alempi),
tavut 1–2 = viestin tunniste (satunnainen 16-bittinen, lähettäjä valitsee).

| tyyppi | nimi | suunta | sisältö tavusta 3 alkaen | L |
|---|---|---|---|---|
| 1 | START | A -> B | viestin pituus (2), kehysten määrä (1), kehyksen datakoko (1), viestin CRC-32 (4) | 11 |
| 2 | START_ACK | B -> A | signaalin laatu: SNR dB (1), liput (1: leikkautuu, hiljainen) | 5 |
| 3 | DATA | A -> B | järjestysnumero (1), dataa enintään 64 tavua. Lippu 0: ryöpyn viimeinen | 4–68 |
| 4 | ACK | B -> A | laatu (2), bittikartta saaduista kehyksistä (1–2). Lippu 0: aloita alusta | 6–7 |
| 5 | DONE | B -> A | viestin CRC-32 (4) | 7 |
| 6 | POLL | A -> B | ei sisältöä: "lähetä kuittauksesi uudelleen" | 3 |
| 7 | CANCEL | A -> B | ei sisältöä | 3 |

Lippu 3 (0x08) on varattu merkitsemään salattua sisältöä.

Viestin enimmäispituus on 512 tavua (8 datakehystä à 64 tavua). Tyhjä viesti
on sallittu: kehyksiä on 0 ja B vastaa aloitukseen suoraan DONE-kehyksellä.

### 5.2 Vuorosuuntaisuus ja oma kaiku

Kone kuulee oman kaiuttimensa. Vastaanotinta ei mykistetä lähetyksen ajaksi,
vaan se purkaa myös oman lähetyksen, ja linkkikerros käyttää sitä hyväksi:

- **Tunnistus:** vastaanotettu kehys, joka on tavu tavulta sama kuin viimeksi
  itse lähetetty, on oma kaiku. Roolit takaavat, ettei vastapuoli lähetä
  samaa: A lähettää vain START/DATA/POLL/CANCEL ja B vain START_ACK/ACK/DONE.
- **Ankkurointi:** oman kehyksen loppu mikrofonin aikajanalla on tarkka
  hetki, jolloin ääni oikeasti lakkasi. Vastauksen odotus alkaa siitä, joten
  selaimen tuntematon lähtö- ja tuloviive (jopa satoja millisekunteja) ei
  vaikuta aikakatkaisuun. Jos omaa kehystä ei kuulu (esim. kuulokkeet),
  ankkurina on lähetyksen loppu + 0,5 s.
- **Suoja-aika:** vastaaja odottaa 150 ms vastaanotetun kehyksen lopusta
  ennen kuin lähettää. Mitattu oma kaiku on kohinapohjassa 62 ms:ssa;
  kaikuisammassa huoneessa alkumerkin suhteellinen kynnys ja sävelhyppy
  sietävät loput.

### 5.3 Tilakone

```text
                 send()                 START_ACK                 kaikki kuitattu
  LEPO ────────────────► ALOITUS ────────────────► LÄHETYS ◄──┐
   ▲  ▲                  (START)                   (DATA-ryöppy)│ ACK: puuttuvia
   │  │                     │ 3 yritystä                │        │
   │  │                     ▼ ilman vastausta           ▼        │
   │  │              "ei vastaanottajaa"         ODOTUS ─────────┘
   │  │                                          (ACK/DONE)
   │  └──────────── DONE: "Perillä" ◄───────────────┘
   │
   │  START                       DATA...                    kaikki koossa, CRC-32 ok
  LEPO ────────► VASTAANOTTO ──────────────► VASTAANOTTO ──────────────► LEPO
        START_ACK              ACK ryöpyn                      DONE, näytä viesti
                               lopussa
```

Lähettäjän (A) tilat: LEPO, KANAVAN KUUNTELU, ALOITUS, LÄHETYS, ODOTUS.
Vastaanottajan (B) tilat: LEPO, VASTAANOTTO. Roolit ovat symmetriset: kumpi
tahansa voi aloittaa, ja roolit määräytyvät viestikohtaisesti.

### 5.4 Aikakatkaisut

Kaikki ajat on ankkuroitu havaitun signaalin päättymiseen (oman tai
vastapuolen), ei omaan kelloon lähetyshetkellä.

| nimi | arvo | ankkuri | tarkoitus |
|---|---|---|---|
| vastausviive | 150 ms | vastaanotetun kehyksen loppu | oman ja huoneen kaiun suoja-aika |
| vastauksen odotus | 1,5 s | oman kehyksen loppu mikrofonissa | kattaa vastausviiveen ja molempien laitteiden ääniviiveet |
| odotuksen jatko | kehyksen loppuun | alkumerkki havaittu odotuksen aikana | vastaus on tulossa, vaikka se alkoi myöhään |
| aloitusyritykset | 3 | | sitten "vastaanottajaa ei löytynyt" (noin 6 s) |
| POLL-yritykset | 3 | | kuittaus tai loppukuittaus katosi |
| uudelleenlähetyskierrokset | 6 | | sitten "siirto epäonnistui" syineen |
| vastaanoton aikakatkaisu | 6 s | viimeisin havaittu signaalin loppu | lähettäjä katosi: osittainen viesti hylätään |
| kanavan kuuntelu | 200 ms hiljaista | | ennen START-kehystä |
| satunnainen odotus | 0,3–1,2 s | | kanava varattu tai aloitus törmäsi |
| valmiiden viestien muisti | 8 tunnistetta, 60 s | | kaksoiskappaleiden tunnistus |

B lähettää kuittauksen, kun (a) se saa DATA-kehyksen, jossa on "ryöpyn
viimeinen" -lippu, tai (b) se on havainnut alkumerkkejä edellisen
kuittauksensa jälkeen ja kanava on ollut hiljaa 400 ms. Kohta (b) kattaa
tapauksen, jossa ryöpyn viimeinen kehys vioittui.

### 5.5 Normaali sessio

1. Molemmat koneet kuuntelevat (LEPO). Alkumerkin ilmaisin käy koko ajan.
2. A:n käyttäjä painaa Lähetä. Teksti on UTF-8-tavuja. A arpoo
   16-bittisen tunnisteen, laskee CRC-32:n ja jakaa viestin 64 tavun paloihin.
3. A kuuntelee kanavaa 200 ms. Jos alkumerkki on havaittu tai kehyksen
   vastaanotto on kesken, A odottaa satunnaisesti 0,3–1,2 s ja kuuntelee uudelleen.
4. A lähettää START-kehyksen ja kuulee sen itse (ankkuri).
5. B saa START-kehyksen, varaa muistin, näyttää "vastaanotetaan" ja
   lähettää 150 ms:n kuluttua START_ACK-kehyksen, jossa on sen mittaama
   signaalin laatu.
6. A saa START_ACK-kehyksen ja lähettää kaikki datakehykset peräkkäin
   (kehysten välissä 50 ms). Viimeisessä on "ryöpyn viimeinen" -lippu.
7. B tallentaa ehjät kehykset ja näyttää edistymisen. Ryöpyn lopussa:
   - kaikki koossa ja CRC-32 täsmää: B lähettää DONE-kehyksen, näyttää
     tekstin ja Kopioi-napin ja palaa lepoon
   - puuttuu: B lähettää ACK-kehyksen, jonka bittikartta kertoo saadut
8. A saa ACK-kehyksen ja lähettää vain puuttuvat (uusi kierros), tai saa
   DONE-kehyksen, vertaa CRC-32:n ja näyttää "Perillä".

### 5.6 Poikkeustilanteet

| tilanne | mitä tapahtuu |
|---|---|
| datakehys katoaa tai vioittuu | Kehyksen CRC-16 (ja Reed–Solomon) hylkää sen. B:n bittikartasta puuttuu kehys, ja A lähettää sen uudelleen seuraavalla kierroksella. Enintään 6 kierrosta. |
| ryöpyn viimeinen kehys katoaa | B ei näe lippua, mutta on havainnut alkumerkkejä ja kuittaa, kun kanava on ollut hiljaa 400 ms. |
| kuittaus (ACK) katoaa tai vioittuu | A:n vastauksen odotus (1,5 s) päättyy. A lähettää POLL-kehyksen ja B toistaa kuittauksensa. Kolmen vastaamattoman POLL-kehyksen jälkeen siirto epäonnistuu. |
| aloituksen kuittaus (START_ACK) katoaa | A lähettää START-kehyksen uudelleen. B tunnistaa tunnisteen ja vastaa uudelleen aloittamatta alusta. |
| loppukuittaus (DONE) katoaa | A lähettää POLL-kehyksen (tai START/DATA-kehyksen uudelleen). B löytää tunnisteen valmiiden viestien muistista, lähettää DONE-kehyksen uudelleen eikä näytä viestiä toista kertaa. |
| molemmat painavat Lähetä yhtä aikaa | Jos lähetykset menevät päällekkäin, kumpikaan ei saa START_ACK-kehystä. Molemmat odottavat satunnaisen ajan (0,3–1,2 s) ja kuuntelevat kanavaa, jolloin toinen ehtii ensin. Se, joka saa vieraan START-kehyksen omaa aloitustaan odottaessaan, siirtyy vastaanottajaksi ja lähettää oman viestinsä heti vastaanoton päätyttyä. Jos molemmat kuulevat toistensa START-kehyksen, pienempi tunniste pitää lähettäjän roolin. |
| lähettäjä katoaa kesken viestin | B ei havaitse session kehyksiä 6 sekuntiin viimeisestä signaalin lopusta. B hylkää osittaisen viestin, kertoo siitä käyttäjälle ja palaa lepoon. |
| käyttäjä peruuttaa lähetyksen | A lopettaa äänen heti, lähettää yhden CANCEL-kehyksen ja palaa lepoon. B hylkää osittaisen viestin. Jos CANCEL katoaa, B:n 6 s:n aikakatkaisu hoitaa asian. |
| vastaanottajaa ei ole | Kolme START-yritystä ilman vastausta, sitten "vastaanottajaa ei löytynyt" noin 6 sekunnissa. Dataa ei lähetetä. |
| voimakas häiriöääni kesken siirron | Osa kehyksistä vioittuu: Reed–Solomon korjaa lyhyen häiriön (kolahdus osuu 1–2 symboliin), pidempi (puhe) kaataa kehyksiä, jotka lähetetään uudelleen. Kuittaus voi myös kadota (POLL). Jos kierrokset loppuvat, A näyttää syyn B:n raportoiman laadun perusteella. |
| ääkköset, emoji | Sisältö on UTF-8-tavuja, joita protokolla ei tulkitse. Katkaisu paloihin saa osua merkin keskelle, koska viesti kootaan ennen tulkintaa. Pituusraja lasketaan tavuista, ja käyttöliittymä näyttää sen. |
| tyhjä viesti | START (pituus 0, 0 kehystä) -> DONE. |
| enimmäispituinen viesti | 512 tavua = 8 datakehystä yhdessä ryöpyssä. |
| vioittunut kehys läpäisee tarkistukset | Kehyksen CRC-16 ja koko viestin CRC-32 yhdessä: jos koottu viesti ei täsmää, B lähettää ACK-kehyksen "aloita alusta" -lipulla. |
| vieras viesti kesken oman session | Kehykset, joiden tunniste ei ole käynnissä olevan session, ohitetaan. |

Epäonnistumisen syy käyttäjälle johdetaan näin: ei yhtään vastausta ->
"vastaanottajaa ei löytynyt"; vastapuoli raportoi leikkautumista ->
"signaali leikkautuu, pienennä äänenvoimakkuutta"; raportoitu SNR matala ja
taso matala -> "signaali liian hiljainen, lisää äänenvoimakkuutta tai siirrä
koneet lähemmäs"; SNR matala ja taso normaali -> "liikaa taustamelua".

### 5.7 Ryöppy vai kehyskohtainen kuittaus

Jokainen suunnanvaihto maksaa vastausviiveen (150 ms), laitteiden ääniviiveet
(arvio 200 ms, mitataan kehässä D) ja kuittauskehyksen (390 ms): yhteensä
noin 0,75 s per kuittaus ja toinen 0,35 s paluusuuntaan.

| viesti | kehyskohtainen kuittaus | ryöppy + koontikuittaus |
|---|---|---|
| 60 tavua (1 datakehys) | sama | sama |
| 512 tavua (8 datakehystä) | noin 23 s | noin 15,5 s |

Yhden kehyksen viestillä tavat ovat identtiset. Pitkällä viestillä ryöppy on
kolmanneksen nopeampi, ja valikoiva uudelleenlähetys bittikartalla hoitaa
huonon kanavan. Ryöpyn koko on linkkikerroksen parametri (1 = kehyskohtainen
kuittaus), joten vaiheen 5 mittausajo vertaa molempia simuloiduilla
kanavilla ennen lopullista valintaa.

## 6. Aika-arviot

Kehysten kestot ehdotetuilla parametreilla (50 ms/symboli, 4 tavua/symboli):

| kehys | L | koodisana | symboleja | kesto |
|---|---|---|---|---|
| START | 11 | 20 tavua | 5 | 0,49 s |
| START_ACK | 5 | 12 | 3 | 0,39 s |
| DATA, 60 tavua | 64 | 100 | 25 | 1,49 s |
| DATA, 64 tavua | 68 | 108 | 27 | 1,59 s |
| ACK | 6 | 12 | 3 | 0,39 s |
| DONE | 7 | 16 | 4 | 0,44 s |

**60 merkin linkki, kaikki onnistuu ensimmäisellä kerralla:**

| vaihe | kesto |
|---|---|
| kanavan kuuntelu | 0,20 s |
| START | 0,49 s |
| suunnanvaihto (150 ms + ääniviiveet noin 200 ms) | 0,35 s |
| START_ACK | 0,39 s |
| suunnanvaihto | 0,35 s |
| DATA | 1,49 s |
| suunnanvaihto | 0,35 s |
| DONE | 0,44 s |
| **yhteensä** | **noin 4,1 s** |

Nettonopeus koko sessiolle on noin 15 tavua/s (v0: 10,8 tavua/s ilman
kuittauksia ja ilman toimivuutta kaiussa). Vaihteluväli ääniviiveiden mukaan
on arviolta 3,8–5 s.

**Enimmäispituinen viesti (512 tavua):** aloitus 1,8 s + 8 datakehystä
(8 × 1,64 s = 13,1 s) + suunnanvaihto ja DONE 0,8 s = **noin 15,5 s**.
Yksi uudelleenlähetyskierros yhdelle kehykselle lisää noin 2,7 s.

## 7. Laajennettavuus

Näitä ei toteuteta nyt, mutta rakenne ei sulje niitä pois:

- **Nopeampi tila:** otsakkeen tilatunniste (5 bittiä) valitsee hyötykuorman
  modulaation. START_ACK kertoo vastaanottajan signaalin laadun, ja A voi
  valita datakehyksille nopeamman tilan. Otsake ja kuittaukset pysyvät
  perustilassa.
- **Lähes kuulumaton tila (18–20 kHz):** mitattu vaste on siellä noin 12 dB
  heikompi mutta käyttökelpoinen. Tila tarvitsee oman alkumerkin (pyyhkäisy
  eri kaistalla), eli vastaanottimeen tulee toinen ilmaisin rinnalle.
  `PhyKind` ja sävelristikon parametrit (alataajuus, sävelväli, alikanavien
  määrä) on tehty tätä varten.
- **Nopeuden sovitus:** START_ACK- ja ACK-kehysten laatutavut ovat jo
  protokollassa.
- **Salaus:** kehyksen lippu 3 on varattu. Salaus koskisi DATA-kehysten
  sisältöä ja START-kehyksen CRC-32:ta, ei kehysrakennetta.
- **Versiointi:** otsakkeen kolme versiobittiä. Tuntematon versio ohitetaan
  hiljaa.

## 8. Tunnetut rajoitukset

- **Vain kaksi laitetta.** Jos kuuloetäisyydellä on useampi vastaanottaja,
  kaikki vastaavat START-kehykseen yhtä aikaa ja kuittaukset törmäävät.
  Lähettäjä näkee tämän vastaanottajan puuttumisena tai epäonnistuneena
  siirtona. Ratkaisu vaatisi vastaanottajan tunnisteen ja satunnaistetun
  vastausviiveen, eikä sitä tehdä ensimmäisessä versiossa.
- **Ei salausta eikä todennusta.** Kuka tahansa kuuloetäisyydellä voi
  vastaanottaa viestin tai lähettää oman. Käyttöliittymä kertoo tämän, ja
  vastaanotettu teksti näytetään aina pelkkänä tekstinä.
- **Tunnisteiden törmäys.** 16-bittinen tunniste voi osua samaksi kuin
  hiljattain vastaanotettu (1/65536 per viesti muistin 8 paikkaa kohden).
  Silloin B luulisi uutta viestiä kaksoiskappaleeksi. Muistissa verrataan
  siksi myös CRC-32:ta ja pituutta.
- **Liikkuvat laitteet.** 25 Hz:n sävelväli sietää Doppler-siirtymää vain
  vähän (kävelyvauhti 7 kHz:n taajuudella on noin 20 Hz). Laitteiden oletetaan
  olevan paikallaan.
- **Kuuluvuus.** Signaali on kuultava, laajakaistainen sirinä.

## 9. Mitä on mitattu ja mitä ei

| väite | peruste |
|---|---|
| kaistan valinta, melun spektri, särö, oman kaiun kesto | mitattu oikealla äänellä Macilla (yksi laite, yksi huone) |
| sävelhypyn hyöty, 4-FSK vs. 16-FSK, PSK:n heikkous kaiussa | simulaatio: synteettiset huoneet ja mitattu impulssivaste |
| toisen koneen kuulema kanava | **arvio** (suora ääni -14 dB), ei mittaus |
| laitteiden ääniviive (200 ms), suunnanvaihdon kesto | **oletus**, mitataan kehissä D ja E |
| kehysten kestot ja 4,1 s:n kokonaisaika | laskettu parametreista, ei vielä toteutettu |
| alkumerkin havaitseminen ja ajoitus | ei vielä mitattu (vertailukoe käytti ihanteellista ajoitusta) |
| Reed–Solomonin koodisuhde, tavujen sijoittelu | lähtöarvo, vaihe 4 mittaa |

**Huipputason leikkaus.** Vertailukoe toistettiin niin, että lähetteen
huiput leikattiin 6 dB tehollisarvon yläpuolelta (kaiuttimen rajallinen
huipputaso), jolloin PAPR putoaa 13 dB:stä 6 dB:iin
(`bench/kokeet/20261111-modulaatiovertailu-leikattu.txt`). Leikkaus ei
heikentänyt yhtäkään FSK-vaihtoehtoa mitattavasti: ehdotetun 16 × 4-FSK:n
virhesuhde kaikuisassa huoneessa oli 0,49 % (ilman leikkausta 0,47 %).
Lähetteen huiput voi siis rajata ja nostaa keskitehoa noin 7 dB, joten
rinnakkaisten sävelten tehohinta on käytännössä noin 5 dB eikä 12 dB.
Tämä on simulaatiotulos ihanteellisella leikkurilla; oikean kaiuttimen
käytös mitataan kehässä C.

## 10. Kysymykset sinulle

1. **Lyhyt viesti aloituskehyksessä?** Tehtävänanto ehdottaa erillistä
   kättelyä (START -> START_ACK -> DATA -> DONE), ja suunnitelma noudattaa
   sitä. Jos viesti mahtuu yhteen kehykseen, sen voisi lähettää
   START-kehyksen mukana, jolloin 60 merkin linkki vie noin 2,6 s eikä 4,1 s.
   Hinta: kun vastaanottajaa ei ole, epäonnistuminen selviää hitaammin (noin
   9 s eikä 6 s), ja laaturaporttia ei saada ennen dataa. Ehdotan, että teen
   tämän linkkikerroksen parametriksi ja valitsen oletuksen vaiheen 5
   mittausten perusteella. Sopiiko?
2. **Kaista 1–7,4 kHz.** Signaali kuulostaa noin 0,5–3 sekunnin sirinältä.
   Kapeampi kaista (esim. 1,5–4,7 kHz) puolittaisi nopeuden. Onko
   äänen luonteella väliä tässä vaiheessa?
3. **Enimmäispituus 512 tavua** (noin 15 s). Riittääkö, vai haluatko enemmän
   (1024 tavua veisi noin 30 s)?
4. **Onko jokin poikkeustilanne käsitelty toisin kuin haluaisit?** Erityisesti
   yhtäaikainen lähetys: häviäjän viesti lähtee automaattisesti vastaanoton
   jälkeen ilman uutta napinpainallusta.
