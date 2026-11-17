# ADR 0007 — Implémentation du GFPE

**Statut :** accepté (session 2)

## Décisions
1. **Formulation à pôle soustrait.** On n'implémente pas le terme d'onde de surface séparément. Le
   spectre réfléchi est régularisé : Ψ·P + 2β[Ψ·P(kz) − Ψ·P(β)]/(kz − β) (idée de Salomons, JASA 2015,
   EL8). La forme « originale » (terme 2iβΨ(β)… ajouté) était instable pour les petits pas, et dérivait
   de 0,1 à 0,2 dB par pas sur sol mou.
2. **Règle du point milieu** (z et kz décalés d'un demi-pas). Sans elle :
   - le point z = 0 est compté deux fois par le terme image (gain parasite à chaque pas) ;
   - l'échantillon kz = 0 vaut Rp = −1 même sur sol quasi rigide, ce qui fausse la composante rasante,
     pourtant dominante.
3. **Champ de départ analytique** (onde directe + Weyl-van der Pol) à r₀ = max(3λ, 10 m) au lieu des
   starters gaussien et de Salomons. Ceux-ci n'utilisent qu'un coefficient de réflexion en incidence
   normale (~1 dB d'erreur sur sol mou). La directivité complexe du réseau de subs selon le site
   pondère ce champ. Dans les creux de directivité (|AF(0)| < 25 % de Σ|a|), le rapport AF(φ)/AF(0) est
   mal conditionné : on prend alors un starter omnidirectionnel, et le rapport est plafonné à +12 dB.
4. **Relief : rotation de phase aux changements de pente** (coordonnées qui suivent le terrain), et non
   GTPE ou transformation conforme. C'est simple et suffisant pour des pentes modérées (< 15–20°). Pour
   les falaises, la méthode d'ingénierie (diffraction) reste la référence ; c'est une limite connue.
5. **Pas** : Δz = λ/10, Δr = 10 λ (bornes 8 λ … 60 m). Domaine de 250 m + couche absorbante de
   max(30 λ, 100 m).
6. **Profils polaires depuis la sono** (360 azimuts), 2 fréquences par tiers d'octave 25–200 Hz. Les
   positions des enceintes entrent par la somme cohérente exacte au récepteur.

## Validation (tests automatiques, `cargo test -p sm-core gfpe`)
- Sol rigide : ±0,05 dB face à l'analytique jusqu'à 1 km.
- Prairie σ = 200 kPa·s/m² : écart max 0,7 dB jusqu'à 1,5 km à 50, 100 et 160 Hz, indépendant du pas.
- Benchmarks qualitatifs de Salomons (2001) : réfraction vers le bas > homogène + 5 dB ; vers le haut
  (zone d'ombre) < homogène − 10 dB à 2 km.

## Performances (28 threads)
- 200 Hz sur 5 km : 93 ms.
- Une passe complète (360 az × 10 bandes × 2 fréquences) : environ 11 s par scénario en release
  (CLI) ; dans l'app de dev, 67 s pour les 4 scénarios.
- Mesuré avec 8 threads (`RAYON_NUM_THREADS=8`, release) : 24 à 36 s par scénario selon la météo
  (cible < 60 s). Aperçu d'ingénierie : 600 ms.
