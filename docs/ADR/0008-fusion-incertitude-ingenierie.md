# ADR 0008 — Fusion des méthodes, incertitude et simplifications de l'ingénierie

**Statut :** accepté (session 2)

## Décisions
- **Deux étages.**
  - Aperçu interactif par la méthode d'ingénierie, en bandes d'octave.
  - Calcul « basses fréquences » GFPE en tiers d'octave 25–200 Hz, fusionné en énergie avec l'ingénierie
    (poids 1 → 0 entre 125 et 250 Hz).
- **Cache incrémental.**
  - Profils par azimut, puis atténuations en excès par (hauteurs des sources, condition, bandes).
  - Déplacer ou réorienter une enceinte ne refait que la sommation : ~80–110 ms au lieu de ~250 ms.
- **Origine de la grille polaire** : l'emplacement de la sono choisi à la création de la zone. Les
  enceintes s'en écartent de quelques dizaines de mètres au plus. Les profils (sol, diffraction) sont
  calculés depuis l'origine, la géométrie exacte n'entre que dans la somme des sources.
  Approximation : < 1 dB au-delà de ~10 fois l'écart.
- **Incertitude.**
  - 4 scénarios : prévision AROME ajustée par Monin-Obukhov, neutre, inversion forte, vent portant vers
    l'habitation la plus proche.
  - Cartes médiane et P90.
  - La méthode utilisée est affichée sous chaque carte et dans le rapport.
  - Précision annoncée : ±3 dB à moins de 500 m, ±5 à 10 dB à plusieurs km de nuit.
- **Condition de l'ingénierie par scénario** : homogène pour le neutre, favorable sinon.
- **Simplifications documentées** :
  - vue obstruée : combinaison sol + diffraction de CNOSSOS-EU, mais avec un seul plan moyen pour
    tout le trajet ;
  - bruit routier : modèle de source linéique par classe d'importance (pas de trafic réel) ;
  - façades : niveau en champ libre (sans les +3 dB de réflexion de façade) ;
  - bâti : rasterisé à max(2 m, R/2500).

## Comparaison avec NoiseModelling (faite)
- Voir `docs/VALIDATION-NOISEMODELLING.md` et le test `cargo test -p sm-core --test noisemodelling`.
- Écart de LAeq : ≤ 2,8 dB en homogène et ≤ 5,7 dB en favorable.
- Les écarts viennent de différences de méthode documentées :
  - cohérence des rayons ;
  - borne CNOSSOS sur le sol mou ;
  - rayons courbes traités par abaissement du relief.
- Deux corrections en sont issues :
  - moyenne de bande adaptée au nombre de lobes d'interférence ;
  - combinaison CNOSSOS de l'effet de sol et de la diffraction.

## Performances 8 threads (mesurées)
- GFPE : 24 à 36 s par scénario.
- Aperçu d'ingénierie : 600 ms au premier calcul.
