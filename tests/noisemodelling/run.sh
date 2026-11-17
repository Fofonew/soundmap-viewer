#!/usr/bin/env bash
# Calcule les scènes de comparaison avec NoiseModelling et exporte les niveaux en CSV.
#
#   NM_HOME=/chemin/NoiseModelling_6.0.0 JAVA_HOME=/chemin/jdk ./tests/noisemodelling/run.sh
#
# Sorties : tests/noisemodelling/out/<scene>_<H|F>.csv (H = homogène, F = favorable).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${NM_HOME:?NM_HOME doit pointer vers le dossier NoiseModelling}"
export PATH="${JAVA_HOME:+$JAVA_HOME/bin:}$PATH"
out="$here/out"
work="$out/work"
mkdir -p "$work"

# Appel direct de la JVM (comme bin/ScriptRunner), portable et sans le
# découpage des virgules par cmd.exe.
sep=":"; [[ "${OS:-}" == "Windows_NT" ]] && sep=";"
runner() {
  (cd "$NM_HOME" && java -Xmx4g -classpath "etc${sep}lib/*" -Dbasedir="$NM_HOME" \
    org.noise_planet.noisemodelling.runner.Main "$@")
}

zeros="0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0"
ones="1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1"
for scene in flat_soft flat_hard barrier; do
  for cond in H F; do
    occ=$zeros; [[ $cond == F ]] && occ=$ones
    runner -w "$work" -s "$here/scene.groovy" --scene "$scene"
    runner -w "$work" -s scripts/NoiseModelling/Noise_level_from_source.groovy \
      --tableBuilding BUILDINGS --tableSources SOURCES --tableReceivers RECEIVERS \
      --tableGroundAbs GROUND --confReflOrder 0 --confMaxSrcDist 1500 \
      --confDiffHorizontal true --confMaxError 0 \
      --confFavourableOccurrencesDefault "$occ"
    runner -w "$work" -s scripts/Import_and_Export/Export_Table.groovy \
      --exportPath "$out/${scene}_${cond}.csv" --tableToExport RECEIVERS_LEVEL
  done
done
echo "OK : $(ls "$out"/*.csv | wc -l) fichiers dans $out"
