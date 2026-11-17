# ADR 0006 — Occupation du sol : tuiles vectorielles OCS GE, repli CoSIA

**Statut :** accepté (session 1)

- Source principale : TMS pbf `OCSGE-TMS.COUVERTURE.2021-2023` (source-layer `occupation_sol`, attribut `code_cs`, z12–16), au zoom 16 sur le disque d'étude.
  - Les tuiles vectorielles gardent les codes de classe ; une tuile WMTS PNG ne garderait qu'une couleur.
- Repli : si aucune tuile OCS GE n'existe sur la zone, des tuiles CoSIA PNG (`IGNF_COSIA_2021-2023`) sont ajoutées au plan puis téléchargées automatiquement.
- La rasterisation sur la grille de calcul (facteur G, résistivité σ) est reportée à la phase 2.
