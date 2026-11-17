# ADR 0009 — GFPE sur GPU (wgpu / WGSL)

**Statut :** accepté (phase 5)

## Contexte
Le GFPE N×2D (360 azimuts × 20 fréquences, 25–200 Hz) représente l'essentiel du temps de calcul
basses fréquences : environ 10 s par scénario météo sur 28 threads, 24 à 36 s sur 8 threads.

## Décisions
1. **Crate séparé `sm-gpu`** (wgpu 30 + WGSL).
   - `sm-core` reste pur, sans I/O ni GPU, et compilable en wasm32.
   - `sm-compute` l'active par la fonctionnalité `gpu` (activée par défaut).
2. **Discrétisation partagée.** `sm_core::gfpe` expose la grille et les données d'entrée :
   - `GfpeGrid` : grille au point milieu, Δr et Δz, couche absorbante ;
   - `step_params` : rotation de relief, longueur du pas, β et P(β) à chaque pas ;
   - `starter` : champ de départ de Weyl-van der Pol pondéré par la directivité ;
   - `receiver_level` : niveau au récepteur.

   Le CPU et le GPU utilisent les mêmes données : seules les opérations de la boucle diffèrent.
3. **Organisation GPU.**
   - Un lot regroupe tous les azimuts d'une fréquence, avec un groupe de travail de 256 threads
     par azimut.
   - Un seul dispatch par pas en distance enchaîne toutes les étapes :
     rotation, réduction Ψ(β), FFT directe, propagateur à pôle soustrait, deux FFT, phase δk(z)
     et niveau au récepteur.
   - La FFT est un Stockham radix 2 en mémoire globale, avec barrières de groupe de travail.
     Un groupe traite toute la FFT de son azimut, donc aucune synchronisation globale n'est
     nécessaire.
   - Le numéro de pas passe par un uniform à offset dynamique. Les commandes sont soumises par
     paquets de 64 pas, avec vérification d'annulation entre deux paquets.
4. **Précision f32.** Les constantes (twiddles, propagateur, β, P(β), champ de départ) sont
   calculées en f64 sur le CPU, puis converties en f32.
5. **Bascule automatique.**
   - `GpuGfpe::new()` renvoie `None` dans ces cas, et le CPU est alors utilisé :
     aucun adaptateur, pas de compute shaders, ou rendu logiciel (llvmpipe, WARP).
   - En cas d'erreur GPU pendant le calcul : repli sur le CPU, avec un avertissement dans les logs.
   - `SOUNDMAP_GPU=0` force le CPU.
   - Le moteur utilisé est indiqué dans la description de méthode des résultats.

## Validation
- `cargo test -p sm-gpu --release` compare le GPU au CPU à 31,5, 100 et 200 Hz sur 3 km :
  sol plat mou, réfraction vers le bas, relief + sol mixte. Écart max mesuré : **0,004 dB**
  (tolérance 0,5 dB, en énergie par fenêtres de 100 m). Le test est ignoré sans GPU, donc en CI.
- Zone Larzac de 5 km, 4 scénarios : surfaces exposées identiques à 10⁻⁴ km² près.

## Performances (RTX 5070 Ti, Vulkan ; CPU 28 threads chargé à 50 % par un autre processus)
| | CPU | GPU |
|---|---|---|
| Un scénario complet, 360 az × 20 fréquences | 10 s | **3,1 à 3,8 s** |

La préparation sur le CPU (champs de départ et paramètres de pas pour 360 azimuts) représente une
part notable du temps GPU ; c'est la prochaine piste d'optimisation.
