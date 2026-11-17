# SoundMap

Simulateur 3D de propagation sonore en extérieur, construit sur le LiDAR HD de l'IGN. Projet de recherche et d'usage personnel.
Spécification : `PROMPT.md` · contexte : `docs/RECHERCHE.md` · endpoints vérifiés : `docs/ENDPOINTS.md` · décisions : `docs/ADR/`.

## Prérequis (Windows)
- Rust stable ≥ 1.90, avec la cible MSVC (et `rustup target add wasm32-unknown-unknown` pour la CI).
- Node.js ≥ 20.
- WebView2 (fourni avec Windows 11).

## Lancer l'application
```bash
cd ui
npm install
npm run tauri dev
```
Les zones sont stockées dans `SOUNDMAP_ZONES_DIR`, ou par défaut dans le dossier de données de l'app (`%APPDATA%\fr.soundmap.viewer\zones`). Le cache de tuiles suit le même principe avec `SOUNDMAP_CACHE_DIR`. `SOUNDMAP_OFFLINE=1` coupe le réseau du proxy de tuiles.

Utilisation :
1. **Exploration** : vue satellite (ou Plan IGN), recherche d'adresse, emprises des dalles LiDAR HD.
   **Assistant de spot** : criblage sur 10 à 30 km (relief, population, écran du relief à 63 Hz).
   Il classe les spots plats par habitants exposés ; un clic place la sono.
2. **Zone de travail** : « Placer la sono », choisir le rayon (estimation du volume et du temps),
   « Valider et télécharger » (annulable, reprenable).
3. **Vue 3D** (hors ligne), onglets :
   - **Sono** : placer les enceintes (presets ou saisie manuelle), les glisser sur la carte, régler
     azimut, hauteur, retard, polarité et gain, dessiner le dancefloor ;
   - **Carte** : niveaux dB(A), dB(C), dB(Z) ou par octave, recalculés en direct (méthode
     d'ingénierie, < 300 ms), isophones, coupe du terrain en un point ;
   - **Carte**, outil coupe : **Écouter ce point** joue deux extraits (dancefloor, et chez le
     riverain avec le bruit résiduel) ;
   - **Basses fréq.** : calcul GFPE (sur GPU s'il y en a un, sinon CPU) sur 4 scénarios météo
     (AROME du soir, neutre, inversion, vent portant), plus un 5ᵉ (lac d'air froid et vent
     catabatique) si la sono est en fond de vallée ; cartes médiane et P90 ;
   - **Impact** : seuils d'émergence libres, bruit résiduel, habitations et habitants exposés,
     bâtiment le plus exposé, marge, lieux sensibles, ZNIEFF ;
   - **Dimens.** : dimensionnement inverse (nombre de boîtes, puissance, homogénéité) et optimiseur
     (position, azimut, montage cardioïde ou end-fire, vérification GFPE) ;
   - **Projet** : réglages, `.smproj`, exports HTML (imprimable en PDF), GeoTIFF et GeoJSON ;
     sol (géologie BRGM, falaises, humidité ou neige à l'heure du calcul) ; végétation LiDAR HD
     (COPC) à la demande ; calage sur mesures de sonomètre (CSV ou GPX).

La physique est détaillée dans `docs/PHYSIQUE.md`, les choix dans `docs/ADR/`.

## Outils en ligne de commande
```bash
cargo run -p sm-data --example probe                     # critère de sortie de la phase 0
cargo run --release -p sm-data --example zone -- 3.013 44.004 5000 "Ma zone" --dir zones
cargo run --release -p sm-compute --example simulate -- zones/<id> --pe --opt --out export   # simulation complète + mesures
cargo run --release -p sm-compute --example vegetation -- zones/<id> 1000 2.0   # LiDAR COPC (rayon, espacement)
cargo run --release -p sm-compute --example ground -- zones/<id>                # géologie + état du sol
cargo run --release -p sm-compute --example spots -- 3.12 44.05 30000           # assistant de spot
cargo run --release -p sm-compute --example calib_check -- zones/<id>           # calage sur mesures synthétiques
cargo run --release -p sm-core --example compare_nm                             # comparaison NoiseModelling
```

Variables : `SOUNDMAP_GPU=0` force le GFPE sur CPU.

## Validation
- NoiseModelling 6.0.0 : `docs/VALIDATION-NOISEMODELLING.md` (scènes dans `tests/noisemodelling/`).
- GFPE GPU = CPU à 0,004 dB : `cargo test -p sm-gpu --release` (ignoré sans GPU).

## Qualité
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # hors ligne
cargo test -p sm-data -- --ignored     # services publics réels
cargo check -p sm-core --target wasm32-unknown-unknown
cargo bench -p sm-geo -p sm-data
cd ui && npm run build && npm test
```
