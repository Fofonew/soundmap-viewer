// Scènes de comparaison SoundMap ↔ NoiseModelling (CNOSSOS-EU).
// Usage (ScriptRunner) : -s scene.groovy --scene flat_soft|flat_hard|barrier
//
// Géométrie commune (L93, sol plat à z = 0) :
//   source ponctuelle omnidirectionnelle à 2 m, Lw = 100 dB par octave ;
//   récepteurs à 4 m sur l'axe +x.
// flat_soft : G = 1 partout ; flat_hard : G = 0 partout ;
// barrier   : G = 1 + bâtiment de 10 m de haut, 20 m d'épaisseur
//             (x = 80…100 m), 500 m de long (pas de contournement latéral).

import groovy.sql.Sql
import org.h2gis.api.ProgressVisitor
import java.sql.Connection

title = 'SoundMap comparison scene'
description = 'Builds a simple scene to compare SoundMap with NoiseModelling'

inputs = [scene: [name: 'scene', title: 'scene', description: 'flat_soft, flat_hard or barrier', type: String.class]]
outputs = [result: [name: 'result', title: 'result', description: 'status', type: String.class]]

def exec(Connection connection, Map input, ProgressVisitor progress) {
    String scene = input['scene'] as String
    double x0 = 700000, y0 = 6300000
    double g = scene == 'flat_hard' ? 0.0 : 1.0
    List<Double> dists = scene == 'barrier' ? [150d, 200d, 300d, 500d, 800d] : [25d, 50d, 100d, 200d, 400d, 800d]
    Sql sql = new Sql(connection)
    ['SOURCES', 'RECEIVERS', 'BUILDINGS', 'GROUND', 'RECEIVERS_LEVEL'].each { sql.execute("DROP TABLE IF EXISTS " + it) }

    def bands = [63, 125, 250, 500, 1000, 2000, 4000, 8000]
    def cols = ['D', 'E', 'N'].collect { p -> bands.collect { "HZ${p}${it} REAL" }.join(', ') }.join(', ')
    sql.execute("CREATE TABLE SOURCES(PK INTEGER PRIMARY KEY, THE_GEOM GEOMETRY(POINTZ, 2154), " + cols + ")")
    def vals = (1..24).collect { '100' }.join(', ')
    sql.execute("INSERT INTO SOURCES VALUES (1, ST_GeomFromText('POINTZ(" + x0 + " " + y0 + " 2)', 2154), " + vals + ")")

    sql.execute("CREATE TABLE RECEIVERS(PK INTEGER PRIMARY KEY, THE_GEOM GEOMETRY(POINTZ, 2154))")
    dists.eachWithIndex { d, i ->
        sql.execute("INSERT INTO RECEIVERS VALUES (" + (i + 1) + ", ST_GeomFromText('POINTZ(" + (x0 + d) + " " + y0 + " 4)', 2154))")
    }

    sql.execute("CREATE TABLE BUILDINGS(PK INTEGER PRIMARY KEY, THE_GEOM GEOMETRY(POLYGON, 2154), HEIGHT REAL)")
    if (scene == 'barrier') {
        def (bx0, bx1, by0, by1) = [x0 + 80, x0 + 100, y0 - 250, y0 + 250]
        sql.execute("INSERT INTO BUILDINGS VALUES (1, ST_GeomFromText('POLYGON((" +
                "$bx0 $by0, $bx1 $by0, $bx1 $by1, $bx0 $by1, $bx0 $by0))', 2154), 10)")
    } else {
        // Petit bâtiment hors de portée (la table ne doit pas être vide).
        def (bx0, by0) = [x0 - 3000, y0 - 3000]
        sql.execute("INSERT INTO BUILDINGS VALUES (1, ST_GeomFromText('POLYGON((" +
                "$bx0 $by0, ${bx0 + 5} $by0, ${bx0 + 5} ${by0 + 5}, $bx0 ${by0 + 5}, $bx0 $by0))', 2154), 5)")
    }

    sql.execute("CREATE TABLE GROUND(PK INTEGER PRIMARY KEY, THE_GEOM GEOMETRY(POLYGON, 2154), G REAL)")
    def (gx0, gx1, gy0, gy1) = [x0 - 2000, x0 + 2000, y0 - 2000, y0 + 2000]
    sql.execute("INSERT INTO GROUND VALUES (1, ST_GeomFromText('POLYGON((" +
            "$gx0 $gy0, $gx1 $gy0, $gx1 $gy1, $gx0 $gy1, $gx0 $gy0))', 2154), " + g + ")")
    return "scene " + scene + " : " + dists.size() + " récepteurs"
}
