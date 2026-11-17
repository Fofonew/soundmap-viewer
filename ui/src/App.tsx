import * as maplibregl from 'maplibre-gl';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  api,
  fmtBytes,
  fmtDuration,
  type AppInfo,
  type Category,
  type EstimateResponse,
  type GeocodeResult,
  type OpenZoneResponse,
  type PackProgress,
  type SpotProgress,
  type SpotReport,
  type ZoneRequest,
  type ZoneSummary,
} from './api';
import { boundsAround } from './geo';
import { exploreStyle, zoneFeatures, zoneStyle } from './mapStyles';
import { SimPanel } from './sim/SimPanel';

type Mode = 'explore' | 'zone3d';
type Basemap = 'ortho' | 'plan';

const CATEGORY_LABEL: Record<Category, string> = {
  relief: 'Relief LiDAR HD (MNT, MNS)',
  vecteurs: 'Vecteurs (bâti, végétation, routes, Filosofi, ZNIEFF)',
  occupation_sol: 'Occupation du sol',
  meteo: 'Météo AROME',
  ortho: 'Ortho haute résolution',
};

function todayLocal(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function errText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function App() {
  const mapDiv = useRef<HTMLDivElement>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [mode, setMode] = useState<Mode>('explore');
  const [basemap, setBasemap] = useState<Basemap>('ortho');
  const [lidar, setLidar] = useState(false);
  const [picking, setPicking] = useState(false);
  const pickingRef = useRef(false);
  const [center, setCenter] = useState<[number, number] | null>(null);
  const [radius, setRadius] = useState(5000);
  const [name, setName] = useState('');
  const [date, setDate] = useState(todayLocal());
  const [est, setEst] = useState<EstimateResponse | null>(null);
  const [estimating, setEstimating] = useState(false);
  const [job, setJob] = useState<string | null>(null);
  const [progress, setProgress] = useState<PackProgress | null>(null);
  const [zones, setZones] = useState<ZoneSummary[]>([]);
  const [opened, setOpened] = useState<OpenZoneResponse | null>(null);
  const [openTiming, setOpenTiming] = useState<{ rust: number; idle: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [spotRadius, setSpotRadius] = useState(30000);
  const [spotThreshold, setSpotThreshold] = useState(50);
  const [spotLevel, setSpotLevel] = useState(125);
  const [spotJob, setSpotJob] = useState<SpotProgress | null | 'start'>(null);
  const [spotReport, setSpotReport] = useState<SpotReport | null>(null);

  const refreshZones = useCallback(() => {
    api.listZones().then(setZones).catch((e) => setError(errText(e)));
  }, []);

  // Carte : créée une seule fois, en mode Exploration.
  useEffect(() => {
    if (!mapDiv.current || mapRef.current) return;
    const map = new maplibregl.Map({
      container: mapDiv.current,
      style: exploreStyle(),
      center: [2.6, 46.6],
      zoom: 5.4,
      maxPitch: 80,
      attributionControl: { compact: true },
    });
    map.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), 'top-right');
    map.addControl(new maplibregl.ScaleControl({ unit: 'metric' }), 'bottom-right');
    map.on('click', (e) => {
      if (pickingRef.current) {
        setCenter([e.lngLat.lng, e.lngLat.lat]);
        setPicking(false);
      }
    });
    mapRef.current = map;
    if (import.meta.env.DEV) (window as unknown as { __map: maplibregl.Map }).__map = map;
    api.appInfo().then(setInfo).catch(() => undefined);
    refreshZones();
    return () => {
      map.remove();
      mapRef.current = null;
    };
  }, [refreshZones]);

  useEffect(() => {
    pickingRef.current = picking;
    const c = mapRef.current?.getCanvas();
    if (c) c.style.cursor = picking ? 'crosshair' : '';
  }, [picking]);

  // Fond de carte et emprises LiDAR (mode Exploration).
  const applyExploreLayers = useCallback(() => {
    const map = mapRef.current;
    if (!map || !map.getLayer('ortho')) return;
    map.setLayoutProperty('ortho', 'visibility', basemap === 'ortho' ? 'visible' : 'none');
    map.setLayoutProperty('plan', 'visibility', basemap === 'plan' ? 'visible' : 'none');
    for (const l of ['lidar-fill', 'lidar-line']) map.setLayoutProperty(l, 'visibility', lidar ? 'visible' : 'none');
  }, [basemap, lidar]);

  const applyZoneOverlay = useCallback(() => {
    const src = mapRef.current?.getSource('zone') as maplibregl.GeoJSONSource | undefined;
    if (!src || mode !== 'explore') return;
    src.setData(
      center
        ? zoneFeatures(center[0], center[1], radius, est?.rings.slice(0, -1) ?? [])
        : { type: 'FeatureCollection', features: [] },
    );
  }, [center, radius, est, mode]);

  useEffect(() => {
    const map = mapRef.current;
    if (!map || mode !== 'explore') return;
    const apply = () => {
      applyExploreLayers();
      applyZoneOverlay();
    };
    // isStyleLoaded() reste faux tant que des tuiles chargent : on teste la
    // présence des couches, sinon on attend le chargement du style.
    if (map.getLayer('ortho')) apply();
    else map.once('style.load', apply);
  }, [mode, applyExploreLayers, applyZoneOverlay]);

  // Estimation (débouncée) dès que la zone change.
  const request = useCallback((): ZoneRequest | null => {
    if (!center) return null;
    const fallback = `Zone ${center[1].toFixed(3)} ${center[0].toFixed(3)}`;
    return { name: name.trim() || fallback, lon: center[0], lat: center[1], radius_m: radius, meteo_date: date };
  }, [center, name, radius, date]);

  useEffect(() => {
    const req = request();
    if (!req) return;
    setEstimating(true);
    const t = setTimeout(() => {
      api
        .estimateZone(req)
        .then((r) => {
          setEst(r);
          setError(null);
        })
        .catch((e) => setError(errText(e)))
        .finally(() => setEstimating(false));
    }, 250);
    return () => clearTimeout(t);
  }, [request]);

  const runJob = useCallback(
    (id: string, p: Promise<ZoneSummary>) => {
      setJob(id);
      setError(null);
      p.then(() => refreshZones())
        .catch((e) => {
          // Une annulation volontaire n'est pas une erreur : la zone reste reprenable.
          const t = errText(e);
          if (!/annulée/.test(t)) setError(t);
        })
        .finally(() => {
          setJob(null);
          refreshZones();
          setEst((cur) => cur && { ...cur });
        });
    },
    [refreshZones],
  );

  const startDownload = () => {
    const req = request();
    if (!req || !est) return;
    setProgress(null);
    runJob(est.id, api.downloadZone(req, setProgress));
  };

  const resume = (id: string) => {
    setProgress(null);
    runJob(id, api.resumeZone(id, setProgress));
  };

  const runSpots = async () => {
    const map = mapRef.current;
    if (!map) return;
    const c = center ?? [map.getCenter().lng, map.getCenter().lat];
    setSpotJob('start');
    setError(null);
    try {
      const r = await api.findSpots(
        c[0],
        c[1],
        { radius_m: spotRadius, grid_m: 500, source_dbc_1m: spotLevel, threshold_dbc: spotThreshold, max_slope_pct: 8, influence_m: 6000 },
        (p) => setSpotJob(p),
      );
      setSpotReport(r);
      const b = new maplibregl.LngLatBounds();
      for (const s of r.spots) b.extend([s.lon, s.lat]);
      if (!b.isEmpty()) map.fitBounds(b, { padding: 80, maxZoom: 12 });
    } catch (e) {
      const t = errText(e);
      if (!/annul/.test(t)) setError(t);
    } finally {
      setSpotJob(null);
    }
  };

  // Candidats de l'assistant (couleur : habitants exposés) et meilleurs spots.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || mode !== 'explore') return;
    const apply = () => {
      const data: GeoJSON.FeatureCollection = {
        type: 'FeatureCollection',
        features: [
          ...(spotReport?.candidates ?? []).map((s) => ({
            type: 'Feature' as const,
            properties: { e: s.exposed, top: 0 },
            geometry: { type: 'Point' as const, coordinates: [s.lon, s.lat] },
          })),
          ...(spotReport?.spots ?? []).map((s, i) => ({
            type: 'Feature' as const,
            properties: { e: s.exposed, top: 1, rank: String(i + 1) },
            geometry: { type: 'Point' as const, coordinates: [s.lon, s.lat] },
          })),
        ],
      };
      const src = map.getSource('spots') as maplibregl.GeoJSONSource | undefined;
      if (src) {
        src.setData(data);
        return;
      }
      map.addSource('spots', { type: 'geojson', data });
      map.addLayer({
        id: 'spots-cand',
        type: 'circle',
        source: 'spots',
        filter: ['==', ['get', 'top'], 0],
        paint: {
          'circle-radius': ['interpolate', ['linear'], ['zoom'], 8, 2, 13, 6],
          'circle-color': ['interpolate', ['linear'], ['get', 'e'], 0, '#2fb67c', 20, '#f5b84a', 200, '#e4302c'],
          'circle-opacity': 0.75,
        },
      });
      map.addLayer({
        id: 'spots-top',
        type: 'circle',
        source: 'spots',
        filter: ['==', ['get', 'top'], 1],
        paint: { 'circle-radius': 11, 'circle-color': '#14161a', 'circle-stroke-color': '#ffffff', 'circle-stroke-width': 2 },
      });
      map.addLayer({
        id: 'spots-rank',
        type: 'symbol',
        source: 'spots',
        filter: ['==', ['get', 'top'], 1],
        layout: { 'text-field': ['get', 'rank'], 'text-size': 12, 'text-allow-overlap': true },
        paint: { 'text-color': '#ffffff' },
      });
    };
    if (map.getLayer('ortho')) apply();
    else map.once('style.load', apply);
  }, [spotReport, mode]);

  const open3d = async (id: string) => {
    const map = mapRef.current;
    if (!map) return;
    const t0 = performance.now();
    try {
      const z = await api.openZone(id);
      setOpened(z);
      setMode('zone3d');
      setPicking(false);
      const [lon, lat] = z.products.center_lonlat;
      map.setStyle(zoneStyle(z), { diff: false });
      map.setMaxBounds(boundsAround(lon, lat, z.spec.radius_m * 1.4));
      map.jumpTo({ center: [lon, lat], zoom: 14.2, pitch: 62, bearing: -25 });
      map.once('idle', () => {
        const idle = performance.now() - t0;
        setOpenTiming({ rust: z.open_ms, idle });
        api.logMetric('zone_open_to_idle', idle).catch(() => undefined);
      });
    } catch (e) {
      setError(errText(e));
    }
  };

  const backToExplore = () => {
    const map = mapRef.current;
    if (!map) return;
    const c = opened?.products.center_lonlat;
    setMode('explore');
    setOpened(null);
    setOpenTiming(null);
    map.setMaxBounds(null);
    map.setStyle(exploreStyle(), { diff: false });
    map.jumpTo({ pitch: 0, bearing: 0, zoom: 12, ...(c ? { center: c } : {}) });
  };

  const deleteZone = async (id: string) => {
    try {
      await api.deleteZone(id);
      refreshZones();
    } catch (e) {
      setError(errText(e));
    }
  };

  const busy = job !== null;
  const existing = est ? zones.find((z) => z.id === est.id) : undefined;

  return (
    <div className="app">
      <aside className="panel">
        <header className="brand">
          <div className="logo" aria-hidden>
            <span />
            <span />
            <span />
          </div>
          <div>
            <h1>SoundMap</h1>
            <p className="muted">{mode === 'explore' ? 'Exploration' : 'Zone de travail · 3D hors ligne'}</p>
          </div>
          {info?.offline && <span className="chip warn">hors ligne</span>}
        </header>

        {error && (
          <div className="alert" role="alert">
            <span>{error}</span>
            <button className="ghost" onClick={() => setError(null)} aria-label="Fermer">
              ×
            </button>
          </div>
        )}

        {mode === 'explore' ? (
          <>
            <SearchBox
              onSelect={(r) => {
                mapRef.current?.flyTo({ center: [r.lon, r.lat], zoom: 13.5, duration: 1200 });
                setName(r.label);
              }}
              offline={!!info?.offline}
            />

            <section className="card">
              <h2>Fond de carte</h2>
              <div className="segmented" role="radiogroup">
                {(['ortho', 'plan'] as Basemap[]).map((b) => (
                  <button key={b} role="radio" aria-checked={basemap === b} className={basemap === b ? 'on' : ''} onClick={() => setBasemap(b)}>
                    {b === 'ortho' ? 'Satellite' : 'Plan IGN'}
                  </button>
                ))}
              </div>
              <label className="check">
                <input type="checkbox" checked={lidar} onChange={(e) => setLidar(e.target.checked)} />
                Emprises des dalles LiDAR HD <span className="muted">(zoom ≥ 10)</span>
              </label>
            </section>

            <section className="card">
              <h2>Zone de travail</h2>
              <button className={picking ? 'primary pulse' : 'primary'} onClick={() => setPicking((p) => !p)} disabled={busy}>
                {picking ? 'Cliquez sur la carte…' : center ? 'Déplacer la sono' : 'Placer la sono'}
              </button>
              {center && (
                <>
                  <p className="mono muted">
                    {center[1].toFixed(5)}° N, {center[0].toFixed(5)}° E
                  </p>
                  <label className="field">
                    <span>
                      Rayon d'étude <strong>{(radius / 1000).toFixed(1)} km</strong>
                    </span>
                    <input type="range" min={1000} max={10000} step={500} value={radius} onChange={(e) => setRadius(Number(e.target.value))} disabled={busy} />
                  </label>
                  <label className="field">
                    <span>Nom</span>
                    <input type="text" value={name} placeholder="ex. Causse Noir" onChange={(e) => setName(e.target.value)} disabled={busy} />
                  </label>
                  <label className="field">
                    <span>Date (météo AROME)</span>
                    <input type="date" value={date} onChange={(e) => setDate(e.target.value)} disabled={busy} />
                  </label>
                  {est && <EstimateCard est={est} loading={estimating} />}
                  {est && !busy && (
                    existing?.status === 'complete' ? (
                      <button className="primary" onClick={() => open3d(est.id)}>
                        Déjà téléchargée · Ouvrir en 3D
                      </button>
                    ) : (
                      <button className="primary" onClick={startDownload} disabled={estimating}>
                        {existing ? 'Reprendre le téléchargement' : 'Valider et télécharger'}
                      </button>
                    )
                  )}
                </>
              )}
            </section>

            <section className="card">
              <h2>Assistant de spot</h2>
              <p className="muted small">
                Criblage sur un grand rayon autour {center ? 'de la sono placée' : 'du centre de la carte'} : relief RGE ALTI 100 m,
                population Filosofi, écran du relief à 63 Hz en conditions favorables. Classe les spots plats par habitants exposés.
              </p>
              <div className="row">
                <label className="field">
                  <span>Rayon</span>
                  <select value={spotRadius} onChange={(e) => setSpotRadius(Number(e.target.value))}>
                    {[10000, 20000, 30000].map((r) => (
                      <option key={r} value={r}>
                        {r / 1000} km
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>Système (dB(C) à 1 m)</span>
                  <select value={spotLevel} onChange={(e) => setSpotLevel(Number(e.target.value))}>
                    <option value={119}>119 (1 sub)</option>
                    <option value={125}>125 (4 subs)</option>
                    <option value={131}>131 (16 subs)</option>
                  </select>
                </label>
                <label className="field">
                  <span>Seuil (dB(C))</span>
                  <input type="number" min={30} max={80} value={spotThreshold} onChange={(e) => setSpotThreshold(Number(e.target.value))} />
                </label>
              </div>
              {spotJob ? (
                <div className="row">
                  <span className="small">{spotJob === 'start' ? 'Démarrage…' : `${spotJob.message} (${spotJob.done}/${spotJob.total})`}</span>
                  <button className="ghost danger" onClick={() => api.cancelJob('spots')}>
                    Annuler
                  </button>
                </div>
              ) : (
                <button onClick={runSpots} disabled={busy}>
                  Chercher des spots
                </button>
              )}
              {spotReport && (
                <>
                  <p className="muted small">
                    {spotReport.candidates.length} candidats, {Math.round(spotReport.population).toLocaleString('fr-FR')} habitants dans le
                    périmètre — {spotReport.seconds.toFixed(1)} s
                  </p>
                  <ol className="small spots">
                    {spotReport.spots.map((s, i) => (
                      <li key={i}>
                        <button
                          className="link"
                          onClick={() => {
                            setCenter([s.lon, s.lat]);
                            mapRef.current?.flyTo({ center: [s.lon, s.lat], zoom: 13 });
                          }}
                        >
                          {Math.round(s.exposed)} exposés · 1er habitant à {(s.nearest_m / 1000).toFixed(1)} km · max{' '}
                          {s.max_dbc.toFixed(0)} dB(C) · pente {s.slope_pct.toFixed(1)} % · {Math.round(s.elevation_m)} m
                        </button>
                      </li>
                    ))}
                  </ol>
                  <p className="muted small">Cliquez un spot pour y placer la sono, puis téléchargez la zone pour un calcul complet.</p>
                </>
              )}
            </section>

            {busy && <ProgressCard progress={progress} onCancel={() => job && api.cancelDownload(job)} />}

            <ZoneList zones={zones} busy={busy} onOpen={open3d} onResume={resume} onDelete={deleteZone} />
          </>
        ) : (
          opened &&
          mapRef.current && (
            <>
              <SimPanel key={opened.summary.id} map={mapRef.current} zone={opened} onBack={backToExplore} />
              {openTiming && (
                <p className="mono muted small">
                  Zone ouverte hors ligne : pack {openTiming.rust.toFixed(0)} ms · rendu 3D complet {openTiming.idle.toFixed(0)} ms
                </p>
              )}
            </>
          )
        )}

        <footer className="muted small">
          Données © IGN Géoplateforme, INSEE, PatriNat, Météo-France via Open-Meteo. Usage de recherche.
        </footer>
      </aside>
      <main ref={mapDiv} className="map" />
    </div>
  );
}

function SearchBox({ onSelect, offline }: { onSelect: (r: GeocodeResult) => void; offline: boolean }) {
  const [q, setQ] = useState('');
  const [results, setResults] = useState<GeocodeResult[]>([]);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (q.trim().length < 3 || offline) {
      setResults([]);
      return;
    }
    const t = setTimeout(() => {
      api.geocode(q).then((r) => {
        setResults(r);
        setOpen(true);
      }).catch(() => setResults([]));
    }, 300);
    return () => clearTimeout(t);
  }, [q, offline]);
  return (
    <div className="search">
      <input
        type="search"
        placeholder={offline ? 'Recherche indisponible hors ligne' : 'Rechercher une adresse, une commune…'}
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onFocus={() => setOpen(true)}
        disabled={offline}
        aria-label="Recherche d'adresse"
      />
      {open && results.length > 0 && (
        <ul className="results" role="listbox">
          {results.map((r) => (
            <li key={`${r.label}-${r.lon}-${r.lat}`}>
              <button
                onClick={() => {
                  onSelect(r);
                  setOpen(false);
                  setQ(r.label);
                }}
              >
                <strong>{r.label}</strong>
                {r.context && <span className="muted"> · {r.context}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function EstimateCard({ est, loading }: { est: EstimateResponse; loading: boolean }) {
  const e = est.estimate;
  return (
    <div className={loading ? 'estimate loading' : 'estimate'}>
      <table>
        <tbody>
          {e.categories.map((c) => (
            <tr key={c.category}>
              <td>{CATEGORY_LABEL[c.category]}</td>
              <td className="num muted">{c.items.toLocaleString('fr-FR')}</td>
              <td className="num">{fmtBytes(c.bytes)}</td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <td>Téléchargement</td>
            <td className="num muted">{e.total_items.toLocaleString('fr-FR')}</td>
            <td className="num">
              <strong>{fmtBytes(e.total_bytes)}</strong>
            </td>
          </tr>
          <tr>
            <td colSpan={2}>Sur disque après finalisation</td>
            <td className="num">{fmtBytes(e.disk_bytes)}</td>
          </tr>
          <tr>
            <td colSpan={2}>Durée estimée</td>
            <td className="num">≈ {fmtDuration(e.est_seconds)}</td>
          </tr>
        </tfoot>
      </table>
      <p className="muted small">
        Anneaux LiDAR : {est.rings.map((r) => `${r.cell_m} m jusqu'à ${(r.half_size_m / 1000).toFixed(1)} km`).join(' · ')}
      </p>
    </div>
  );
}

function ProgressCard({ progress: p, onCancel }: { progress: PackProgress | null; onCancel: () => void }) {
  const pct = p && p.total_items > 0 ? (100 * p.done_items) / p.total_items : 0;
  const label =
    p?.stage === 'finalizing'
      ? `Finalisation : ${p.message}`
      : p
        ? `${p.done_items.toLocaleString('fr-FR')} / ${p.total_items.toLocaleString('fr-FR')} éléments · ${fmtBytes(p.bytes)}`
        : 'Préparation…';
  return (
    <section className="card">
      <h2>Téléchargement du pack</h2>
      <div className="bar" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
        <div style={{ width: `${p?.stage === 'finalizing' ? 100 : pct}%` }} className={p?.stage === 'finalizing' ? 'indeterminate' : ''} />
      </div>
      <p className="small">{label}</p>
      {p && p.stage === 'downloading' && (
        <p className="muted small">
          {fmtDuration(p.elapsed_s)} écoulées{p.failed_items > 0 && ` · ${p.failed_items} en échec (reprise possible)`}
        </p>
      )}
      <button className="ghost danger" onClick={onCancel}>
        Annuler (reprenable)
      </button>
    </section>
  );
}

function ZoneList({
  zones,
  busy,
  onOpen,
  onResume,
  onDelete,
}: {
  zones: ZoneSummary[];
  busy: boolean;
  onOpen: (id: string) => void;
  onResume: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  if (zones.length === 0) return null;
  return (
    <section className="card">
      <h2>Zones téléchargées</h2>
      <ul className="zones">
        {zones.map((z) => (
          <li key={z.id}>
            <div>
              <strong>{z.name}</strong>
              <span className="muted small">
                {' '}
                · {(z.radius_m / 1000).toFixed(1)} km · {fmtBytes(z.downloaded_bytes)}
              </span>
              <div>
                <span className={`chip ${z.status}`}>
                  {z.status === 'complete' ? 'complète' : z.status === 'downloaded' ? 'à finaliser' : `partielle ${z.done_items}/${z.total_items}`}
                </span>
              </div>
            </div>
            <div className="actions">
              {z.status === 'complete' ? (
                <button className="primary small" onClick={() => onOpen(z.id)} disabled={busy}>
                  Vue 3D
                </button>
              ) : (
                <button className="small" onClick={() => onResume(z.id)} disabled={busy}>
                  Reprendre
                </button>
              )}
              <button className="ghost small danger" onClick={() => onDelete(z.id)} disabled={busy} aria-label={`Supprimer ${z.name}`}>
                Suppr.
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
