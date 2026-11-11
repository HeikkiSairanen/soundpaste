# Tehtävä: soundpaste, äänimodeemi selaimeen (Rust + WebAssembly)

Alkuperäinen tehtävänanto sellaisenaan (2026-11-11). Sivu julkaistaan osoitteeseen
https://sairanen.org/soundpaste.

## Tavoite
Rakenna staattinen verkkosivu, jolla kaksi vierekkäistä tietokonetta siirtää
lyhyttä tekstiä (linkki, salasana, koodinpätkä) toisilleen äänen avulla:
toinen kone soittaa kaiuttimesta, toinen kuuntelee mikrofonilla. Yhteys on
kaksisuuntainen: vastaanottaja kuittaa äänellä takaisin, että viesti tuli
perille ehjänä, ja lähettäjä näkee vahvistuksen. Palvelinta ei ole, kaikki
tapahtuu selaimessa. Sivu julkaistaan osoitteeseen sairanen.org/soundpaste.

Rakenna tämä kuin projekti, josta voi kasvaa jotain isompaa: selkeä
arkkitehtuuri, hyvä testikattavuus ja koodi, jonka päälle on helppo jatkaa.
Itse siirto-ongelma (modulaatio, synkronointi, virheenkorjaus, kuittaukset)
on työn ydin ja kiinnostavin osa. Haluan ymmärtää ratkaisut, joten perustele
suunnitteluvalinnat äläkä piilota niitä kirjastojen taakse.

Työtapa on iteratiivinen: ensin rakennetaan mittaristo ja karkea päästä
päähän toimiva versio, sitten modeemia parannetaan mitattavasti. Teen työtä
lähinnä Macilla, joten kehityssyklin pitää pyöriä pääosin sillä.

## Reunaehdot
- Ydin Rustilla, käännetään kohteeseen wasm32-unknown-unknown.
- Mahdollisimman vähän ulkoisia riippuvuuksia. Tavoite on nolla cratea
  ytimessä: toteuta itse FFT tai Goertzel, CRC, Reed-Solomon (GF(256)) ja
  WAV-luku ja -kirjoitus. Kehitystyökaluissa riippuvuus on helpommin
  hyväksyttävissä, mutta kysy silti ensin ja perustele.
- Ei wasm-bindgeniä, ei npm:ää, ei bundleria. Vie funktiot suoraan
  (`#[no_mangle] extern "C"`) ja kirjoita pieni käsintehty JS-liima, joka
  hoitaa vain selainrajapinnat (getUserMedia, AudioContext, AudioWorklet,
  leikepöytä).
- Lopputulos on kansio staattisia tiedostoja (index.html, .js, .wasm).
  Käytä suhteellisia polkuja, jotta sivu toimii alihakemistossa.
- Ensimmäinen versio on rajattu kahteen laitteeseen. Useamman
  vastaanottajan tilanne (päällekkäiset kuittaukset) saa jäädä
  ratkaisematta, mutta kirjaa se DESIGN.md:hen tunnettuna rajoituksena.

## Arkkitehtuuri
1. `modem-core` (Rust-kirjasto, ei selainriippuvuuksia), kahdessa kerroksessa:
   - fyysinen kerros: tavut -> f32-näytteet ja f32-näytteet -> tavut.
     Vastaanotin toimii virtaavasti, eli sille syötetään näytelohkoja sitä
     mukaa kuin niitä tulee.
   - linkkikerros: kehykset, kuittaukset ja uudelleenlähetykset
     tilakoneena, joka ei tiedä äänestä mitään ja jonka aika annetaan
     ulkoa, jotta sitä voi testata deterministisesti.
2. Natiivi CLI samasta ytimestä: wav-koodaus ja -dekoodaus, koko
   linkkikerroksen ajo oikealla äänellä sekä kehityssyklin mittauskomennot.
3. Web-osa: wasm-kääre ja minimaalinen käyttöliittymä. Sama tilakone
   pyörii selaimessa ja CLI:ssä.
4. Kehityspalvelin Macille (vain kehityskäyttöön, ei osa julkaistavaa
   sivua): tarjoilee sivun lähiverkkoon ja ottaa vastaan testiaseman
   lokit ja äänitteet.

## Normaali sessio
Tämä on lähtöehdotus siitä, miten tieto kulkee. Saat ehdottaa parempaa
DESIGN.md:ssä, mutta jokaiseen alla olevaan tilanteeseen pitää olla vastaus.

1. Molemmat koneet avaavat sivun ja painavat Aloita. Kumpikin on
   lepotilassa ja kuuntelee. Roolit ovat symmetriset: kumpi tahansa voi
   lähettää.
2. A syöttää tekstin ja painaa Lähetä. Teksti koodataan UTF-8-tavuiksi,
   viestille luodaan satunnainen tunniste ja koko viestin tarkistussumma,
   ja se jaetaan numeroituihin kehyksiin.
3. A kuuntelee ennen lähetystä, että kanava on vapaa. Jos ei ole, se
   odottaa satunnaisen ajan ja yrittää uudelleen.
4. A lähettää lyhyen aloituskehyksen: viestin tunniste, pituus ja kehysten
   määrä. B vastaa lyhyellä kuittauksella, jossa on vastaanottopään
   signaalin laatu. Jos vastausta ei tule muutaman yrityksen jälkeen, A
   lopettaa heti ja kertoo, ettei vastaanottajaa löytynyt.
5. A lähettää datakehykset. B näyttää edistymisen.
6. B kuittaa, mitkä kehykset tulivat ehjinä. A lähettää vain puuttuvat
   uudelleen. Tämä toistuu rajatun määrän kierroksia.
7. Kun kaikki kehykset ovat perillä, B kokoaa viestin, tarkistaa koko
   viestin tarkistussumman ja lähettää loppukuittauksen, jossa summa on
   mukana.
8. A näyttää "Perillä". B näyttää tekstin ja Kopioi-napin. Molemmat
   palaavat lepotilaan.

Huomioitavaa suunnittelussa:
- Jokainen suunnanvaihto maksaa alkumerkin ja kaiun suoja-ajan. Vertaa
  siksi kehyskohtaista kuittausta ja ryöppynä lähettämistä yhdellä
  koontikuittauksella, ja valitse mittausten perusteella.
- Yhteys on vuorosuuntainen (half-duplex). Kone kuulee oman kaiuttimensa,
  joten vastaanotin ei saa tulkita omaa lähetystä tai sen kaikua
  saapuvaksi viestiksi. Mitoita suoja-aika mitatun kaiun mukaan.
- Selaimen äänen tulo- ja lähtöviive vaihtelee ja voi olla satoja
  millisekunteja. Ankkuroi aikakatkaisut havaitun signaalin päättymiseen
  ja mitoita ne väljästi.
- Kuittauskehysten pitää olla lyhyitä ja vähintään yhtä kestäviä kuin
  datakehysten.
- Kehyksen otsakkeessa on protokollaversio, kehystyyppi ja tilatunniste,
  jotta nopeammat tilat ja salaus voidaan lisätä myöhemmin rikkomatta
  yhteensopivuutta.

Poikkeustilanteet, jotka pitää käsitellä ja testata:
- datakehys katoaa tai vioittuu
- kuittaus katoaa tai vioittuu
- loppukuittaus katoaa: A lähettää uudelleen, eikä B saa näyttää samaa
  viestiä kahdesti, vaan tunnistaa tunnisteen ja kuittaa uudelleen
- molemmat painavat Lähetä yhtä aikaa
- lähettäjä katoaa kesken viestin: B luopuu osittaisesta viestistä
  aikakatkaisulla ja palaa lepotilaan
- käyttäjä peruuttaa lähetyksen kesken
- vastaanottajaa ei ole lainkaan
- huoneessa on voimakas häiriöääni kesken siirron (puhe, kolahdus)
- sisältö: ääkköset, emoji, tyhjä viesti ja enimmäispituinen viesti

## Fyysinen kerros
Lopullinen ensimmäinen versio painottaa luotettavuutta, ei nopeutta:
- kuultava taajuusalue, MFSK tai vastaava yksinkertainen ja kestävä modulaatio
- alkumerkki (preamble) synkronointiin, kehys jossa pituus ja CRC
- Reed-Solomon-virheenkorjaus
- viestin enimmäispituus muutama sata tavua

Myöhempi vaihe (älä toteuta vielä, mutta älä sulje pois rakenteessa):
nopeampi tila, lähes kuulumaton tila noin 18–20 kHz:n alueella, nopeuden
sovitus vastaanottajan raportoiman signaalilaadun perusteella ja sisällön
salaus.

## Kehityssykli
Periaate: jokainen muutos on mitattavissa yhdellä komennolla, ja suurin osa
iteroinnista tapahtuu hiljaa ja pelkällä Macilla.

### Mittarit
Jokainen mittausajo tuottaa saman pistetaulun:
- päämittari: kauanko noin 60 merkin linkin siirto kestää kuittauksineen,
  ja kuinka suuri osa siirroista onnistuu
- kestävyys: alin signaali-kohinasuhde, jolla onnistumisosuus pysyy korkeana
- marginaali: bittivirhesuhde ennen virheenkorjausta ja korjattujen
  symbolien määrä, jotta nähdään, kuinka lähellä rajaa ollaan
- nettonopeus tavuina sekunnissa

Tulokset tallennetaan koneluettavana (esim. JSON) hakemistoon
`bench/results/` yhdessä git-commitin tunnisteen kanssa, ja erillinen
komento vertaa ajoa edelliseen perustasoon ja nostaa heikennykset esiin.

### Kehät nopeimmasta hitaimpaan
A. Simulaatio (sekunteja, hiljainen). `cargo test` ja mittausajo
   simuloidun kanavan läpi kiinteillä siemenluvuilla. Kanava mallintaa:
   taustakohinan eri signaali-kohinasuhteilla, vaimennuksen ja
   leikkautumisen, kaiun, näytteenottotaajuuksien eron (44,1 kHz vs
   48 kHz sekä pieni kellovirhe), satunnaisen alkuviiveen ja vaihtelevan
   viiveen suuntien välillä. Kaksisuuntaisessa sessiossa on kaksi
   modeemi-instanssia, kummallekin suunnalle oma kanava ja lisäksi oman
   lähetyksen vuoto omaan mikrofoniin. Jokaisella Normaali sessio -osion
   poikkeustilanteella on oma testinsä.

B. Äänitekorpus (sekunteja, hiljainen). Hakemisto oikeita äänitteitä,
   joilla on metatiedot (laite, äänenvoimakkuus, etäisyys,
   protokollaversio, odotettu sisältö). Korpuksessa on kahdenlaista
   aineistoa:
   - kanavamittaukset: pyyhkäisyt, kohina ja pelkkä huoneen taustaääni.
     Näistä johdetaan huoneen impulssivaste ja oikea taustamelu, jotka
     syötetään kehän A simulaatioon. Näin uutta modulaatiota voi kokeilla
     oikeaa huonetta vastaan soittamatta ääntä. Nämä eivät vanhene, kun
     modulaatio muuttuu.
   - modeemilähetysten äänitteet: vastaanottimen regressiotestejä niin
     kauan kuin aaltomuoto pysyy samana. Merkitse protokollaversio, jotta
     vanhentuneet tunnistetaan.
   Pidä osa korpuksesta ja mitatuista kanavista sivussa
   vertailuaineistona, jota ei käytetä virittämiseen.

C. Akustinen kehä Macilla (kymmeniä sekunteja, kuuluu ääntä). Yksi
   komento soittaa lähetyksen Macin kaiuttimesta, äänittää sen
   samanaikaisesti mikrofonilla, dekoodaa ja lisää äänitteen korpukseen.
   Toista usealla äänenvoimakkuudella.
   - Käytä toistoon ja äänitykseen järjestelmän valmiita
     komentorivityökaluja ja selvitä ensin, mitä Macissa on. Kysy ennen
     kuin asennat mitään. Mikrofonin käyttö voi vaatia minulta
     kertaluonteisen luvan päätteelle.
   - Kysy minulta ennen kuin soitat ääntä ensimmäisen kerran kussakin
     työrupeamassa, ja pidä äänenvoimakkuus maltillisena.

D. Selain samalla Macilla (minuutti). Macin CLI ja Macin selaimessa
   pyörivä sivu keskustelevat keskenään. Tämä testaa wasm-kääreen ja
   JS-liiman ilman toista konetta.

E. Kaksi konetta (minuutteja). Macin viereen tulee PC passiiviseksi
   testiasemaksi, johon ei tarvitse koskea käyttöönoton jälkeen:
   - Macin kehityspalvelin tarjoilee sivun lähiverkkoon.
   - PC:n sivu on testitilassa (URL-parametri) ja tarkistaa säännöllisesti,
     onko uusi versio saatavilla. Uusi wasm-moduuli vaihdetaan lennossa
     ilman sivun uudelleenlatausta, jotta mikrofoni ja ääni pysyvät
     päällä eikä uutta napinpainallusta tarvita.
   - PC:n sivu lähettää session lokit ja mikrofoniäänitteensä Macin
     kehityspalvelimelle, ja ne lisätään korpukseen.
   - Testitilat: vastaanottotila (normaali protokolla), kaikutila
     (lähettää vastaanottamansa viestin takaisin uutena viestinä) ja
     majakkatila (lähettää tunnettua testiviestiä tasaisin väliajoin).
   - Mikrofoni vaatii selaimessa turvallisen yhteyden, eikä lähiverkon
     http-osoite ole sellainen. Selvitä vaihtoehdot ja ehdota minulle
     yksinkertaisin kertaluonteinen käyttöönotto PC:lle.
   - Mikään tästä ei saa päätyä julkaistavaan versioon: tuotantosivu ei
     ota yhteyttä mihinkään.
   Julkaisusta osoitteeseen sairanen.org/soundpaste sovitaan erikseen,
   joten kysy siitä minulta.

### Parannussykli
1. Kirjaa hypoteesi: mitä muutetaan ja minkä mittarin odotetaan paranevan.
2. Tee muutos ja aja kehät A ja B.
3. Jos tulos on lupaava, aja kehä C ja myöhemmin D ja E.
4. Vertaa perustasoon. Kirjaa tulos ja johtopäätös, myös silloin kun
   kokeilu epäonnistui.
5. Jos muutos jää voimaan, päivitä perustaso ja tee commit.

Älä viritä parametreja vain yhden huoneen ja yhden laiteparin mukaan.
Raportoi rehellisesti, missä raja kulkee, sen sijaan että säätäisit testit
menemään läpi.

### Jatkuvuus työrupeamien välillä
- `EXPERIMENTS.md`: kokeilupäiväkirja, jossa on hypoteesi, muutos,
  mittaustulos ja johtopäätös jokaisesta kokeilusta.
- `CLAUDE.md`: kehityssyklin komennot, nykyinen perustaso ja seuraavat
  kokeilemisen arvoiset ideat, jotta uusi istunto pääsee heti jatkamaan.

## DESIGN.md
Kirjoita `DESIGN.md` vasta, kun karkea ensimmäinen versio ja kanavan
mittaukset ovat olemassa (ks. Työjärjestys), jotta valinnat perustuvat
mitattuun dataan. Siinä
- vertaat lyhyesti modulaatiovaihtoehtoja (esim. MFSK, PSK, OFDM,
  chirp-pohjainen synkronointi) ja perustelet valinnan mittauksilla
- kuvaat kehysrakenteen tavu tavulta
- kuvaat linkkikerroksen tilakoneen ja aikakatkaisut sekä käyt läpi
  normaalin session ja jokaisen poikkeustilanteen
- arvioit, kauan tyypillisen linkin ja enimmäispituisen viestin siirto
  kestää kuittauksineen

Näytä se minulle ja odota kommenttini ennen kuin jatkat varsinaiseen
modeemiin.

## Selainpuolen sudenkuopat
- Pyydä mikrofoni niin, että echoCancellation, noiseSuppression ja
  autoGainControl ovat pois päältä. Ne on tehty puheelle ja rikkovat
  modeemisignaalin.
- Älä oleta näytteenottotaajuutta, vaan lue se AudioContextista.
- Äänen toisto ja mikrofoni vaativat käyttäjän napinpainalluksen ja
  HTTPS:n (tai localhostin).
- Mikrofonin luku ja signaalinkäsittely AudioWorkletissa. Selvitä, miten
  taustalle jäänyt välilehti vaikuttaa, ja varoita käyttäjää tarvittaessa.
- Leikepöydälle kirjoittaminen vaatii käyttäjän toiminnon, joten kopiointi
  tapahtuu napista.
- Vastaanotettu teksti on vierasta syötettä, koska kuka tahansa
  kuuloetäisyydellä voi lähettää. Näytä se aina pelkkänä tekstinä
  (textContent), älä koskaan HTML:nä, äläkä avaa linkkejä automaattisesti.

## Käyttöliittymä
Suomenkielinen ja pelkistetty.
- Aloita-nappi ottaa mikrofonin ja äänen käyttöön molemmilla koneilla.
- Tekstikenttä ja Lähetä-nappi. Näytä ennen lähetystä arvio siirron
  kestosta ja estä liian pitkä viesti selkeällä ilmoituksella.
- Lähettäjälle siirron tila: etsitään vastaanottajaa, lähetetään (kehys
  n/m), odotetaan kuittausta, yritetään uudelleen, perillä tai
  epäonnistui. Peruuta-nappi lähetyksen ajaksi.
- Vastaanottajalle edistyminen, valmis teksti ja Kopioi-nappi.
- Epäonnistuessa kerro syy ja käytännön neuvo: vastaanottajaa ei
  löytynyt, signaali liian hiljainen, signaali leikkautuu tai liikaa
  taustamelua.
- Jatkuva signaalitason näyttö, josta näkee, kuuleeko kone ylipäätään
  mitään.
- Huomautus, että siirto ei ole salattu ja että lähellä olevat laitteet
  voivat vastaanottaa sen.
- Vianetsintänäkymä: session tapahtumaloki aikaleimoineen ja mahdollisuus
  ladata viimeisimmän session mikrofoniäänite WAV-tiedostona, jotta
  epäonnistuneen siirron voi lisätä korpukseen ja toistaa CLI:llä.
- Ei analytiikkaa, ei ulkoisia fontteja eikä CDN-latauksia.

## Työjärjestys
1. Karkea päästä päähän -versio ja mittaristo: yksinkertaisin mahdollinen
   yksisuuntainen modulaatio ilman virheenkorjausta, CLI, simuloitu
   kanava, pistetaulu ja tulosten tallennus. Tavoite on saada kehä A
   pyörimään, ei hyvä modeemi.
2. Akustinen kehä Macilla: toisto ja äänitys, kanavan mittaus, korpuksen
   alku ja mitatun kanavan syöttö simulaatioon (kehät B ja C).
3. DESIGN.md mittausten pohjalta, sitten pysähdys kommenttejani varten.
4. Varsinainen fyysinen kerros parannussyklillä.
5. Linkkikerros: sessio, kuittaukset ja poikkeustilanteet testeineen.
6. wasm-kääre, verkkosivu ja testitilat (kehä D).
7. Kehityspalvelin ja PC-testiasema (kehä E).
8. README: käännösohjeet, kehityssyklin käyttö ja julkaisu.

Kerro jokaisen vaiheen lopuksi pistetaulu ja se, mikä on testattu vain
simulaatiossa, mikä oikealla äänellä Macilla ja mikä kahden koneen välillä.
