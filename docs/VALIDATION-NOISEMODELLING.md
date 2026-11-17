# Validation de la méthode d'ingénierie face à NoiseModelling

Critère de sortie de la phase 2 : comparer la méthode d'ingénierie de SoundMap à
[NoiseModelling](https://github.com/Universite-Gustave-Eiffel/NoiseModelling) 6.0.0
(Université Gustave Eiffel, CNOSSOS-EU, directive 2015/996) sur des scènes communes.

## Scènes
Sol plat, 15 °C, 70 % HR, 101,325 kPa. Source ponctuelle omnidirectionnelle à 2 m,
Lw = 100 dB par octave (63 Hz–8 kHz). Récepteurs à 4 m sur un axe.

| Scène | Sol | Obstacle | Distances |
|---|---|---|---|
| `flat_soft` | G = 1 (SoundMap : classe D, σ = 200 kPa·s/m²) | aucun | 25 – 800 m |
| `flat_hard` | G = 0 (SoundMap : classe H) | aucun | 25 – 800 m |
| `barrier` | G = 1 | bâtiment 10 m de haut, 20 m d'épaisseur, à 80–100 m | 150 – 800 m |

Chaque scène est calculée en conditions homogènes (H) et favorables (F). Côté
NoiseModelling, ce sont 100 % d'occurrences, sans réflexion, avec diffraction sur
les arêtes horizontales.

## Reproduire
```bash
# NoiseModelling 6.0.0 + Java 21 (portable)
NM_HOME=/chemin/NoiseModelling_6.0.0 JAVA_HOME=/chemin/jdk-21 ./tests/noisemodelling/run.sh
cargo run --release -p sm-core --example compare_nm      # tableaux ci-dessous
cargo test -p sm-core --test noisemodelling               # non-régression (CSV versionnés)
```

## Résultats
Pire écart sur le LAeq : **2,8 dB en conditions homogènes** et **5,7 dB en
conditions favorables**. Dans les deux cas, l'écart reste dans la tolérance du
test : ±3 dB en homogène, ±6 dB en favorable.

## Écarts par bande et leur origine
1. **Sol mou, 63 Hz : +4 à +5 dB.** CNOSSOS borne l'effet de sol à
   Aground ≥ −3(1 − G), soit aucun renforcement sur un sol G = 1. En réalité,
   à 63 Hz l'herbe est presque réfléchissante. Notre formule de Weyl-van der Pol,
   vérifiée par le GFPE (±0,7 dB), donne le renforcement attendu.
2. **Creux d'interférence de sol : −4 à −8 dB vers 250 Hz sur sol mou, un creux
   par bande sur sol dur.** Nous sommons les deux rayons de manière cohérente,
   alors que CNOSSOS utilise une forme énergétique lissée (+3 dB constant sur sol
   dur). Le creux est physique pour une source compacte. La turbulence l'atténue
   en pratique, ce que nous ne modélisons pas (voir les pistes).
3. **Sol dur, conditions favorables, 400–800 m : −4 à −5 dB(A).** NoiseModelling
   donne +6,3 dB à 400 m et +7,6 dB à 800 m, identiques sur toutes les bandes.
   Cela dépasse le doublement de pression (+6 dB), plafond physique d'une seule
   réflexion. C'est un effet de la formulation CNOSSOS en conditions favorables,
   pas de la physique.
4. **Écran en conditions favorables : +3 à +6 dB(A) chez nous.** CNOSSOS calcule
   δF avec des longueurs d'arc entre les points réels. Nous ramenons les rayons
   courbes à des droites en abaissant le terrain de x(d−x)/2Rc, comme dans
   Nord2000 et Harmonoise : la hauteur de l'écran au-dessus du rayon courbe est
   conservée. L'écart va dans le sens prudent (gêne surestimée).

## Corrections apportées grâce à la comparaison
- **Repliement des interférences de sol.** La moyenne de bande utilisait 3 fréquences
  ponctuelles : sur sol dur, des creux de −8 dB apparaissaient à une octave différente à
  chaque doublement de distance.
  - Chaque sous-bande est maintenant moyennée analytiquement :
    E = 1 + |Qρ|² + 2·Re(Qρ·e^(iφm))·sinc(Δφ/2).
  - Le coût reste fixe et la moyenne tend vers la somme en énergie quand plusieurs lobes
    tombent dans la bande.
  - Test : `wide_band_on_hard_ground_tends_to_energy_sum`.
  - Mesures, machine chargée en arrière-plan : 19 µs par trajet, contre 18,6 µs pour
    l'ancienne formule dans les mêmes conditions. Aperçu 5 km : 290–305 ms.
- **Sol + diffraction.** La moyenne simple des effets de sol de part et d'autre
  de l'arête est remplacée par la combinaison CNOSSOS-EU :
  Δground(S,O) = 20·log10(1 + (10^(g/20) − 1)·10^(−(Δdif(S′,R) − Δdif(S,R))/20)),
  avec la source image S′ et le récepteur image R′ symétriques par rapport au
  plan moyen.

## Tableaux détaillés (SoundMap − NoiseModelling, dB)

### flat_soft — H (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 | +4.6 | +1.2 | -3.7 | +2.3 | +0.3 | +0.8 | +0.6 | +0.9 | 67.4 | 68.2 | +0.8 |
| 50 | +5.1 | +2.9 | -3.4 | +0.6 | +0.7 | +1.8 | +1.1 | +1.7 | 61.0 | 62.1 | +1.1 |
| 100 | +5.1 | +2.8 | -3.9 | -0.9 | +3.7 | +0.3 | +2.9 | +1.7 | 54.1 | 56.1 | +2.0 |
| 200 | +5.0 | +2.0 | -4.8 | +0.5 | +0.9 | +4.9 | +0.5 | +3.5 | 46.5 | 49.3 | +2.8 |
| 400 | +4.8 | +0.4 | -6.2 | +1.0 | +0.1 | +2.1 | +5.3 | +0.6 | 37.0 | 39.1 | +2.1 |
| 800 | +4.3 | -2.3 | -7.6 | +1.1 | +0.5 | +0.1 | +2.5 | +5.5 | 24.2 | 24.4 | +0.1 |

### flat_soft — F (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 | +4.6 | +1.0 | -3.2 | +2.2 | +0.7 | +0.5 | +0.8 | +0.8 | 67.4 | 68.2 | +0.8 |
| 50 | +5.0 | +2.6 | -4.2 | +1.4 | +0.3 | +1.5 | +1.4 | +1.6 | 61.0 | 62.1 | +1.1 |
| 100 | +4.8 | +2.1 | -5.0 | +1.1 | +1.7 | +1.4 | +2.2 | +2.2 | 54.3 | 55.8 | +1.5 |
| 200 | +4.4 | +0.4 | -4.4 | +3.0 | +2.7 | +1.8 | +2.1 | +2.3 | 47.0 | 49.2 | +2.2 |
| 400 | +3.6 | -2.4 | -3.2 | +2.8 | +2.7 | +2.1 | +2.4 | +2.5 | 39.7 | 42.0 | +2.3 |
| 800 | +2.0 | -6.6 | -1.6 | +4.2 | +0.4 | +3.3 | +2.0 | +2.7 | 31.7 | 33.9 | +2.2 |

### flat_hard — H (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 | +2.3 | +0.2 | -7.6 | +1.3 | -1.2 | +0.3 | -0.2 | -0.3 | 70.4 | 70.3 | -0.2 |
| 50 | +2.8 | +2.3 | +0.2 | -7.6 | +1.4 | -1.2 | +0.4 | -0.4 | 64.0 | 63.7 | -0.3 |
| 100 | +3.0 | +2.8 | +2.3 | +0.1 | -7.6 | +1.3 | -1.4 | +0.0 | 57.3 | 56.7 | -0.6 |
| 200 | +3.0 | +3.0 | +2.8 | +2.2 | -0.2 | -7.8 | +0.9 | -2.0 | 50.3 | 49.8 | -0.5 |
| 400 | +3.0 | +3.0 | +2.9 | +2.7 | +1.9 | -1.2 | -7.1 | -0.2 | 42.8 | 44.0 | +1.2 |
| 800 | +3.0 | +3.0 | +3.0 | +2.8 | +2.3 | +0.6 | -5.0 | -3.1 | 34.7 | 36.8 | +2.1 |

### flat_hard — F (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 | +2.2 | +0.0 | -7.6 | +1.3 | -1.2 | +0.4 | -0.4 | -0.1 | 70.4 | 70.2 | -0.2 |
| 50 | +2.8 | +2.1 | -0.8 | -5.9 | +1.0 | +0.0 | -0.3 | -0.3 | 64.0 | 63.8 | -0.2 |
| 100 | +2.9 | +2.5 | +1.0 | -5.8 | +0.7 | -0.3 | -0.7 | -0.5 | 57.3 | 56.9 | -0.4 |
| 200 | +2.3 | +2.0 | +0.7 | -4.9 | -0.9 | -0.7 | -0.9 | -1.2 | 50.9 | 49.7 | -1.2 |
| 400 | -0.4 | -0.7 | -2.0 | -8.0 | -3.5 | -3.5 | -3.8 | -4.1 | 46.1 | 42.2 | -3.9 |
| 800 | -1.8 | -2.3 | -4.5 | -12.3 | -3.4 | -6.2 | -4.7 | -5.8 | 39.3 | 34.1 | -5.3 |

### barrier — H (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150 | +4.3 | +0.7 | -2.2 | +3.1 | +0.9 | +1.4 | +1.6 | +2.1 | 27.3 | 28.7 | +1.4 |
| 200 | +6.7 | -0.5 | -2.7 | +2.3 | +1.0 | +1.8 | +2.4 | +2.9 | 25.3 | 26.6 | +1.3 |
| 300 | +7.3 | +0.9 | -5.8 | +4.1 | +0.9 | +1.8 | +3.3 | +3.5 | 21.8 | 23.7 | +1.9 |
| 500 | +7.3 | -0.2 | -5.9 | +2.8 | +4.0 | +2.3 | +2.8 | +4.2 | 16.5 | 19.1 | +2.6 |
| 800 | +6.9 | -2.4 | -5.0 | +1.5 | +3.8 | +4.5 | +3.6 | +4.0 | 10.6 | 13.1 | +2.5 |

### barrier — F (écart SoundMap − NoiseModelling, dB)

| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150 | +3.0 | +2.2 | +4.0 | +3.8 | +3.7 | +2.7 | +1.4 | +2.0 | 27.6 | 30.9 | +3.2 |
| 200 | +5.7 | -1.3 | +5.8 | +4.2 | +4.4 | +5.0 | +2.9 | +2.7 | 25.9 | 30.3 | +4.4 |
| 300 | +5.3 | -1.8 | +4.5 | +4.2 | +5.3 | +5.5 | +5.8 | +6.2 | 27.1 | 31.8 | +4.7 |
| 500 | +2.5 | +0.3 | +3.7 | +6.1 | +5.6 | +6.2 | +6.6 | +7.0 | 22.5 | 27.8 | +5.3 |
| 800 | +3.2 | -1.6 | +4.5 | +6.5 | +6.0 | +6.6 | +7.0 | +7.3 | 17.8 | 23.4 | +5.7 |
