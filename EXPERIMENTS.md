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
