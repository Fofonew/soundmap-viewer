# SoundMap — Équations et références

Toutes les équations implémentées, avec leur référence, le code et les tests qui les vérifient.
Convention temporelle **e^(−iωt)** partout (onde sortante e^(ikr)), comme Salomons (2001).

---

## 1. Bandes de fréquence (IEC 61260-1:2014, base 10)

Tiers d'octave d'indice n (relatif à 1 kHz) : `f_m = 1000·10^(n/10)`, n ∈ [−16, 9] (25 Hz … 8 kHz).
Octaves : n multiple de 3 (31,5 Hz … 8 kHz). Bornes : `f_m·10^(∓b/20)` (b = largeur en tiers).

Code : `sm-core/src/bands.rs`. Tests : nombre de bandes, nominaux, rapport des bornes.

## 2. Pondérations A et C (IEC 61672-1:2013, annexe E)

f₁ = 20,598997 ; f₂ = 107,65265 ; f₃ = 737,86223 ; f₄ = 12194,217 Hz.

    R_A = f₄² f⁴ / [(f² + f₁²)·√((f² + f₂²)(f² + f₃²))·(f² + f₄²)],   A = 20 lg R_A + 2,000
    R_C = f₄² f² / [(f² + f₁²)(f² + f₄²)],                            C = 20 lg R_C + 0,062

Code : `weighting.rs`. Tests : ≤ 0,05 dB des valeurs tabulées (26 tiers d'octave).

## 3. Atmosphère

### 3.1 Absorption (ISO 9613-1:1993, éq. 3–5)

    h = RH·(p_sat/p_r)/(p_a/p_r),  p_sat/p_r = 10^(−6,8346 (T01/T)^1,261 + 4,6151)
    f_rO = (p_a/p_r)·(24 + 4,04·10⁴ h (0,02 + h)/(0,391 + h))
    f_rN = (p_a/p_r)(T/T0)^(−1/2)·(9 + 280 h·exp(−4,170((T/T0)^(−1/3) − 1)))
    α = 8,686 f² [1,84·10⁻¹¹ (p_r/p_a)(T/T0)^(1/2)
         + (T/T0)^(−5/2)(0,01275 e^(−2239,1/T)/(f_rO + f²/f_rO) + 0,1068 e^(−3352/T)/(f_rN + f²/f_rN))]  dB/m

Code : `atmosphere.rs`. Tests : tableau 2 d'ISO 9613-2 (6 conditions × 8 octaves, ±3 %).

### 3.2 Célérité et célérité effective

    c(T) = 331,3·√(1 + T/273,15)
    c_eff(z, θ) = c(T(z)) + u(z)·cos(θ_vent→ − θ_propagation)

### 3.3 Monin-Obukhov (Businger-Dyer)

    u(z) = (u*/κ)[ln(z/z₀) − ψ_m(z/L)],   T(z) = T_ref + (T*/κ)[ln(z/z_ref) − ψ_h(z/L) + ψ_h(z_ref/L)] − Γ_d (z − z_ref)
    stable (ζ ≥ 0) : ψ_m = ψ_h = −5ζ
    instable : x = (1 − 16ζ)^¼, ψ_m = 2 ln((1+x)/2) + ln((1+x²)/2) − 2 atan x + π/2, ψ_h = 2 ln((1+x²)/2)
    κ = 0,4, Γ_d = 0,0098 K/m, z₀ = 0,05 m

**Ajustement sur AROME** (T à 2/20/50/100/150 m, vent à 10/20/50/100/150 m) : recherche de 1/L sur une
grille, u* et (T_ref, T*) par moindres carrés à L fixé. Le coût combine les résidus (σ_u = 0,3 m/s,
σ_T = 0,1 K) et la cohérence physique 1/L = κ g T*/(u*² T). Au-dessus de 150 m, on extrapole à gradient
constant. Direction du vent interpolée en hauteur.

**Repli Pasquill** (Turner simplifié : vent à 10 m, nébulosité, jour/nuit) puis Golder (1972) :
`1/L = a + b·lg z₀` (A : −0,096/0,029 … F : 0,035/−0,036).

Code : `atmosphere.rs`, `sm-compute/src/meteo.rs`. Tests : détection d'une inversion nocturne,
reproduction des observations (±0,5 K, ±1 m/s), classes de nuit, profil homogène constant.

## 4. Sol

### 4.1 Impédance (milieu poreux à réaction locale), σ en kPa·s·m⁻²

    Delany-Bazley : Z = 1 + 9,08 (f/σ)^−0,75 + i·11,9 (f/σ)^−0,73
    Miki          : Z = 1 + 5,50 (f/σ)^−0,632 + i·8,43 (f/σ)^−0,632

Classes Nordtest (NT ACOU 104) : A 12,5 · B 31,5 · C 80 · D 200 · E 500 · F 2000 · G 20 000 · H 200 000.
Correspondance OCS GE → classe : ADR 0006 et `scene::ocsge_class`.

### 4.2 Réflexion sphérique (Weyl-van der Pol)

    Rp = (Z sinψ − 1)/(Z sinψ + 1)
    w  = ½(1 + i)·√(k R₂)·(sinψ + 1/Z)
    F(w) = 1 + i√π·w·w_F(w),   w_F(z) = e^(−z²) erfc(−iz)   (fonction de Faddeeva)
    Q  = Rp + (1 − Rp)·F(w)
    p/p_direct = 1 + Q·(R₁/R₂)·e^(ik(R₂ − R₁))

Faddeeva : algorithme W4 de Humlíček (1982), w(z) = 2e^(−z²) − w(−z) pour Im z < 0.
Moyenne énergétique sur n fréquences géométriques dans chaque bande (lisse les interférences).

Code : `faddeeva.rs`, `ground.rs`. Tests : valeurs de référence et série entière de w(z) ; sol rigide
(Q = 1, +6 dB en basses fréquences) ; décroissance de 6 dB par doublement en incidence rasante ;
limite onde plane ; creux d'interférence sur prairie.

## 5. Diffraction

Chemin élastique : enveloppe convexe supérieure du profil (sol + bâti) entre S et R (chaîne monotone
d'Andrew). δ = longueur du chemin − |SR| ; δ < 0 (marge) si la vue est dégagée.

    CNOSSOS-EU : Δdif = 10·lg(3 + 40/λ·C''·δ) si 40δ/λ·C'' ≥ −2, plafonné à 25 dB
    C'' = (1 + (5λ/e)²)/(1/3 + (5λ/e)²)  (e = distance entre la première et la dernière arête)
    Maekawa (validation) : A = 10·lg(3 + 20N), N = 2δ/λ

Code : `diffraction.rs`. Tests : points de Maekawa (4,8 / 13,6 / 23,1 dB), arête unique = Maekawa,
écran mince, marge négative, double arête.

## 6. Feuillage (ISO 9613-2, annexe A)

10 ≤ d_f < 20 m : forfait (0/0/1/1/1/1/2/3 dB de 63 Hz à 8 kHz) ; 20 ≤ d_f ≤ 200 m :
0,02/0,03/0,04/0,05/0,06/0,08/0,09/0,12 dB/m. d_f = longueur du rayon sous la canopée (MNH ≥ 3 m).

## 7. Méthode d'ingénierie (`engineering.rs`)

    L_bande = 10 lg(E_subs·10^(ΔL_subs/10) + E_autres·10^(ΔL_autres/10)) − α·r
    ΔL = effet de sol + (− Δdif) − A_feuillage

- **Vue dégagée** : effet de sol à deux rayons sur le **plan moyen** (moindres carrés du profil), σ
  moyen géométrique sur 10–90 % du trajet.
- **Moyenne de bande** : `n_sub` sous-bandes géométriques. Q est pris au centre fm de chaque
  sous-bande, et le terme croisé est moyenné analytiquement sur la phase φ = 2πf(R₂ − R₁)/c :
  E = 1 + |Qρ|² + 2·Re(Qρ·e^(iφm))·sinc(Δφ/2), avec ρ = R₁/R₂. Pas de creux replié ; la somme en
  énergie est atteinte quand plusieurs lobes tombent dans la bande.
- **Vue obstruée** (combinaison CNOSSOS-EU) :
  −Δdif(S,R) + Δground(S,O) + Δground(O,R), avec
  Δground(S,O) = 20 lg(1 + (10^(G₁/20) − 1)·10^(−(Δdif(S′,R) − Δdif(S,R))/20)).
  - G₁ : gain de sol (deux rayons) de la source à la première arête, borné à [−15, +6] dB.
  - S′ : source image par rapport au plan moyen ; symétriquement R′ pour le côté récepteur.
  - Simplification : un seul plan moyen pour tout le trajet.
- **Validation** : face à NoiseModelling 6.0.0, écart de LAeq ≤ 2,8 dB en homogène et ≤ 5,7 dB en
  favorable. Écarts expliqués dans `docs/VALIDATION-NOISEMODELLING.md`.
- **Conditions favorables** : rayons courbes Rc = max(1000 m, 8d), traités comme un abaissement du
  relief z' = z − x(d − x)/(2Rc).

Tests : sol plan = deux rayons, colline (HF plus atténuées), favorable < homogène sur l'écran,
bâtiment épais (2 arêtes), forêt de 200 m (+24 dB à 8 kHz, +4 dB à 63 Hz).

## 8. Sources (`source.rs`)

- Niveau émis dans une bande : `L = SPL_max,oct(1 m) + R(f) + P_programme(f) + G + 10 lg(b/3)`,
  avec R(f) la pente hors bande passante (24 dB/oct) et P le spectre du programme (tekno, techno,
  DnB, rock, rose).
- Directivité :
  - subs : monopôles ;
  - pavillon : −6(θh²/(cov_h/2)² + θv²/(cov_v/2)²) dB au-dessus de f_c, piston 2J₁(ka sinθ)/(ka sinθ) en dessous ;
  - ligne verticale : |sin(Nπs sinφ/λ)/(N sin(πs sinφ/λ))|.
- **Subs en cohérence** :
  `p = Σ aᵢ·Dᵢ·polᵢ·e^(i(k rᵢ + ω τᵢ))/rᵢ`, moyenné en énergie sur 3 à 5 fréquences par bande.
- **Têtes en énergie** : `E = Σ |aᵢ Dᵢ|²/rᵢ²`.

Tests : loi en 1/r², deux subs cohérents +6 dB, réjection arrière d'un cardioïde > 15 dB, pavillon
−6 dB en bord de couverture, line array plus directive en hautes fréquences, Bessel J₁.

## 9. GFPE (`gfpe.rs`, ADR 0007)

    ψ(r+Δr, z) = e^(iΔr δk(z)) · { (1/2π)∫[Ψ(kz) + Rp(kz)Ψ(−kz)]·P(kz)·e^(ikz z) dkz + 2iβ Ψ(β) P(β) e^(−iβz) }
    Ψ(kz) = ∫₀^∞ ψ e^(−ikz z) dz,   P(kz) = e^(iΔr(√(ka² − kz²) − ka)),   Rp(kz) = (kz Z − ka)/(kz Z + ka),   β = ka/Z

Implémentation :
- **Pôle soustrait** : le spectre réfléchi s'écrit Ψ·P + 2β[Ψ·P(kz) − Ψ·P(β)]/(kz − β). L'onde de
  surface est alors exactement compensée (intégrale de contour).
- **Règle du point milieu** : z_j = (j + ½)Δz, kz_n = (n + ½)Δk ; Δz = λ/10, Δr = 10 λ (≥ 8 λ).
- **Départ analytique** à r₀ = max(3λ, 10 m) : onde directe + réflexion de Weyl-van der Pol, pondérées
  par la directivité complexe du réseau de subs selon le site.
- **Couche absorbante** de max(30 λ, 100 m), Im k = A_t((z − z_t)/h)².
- **Relief** : coordonnées qui suivent le terrain ; rotation de phase e^(−i ka z sin Δθ) aux
  changements de pente.

Sortie : `ΔL = 20 lg|ψ| − 10 lg r + 20 lg R`.

Tests : sol rigide ±0,05 dB ; prairie (σ = 200) ±0,7 dB jusqu'à 1,5 km à 50/100/160 Hz ;
réfraction vers le bas > homogène + 5 dB, vers le haut < homogène − 10 dB (Salomons 2001).

## 10. Fusion, scénarios, incertitude (ADR 0008)

Poids PE : 1 (≤ 125 Hz), 0,67 (160 Hz), 0,33 (200 Hz), 0 (≥ 250 Hz), mélange en énergie avec la
méthode d'ingénierie. Scénarios :
- prévision AROME (Monin-Obukhov) ;
- neutre sans vent ;
- inversion forte (Pasquill F, vent 1 m/s) ;
- vent portant 5 m/s vers les habitations.

Cartes médiane et P90 cellule par cellule.

## 11. Impact (`impact.rs`)

- **Résiduel** : max énergétique du rural de nuit (25 dB(A) par défaut) et des routes
  (L₁₀ₘ = 58/52/46/40/34 dB(A) selon l'importance, −10 lg(d/10)). Une mesure saisie le remplace.
  Spectre type par octave recalé sur le niveau A.
- **Émergence** : `E = 10 lg(10^(Ls/10) + 10^(Lr/10)) − Lr`.
- **Marge** jusqu'au seuil T : `Ls_max = Lr + 10 lg(10^(T/10) − 1)`, marge = Ls_max − Ls.
- **Façades** : milieu des façades ≥ 3 m, à 1 m de l'extérieur, à 1,5 m et 4 m ; on retient la plus
  exposée.
- **Habitants** : Filosofi 200 m réparti au prorata des logements.

Tests : émergence (+3,01 dB à niveaux égaux), inverse exact, spectre résiduel, façades extérieures.

## 12. Dimensionnement et optimiseur

- **Subs** (serrés, cohérents) : n = n₀·10^((cible_C − L_C)/20).
- **Têtes** (énergie) : n = n₀·10^((cible_A − L_A)/10).
- **Puissance** = Σ n × P_nominale.
- **Montages** : cardioïde (moitié arrière décalée de λ/4 à 63 Hz, inversée, retardée de d/c) ;
  end-fire (subs alignés sur l'axe, retards k·d/c).
- **Score de l'optimiseur** :
  `marge − 3·déficit_dancefloor`, avec la référence min(cible, niveau actuel − 1 dB).
  Les 3 meilleurs candidats sont vérifiés au GFPE.

## 13. Phase 5 : sol, végétation, vallées, calage (ADR 0009, ADR 0010)

- **Facteur de feuillage LiDAR** (`vegetation.rs`) : d_f = Σ Δx·f sous la canopée.
  - f = ln P / ln 0,2, borné à [0 ; 1,5], avec P = n(< 1 m)/n(< 11 m), le taux de passage des
    impulsions (Beer-Lambert).
  - Densité d'un voxel : d_l = n_l / Σ_{k ≤ l} n_k.
- **État du sol** (`ground_state.rs`) :
  - σ × [1 + 1,5·clamp((θ − 0,25)/0,20, 0, 1)], avec θ l'humidité 0–1 cm en m³/m³ ;
  - neige ≥ 5 cm → σ = 12,5 kPa·s/m² ;
  - falaises (pente ≥ 45°) → classe G ;
  - sols minces sur roche dure (lithologie BRGM) durcis d'une classe.
- **Réfraction imposée** (`engineering.rs`, `Refraction`) : g(az) = g_T + g_W·cos(az − az_vent).
  Rc = c/g si g > 0 (au moins 200 m), sinon rayons droits. Elle remplace Rc = max(1000, 8d).
- **Écoulements de vallée** (`atmosphere.rs`, `ValleyFlow`) :
  - lac d'air froid : T(z) += −ΔT·(1 − z/h)² sous h ;
  - vent catabatique : u(z) = u₀·e^(−z/l)·sin(z/l), u_max = u₀·e^(−π/4)·sin(π/4) en z = πl/4,
    ajouté à c_eff selon cos(az_aval − az).
- **Calage bayésien** (`calibrate.rs`) : vraisemblance marginale sur δ (a priori N(0, τ²)), avec
  w = 1/s² :
  ln L = −½·[Σr²·w − (Σr·w)²/(N·w + 1/τ²)] ; δ̂ = Σr·w/(N·w + 1/τ²).
- **Criblage de spots** (`spots.rs`) : L = L₁ − 20 lg d + 3 − A, avec A = 10 lg(3 + 20N),
  N = 2δ/λ à 63 Hz, rayons courbes favorables, A ≤ 20 dB.
- **Auralisation** (`auralize.rs`) : égaliseur à phase nulle par FFT, gain en dB interpolé sur
  log f entre les centres de bande ; bruit rose (filtre de P. Kellet) au niveau résiduel relatif.

## 14. Modèles d'enceintes saisis (ADR 0011)
- **Niveau du modèle :** SPL max continu, champ libre, par octave.
  - Depuis une fiche technique : L_oct = L − 6·[crête] − 6·[demi-espace] − 10·lg(n), où n est
    le nombre d'octaves de la bande passante (≥ 1).
  - Le −6 dB de crête correspond au facteur de crête du bruit rose. Le −6 dB de demi-espace
    vient de ce que le gain de sol est recalculé par la propagation.
- **Coupures données à −10 dB :** f₋₃ = f₋₁₀·2^(±7/pente).
- **Réponse mesurée :** interpolation linéaire en dB sur log f, prolongée par la pente hors
  des points.
- **Cardioïde du premier ordre :** p(θ) = A + B·cos θ, A + B = 1, A − B = 10^(−R/20).
  Exemple : R = 20 dB donne −5,2 dB sur le côté et −20 dB à l'arrière.

## 15. Terrain 3D (non acoustique)

Terrarium : `h = R·256 + G + B/256 − 32768`.
