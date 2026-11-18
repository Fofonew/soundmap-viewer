import * as maplibregl from 'maplibre-gl';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  api,
  DEFAULT_SITE,
  fmtBytes,
  fmtDuration,
  type AccessKind,
  type AppInfo,
  type EstimateResponse,
  type GeocodeResult,
  type ParcelInfo,
  type PoliceNear,
  type SiteSpec,
  type OpenZoneResponse,
  type PackProgress,
  type SpotProgress,
  type SpotReport,
  type ZoneRequest,
  type ZoneSummary,
} from './api';
import { boundsAround } from './geo';
import { Hint } from './Hint';
import { LANGUAGES, setLang, t as tr, useT, type TKey } from './i18n';
import { LevelHelper } from './LevelHelper';
import { sub18Equivalent } from './levelMath';
import { LayersPanel, ParcelInspector, type Basemap } from './MapOverlays';
import { exploreStyle, zoneFeatures, zoneStyle } from './mapStyles';
import { useOwnerLayer } from './ownerLayer';
import { SimPanel } from './sim/SimPanel';

type Mode = 'explore' | 'zone3d';

function todayLocal(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function errText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

const ACCESS_KEY: Record<AccessKind, TKey> = { route: 'site.access.route', piste: 'site.access.piste', chemin: 'site.access.chemin' };

/** Préférence d'affichage mémorisée (sans effet si le stockage est indisponible). */
function readPref(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === '1';
  } catch {
    return fallback;
  }
}

function writePref(key: string, value: boolean) {
  try {
    localStorage.setItem(key, value ? '1' : '0');
  } catch {
    // préférence non mémorisée
  }
}

function readSitePref(): SiteSpec {
  try {
    const v = localStorage.getItem('soundmap.site');
    return v ? { ...DEFAULT_SITE, ...(JSON.parse(v) as Partial<SiteSpec>) } : DEFAULT_SITE;
  } catch {
    return DEFAULT_SITE;
  }
}

function writeSitePref(s: SiteSpec) {
  try {
    localStorage.setItem('soundmap.site', JSON.stringify(s));
  } catch {
    // préférence non mémorisée
  }
}

/** Résumé des critères du site, à côté de la case à cocher. */
function siteSummary(s: SiteSpec): string {
  const ways = [s.access_route && tr('site.access.route'), s.access_piste && tr('site.access.piste'), s.access_chemin && tr('site.access.chemin_short')]
    .filter(Boolean)
    .join(', ');
  return tr('site.summary', { radius: s.clear_radius_m, ways: ways || tr('site.noWay'), access: s.max_access_m });
}

/** Marque : barres de vumètre aux couleurs de l'échelle des dB. */
function Logo() {
  return (
    <svg className="logo" viewBox="0 0 28 28" aria-hidden>
      {[
        ['#57b45a', 15],
        ['#f3e45a', 11],
        ['#ef7d2d', 7],
        ['#c41f3a', 3],
      ].map(([c, y], i) => (
        <rect key={i} x={3 + i * 6} y={y as number} width={4} height={25 - (y as number)} rx={1} fill={c as string} />
      ))}
    </svg>
  );
}

export function App() {
  const { t, num, lang } = useT();
  const mapDiv = useRef<HTMLDivElement>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [mapObj, setMapObj] = useState<maplibregl.Map | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [mode, setMode] = useState<Mode>('explore');
  const [basemap, setBasemap] = useState<Basemap>('ortho');
  const [lidar, setLidar] = useState(false);
  const [names, setNames] = useState(() => readPref('soundmap.names', true));
  const [showPolice, setShowPolice] = useState(() => readPref('soundmap.police', false));
  const [police, setPolice] = useState<{ at: [number, number]; list: PoliceNear[] } | null>(null);
  const [policeErr, setPoliceErr] = useState<string | null>(null);
  const policeHover = useRef<maplibregl.Map | null>(null);
  const [parcel, setParcel] = useState<ParcelInfo | null>(null);
  const [parcelBusy, setParcelBusy] = useState(false);
  const [parcelErr, setParcelErr] = useState<string | null>(null);
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
  const [spotCheckSite, setSpotCheckSite] = useState(true);
  const [site, setSiteState] = useState<SiteSpec>(readSitePref);
  const setSite = (patch: Partial<SiteSpec>) =>
    setSiteState((cur) => {
      const next = { ...cur, ...patch };
      writeSitePref(next);
      return next;
    });
  const [showParcels, setShowParcels] = useState(() => readPref('soundmap.parcels', false));
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
    setMapObj(map);
    if (import.meta.env.DEV) (window as unknown as { __map: maplibregl.Map }).__map = map;
    api.appInfo().then(setInfo).catch(() => undefined);
    refreshZones();
    return () => {
      map.remove();
      mapRef.current = null;
      setMapObj(null);
    };
  }, [refreshZones]);

  useEffect(() => {
    pickingRef.current = picking;
    const c = mapRef.current?.getCanvas();
    if (c) c.style.cursor = picking ? 'crosshair' : '';
  }, [picking]);

  // Cadastre : parcelle au point, propriétaire personne morale, forêt publique.
  const queryParcel = useCallback(async (at: [number, number]) => {
    setParcelBusy(true);
    setParcelErr(null);
    try {
      setParcel(await api.parcelInfo(at[0], at[1]));
    } catch (e) {
      setParcelErr(errText(e));
    } finally {
      setParcelBusy(false);
    }
  }, []);
  const owners = useOwnerLayer(
    mapObj,
    showParcels && mode === 'explore' && !info?.offline,
    () => !pickingRef.current,
    (at) => void queryParcel(at),
  );
  const closeInspector = () => {
    setParcel(null);
    setParcelErr(null);
    owners.clearSelection();
  };
  useEffect(() => {
    const map = mapRef.current;
    if (!map || mode !== 'explore') return;
    const apply = () => {
      const ring = parcel?.parcel?.ring;
      const data: GeoJSON.FeatureCollection = {
        type: 'FeatureCollection',
        features: ring ? [{ type: 'Feature', properties: {}, geometry: { type: 'Polygon', coordinates: [ring] } }] : [],
      };
      const src = map.getSource('parcel') as maplibregl.GeoJSONSource | undefined;
      if (src) {
        src.setData(data);
        return;
      }
      map.addSource('parcel', { type: 'geojson', data });
      map.addLayer({ id: 'parcel-line', type: 'line', source: 'parcel', paint: { 'line-color': '#ffffff', 'line-width': 3 } });
    };
    if (map.getLayer('ortho')) apply();
    else map.once('style.load', apply);
  }, [parcel, mode]);

  // Fond de carte et calques (mode Exploration).
  const applyExploreLayers = useCallback(() => {
    const map = mapRef.current;
    if (!map || !map.getLayer('ortho')) return;
    map.setLayoutProperty('ortho', 'visibility', basemap === 'ortho' ? 'visible' : 'none');
    map.setLayoutProperty('plan', 'visibility', basemap === 'plan' ? 'visible' : 'none');
    // Le Plan IGN porte déjà ses toponymes : la surcouche ne sert que sur le satellite.
    map.setLayoutProperty('names', 'visibility', basemap === 'ortho' && names ? 'visible' : 'none');
    for (const l of ['lidar-fill', 'lidar-line']) map.setLayoutProperty(l, 'visibility', lidar ? 'visible' : 'none');
    for (const l of ['pci-fill', 'pci-line']) map.setLayoutProperty(l, 'visibility', showParcels ? 'visible' : 'none');
  }, [basemap, lidar, names, showParcels]);

  const applyZoneOverlay = useCallback(() => {
    const src = mapRef.current?.getSource('zone') as maplibregl.GeoJSONSource | undefined;
    if (!src || mode !== 'explore') return;
    src.setData(center ? zoneFeatures(center[0], center[1], radius, est?.rings.slice(0, -1) ?? []) : { type: 'FeatureCollection', features: [] });
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
    const fallback = tr('zone.defaultName', { lat: center[1].toFixed(3), lon: center[0].toFixed(3) });
    return { name: name.trim() || fallback, lon: center[0], lat: center[1], radius_m: radius, meteo_date: date };
  }, [center, name, radius, date]);

  useEffect(() => {
    const req = request();
    if (!req) return;
    setEstimating(true);
    const h = setTimeout(() => {
      api
        .estimateZone(req)
        .then((r) => {
          setEst(r);
          setError(null);
        })
        .catch((e) => setError(errText(e)))
        .finally(() => setEstimating(false));
    }, 250);
    return () => clearTimeout(h);
  }, [request]);

  const runJob = useCallback(
    (id: string, p: Promise<ZoneSummary>) => {
      setJob(id);
      setError(null);
      p.then(() => refreshZones())
        .catch((e) => {
          // Une annulation volontaire n'est pas une erreur : la zone reste reprenable.
          const m = errText(e);
          if (!/annulée|cancel/i.test(m)) setError(m);
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
        {
          radius_m: spotRadius,
          grid_m: 500,
          source_dbc_1m: spotLevel,
          threshold_dbc: spotThreshold,
          max_slope_pct: 8,
          influence_m: 6000,
          check_site: spotCheckSite,
          site,
        },
        (p) => setSpotJob(p),
      );
      setSpotReport(r);
      const b = new maplibregl.LngLatBounds();
      for (const s of r.spots) b.extend([s.lon, s.lat]);
      if (!b.isEmpty()) map.fitBounds(b, { padding: 80, maxZoom: 12 });
    } catch (e) {
      const m = errText(e);
      if (!/annul|cancel/i.test(m)) setError(m);
    } finally {
      setSpotJob(null);
    }
  };

  // Gendarmeries et commissariats autour de la sono (ou du centre de la carte).
  const loadPolice = useCallback(async (at: [number, number]) => {
    setPoliceErr(null);
    try {
      setPolice({ at, list: await api.nearbyPolice(at[0], at[1], 40_000) });
    } catch (e) {
      setPoliceErr(errText(e));
    }
  }, []);
  useEffect(() => {
    const map = mapRef.current;
    if (!showPolice || !map || mode !== 'explore') return;
    const at: [number, number] = center ?? [map.getCenter().lng, map.getCenter().lat];
    if (police && Math.abs(police.at[0] - at[0]) < 1e-6 && Math.abs(police.at[1] - at[1]) < 1e-6) return;
    void loadPolice(at);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showPolice, center, mode, loadPolice]);
  // Sans sono placée : liste recalculée autour du centre de la carte après chaque déplacement.
  useEffect(() => {
    const map = mapObj;
    if (!map || !showPolice || center || mode !== 'explore') return;
    let h: ReturnType<typeof setTimeout> | undefined;
    const onMove = () => {
      clearTimeout(h);
      h = setTimeout(() => {
        if (map.getZoom() >= 8) void loadPolice([map.getCenter().lng, map.getCenter().lat]);
      }, 600);
    };
    map.on('moveend', onMove);
    return () => {
      clearTimeout(h);
      map.off('moveend', onMove);
    };
  }, [mapObj, showPolice, center, mode, loadPolice]);
  useEffect(() => {
    const map = mapRef.current;
    if (!map || mode !== 'explore') return;
    const apply = () => {
      const data: GeoJSON.FeatureCollection = {
        type: 'FeatureCollection',
        features: (showPolice ? (police?.list ?? []) : []).map((p) => ({
          type: 'Feature' as const,
          properties: { name: p.name, gn: p.nature === 'Gendarmerie' ? 1 : 0, d: p.distance_m },
          geometry: { type: 'Point' as const, coordinates: [p.lon, p.lat] },
        })),
      };
      const src = map.getSource('police') as maplibregl.GeoJSONSource | undefined;
      if (src) {
        src.setData(data);
        return;
      }
      map.addSource('police', { type: 'geojson', data });
      map.addLayer({
        id: 'police',
        type: 'circle',
        source: 'police',
        paint: {
          'circle-radius': ['interpolate', ['linear'], ['zoom'], 8, 5, 14, 9],
          'circle-color': ['match', ['get', 'gn'], 1, '#2f5bd8', '#29a3d6'],
          'circle-stroke-color': '#ffffff',
          'circle-stroke-width': 2,
        },
      });
      // Les écouteurs par couche survivent aux changements de style : une fois par carte.
      if (policeHover.current === map) return;
      policeHover.current = map;
      const pop = new maplibregl.Popup({ closeButton: false, closeOnClick: false, className: 'hover-pop', offset: 12 });
      map.on('mouseenter', 'police', (e) => {
        const f = e.features?.[0];
        if (!f) return;
        const el = document.createElement('div');
        const title = document.createElement('div');
        title.className = 'hp-title';
        title.textContent = String(f.properties?.name ?? '');
        const d = document.createElement('div');
        d.className = 'hp-sub';
        d.textContent = tr('layers.policeDistance', { km: Number((Number(f.properties?.d) / 1000).toFixed(1)) });
        el.append(title, d);
        pop.setLngLat(e.lngLat).setDOMContent(el).addTo(map);
      });
      map.on('mouseleave', 'police', () => pop.remove());
    };
    if (map.getLayer('ortho')) apply();
    else map.once('style.load', apply);
  }, [police, showPolice, mode]);

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
          'circle-color': ['interpolate', ['linear'], ['get', 'e'], 0, '#57b45a', 20, '#f3e45a', 80, '#ef7d2d', 200, '#c41f3a'],
          'circle-opacity': 0.8,
        },
      });
      map.addLayer({
        id: 'spots-top',
        type: 'circle',
        source: 'spots',
        filter: ['==', ['get', 'top'], 1],
        paint: { 'circle-radius': 11, 'circle-color': '#0d0f12', 'circle-stroke-color': '#ff7a1a', 'circle-stroke-width': 2 },
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
      closeInspector();
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
  const offline = !!info?.offline;
  const subs = sub18Equivalent(spotLevel);

  return (
    <div className="app">
      <aside className="panel">
        <header className="brand">
          <Logo />
          <div className="brand-text">
            <h1>SoundMap</h1>
            <p>{mode === 'explore' ? t('app.modeExplore') : t('app.modeZone')}</p>
          </div>
          {offline && <span className="chip warn">{t('app.offline')}</span>}
          <select className="lang" value={lang} onChange={(e) => setLang(e.target.value)} aria-label={t('app.language')}>
            {LANGUAGES.map((l) => (
              <option key={l.code} value={l.code}>
                {l.code.toUpperCase()}
              </option>
            ))}
          </select>
        </header>

        {error && (
          <div className="alert" role="alert">
            <span>{error}</span>
            <button className="icon-btn" onClick={() => setError(null)} aria-label={t('common.close')}>
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
              offline={offline}
            />

            <section className="card" data-step="01">
              <h2>
                {t('spots.title')}{' '}
                <Hint label={t('spots.title')}>
                  <p>{t('spots.how1')}</p>
                  <p className="muted">{t('spots.how2')}</p>
                </Hint>
              </h2>
              <p className="lede">{center ? t('spots.ledeSono') : t('spots.ledeMap')}</p>
              <div className="fields spot-fields">
                <label className="field">
                  <span>
                    {t('spots.radius')} <Hint>{t('spots.radiusHint')}</Hint>
                  </span>
                  <select value={spotRadius} onChange={(e) => setSpotRadius(Number(e.target.value))}>
                    {[10000, 20000, 30000].map((r) => (
                      <option key={r} value={r}>
                        {r / 1000} km
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>
                    {t('spots.level')}{' '}
                    <Hint label={t('spots.level')}>
                      <p>{t('spots.levelHint1')}</p>
                      <p>{t('spots.levelHint2')}</p>
                      <p>{t('spots.levelHint3')}</p>
                    </Hint>
                  </span>
                  <span className="input-unit">
                    <input
                      type="number"
                      list="spot-levels"
                      min={100}
                      max={150}
                      step={1}
                      value={spotLevel}
                      onChange={(e) => setSpotLevel(Number(e.target.value))}
                    />
                    <em>dB(C)</em>
                  </span>
                  <datalist id="spot-levels">
                    {[1, 2, 4, 8].map((n) => (
                      <option key={n} value={119 + Math.round(20 * Math.log10(n))} label={t('spots.subs18', { count: n })} />
                    ))}
                  </datalist>
                </label>
                <label className="field">
                  <span>
                    {t('spots.threshold')}{' '}
                    <Hint>
                      <p>{t('spots.thresholdHint1')}</p>
                      <p>{t('spots.thresholdHint2')}</p>
                    </Hint>
                  </span>
                  <span className="input-unit">
                    <input type="number" min={30} max={80} value={spotThreshold} onChange={(e) => setSpotThreshold(Number(e.target.value))} />
                    <em>dB(C)</em>
                  </span>
                </label>
              </div>
              <p className="readout small">
                {t('spots.levelEquiv', {
                  level: spotLevel,
                  subs: subs < 1.5 ? t('spots.subs18', { count: 1 }) : t('spots.subs18', { count: Number(subs < 10 ? subs.toFixed(1) : Math.round(subs)) }),
                })}
              </p>
              <LevelHelper onUse={setSpotLevel} />

              <label className="check">
                <input type="checkbox" checked={spotCheckSite} onChange={(e) => setSpotCheckSite(e.target.checked)} />
                {t('site.check')} <span className="muted">— {siteSummary(site)}</span>
              </label>
              {spotCheckSite && <SiteCriteria site={site} setSite={setSite} />}

              {spotJob ? (
                <div className="row">
                  <span className="small">
                    {spotJob === 'start' ? t('common.starting') : `${spotJob.message} (${spotJob.done}/${spotJob.total})`}
                  </span>
                  <button className="ghost danger" onClick={() => api.cancelJob('spots')}>
                    {t('common.cancel')}
                  </button>
                </div>
              ) : (
                <button className="primary" onClick={runSpots} disabled={busy}>
                  {t('spots.run')}
                </button>
              )}
              {spotReport && (
                <>
                  <p className="readout small">
                    {t('spots.report', {
                      candidates: spotReport.candidates.length,
                      people: Math.round(spotReport.population),
                      seconds: Number(spotReport.seconds.toFixed(1)),
                    })}
                    {spotReport.site_checked > 0 &&
                      ` · ${t('spots.reportSite', {
                        checked: spotReport.site_checked,
                        wooded: spotReport.rejected_wooded,
                        noAccess: spotReport.rejected_access,
                      })}`}
                  </p>
                  {spotReport.spots.length === 0 && <p className="small muted">{t('spots.none')}</p>}
                  <ol className="spot-list">
                    {spotReport.spots.map((s, i) => (
                      <li key={i}>
                        <button
                          onClick={() => {
                            setCenter([s.lon, s.lat]);
                            mapRef.current?.flyTo({ center: [s.lon, s.lat], zoom: 15 });
                            owners.clearSelection();
                            if (!offline) void queryParcel([s.lon, s.lat]);
                          }}
                        >
                          <span className="spot-rank">{i + 1}</span>
                          <span className="spot-main">
                            <span className="spot-head">
                              <strong>{t('spots.exposed', { count: Math.round(s.exposed) })}</strong>
                              <span className="num">{num(s.max_dbc, 0)} dB(C) max</span>
                            </span>
                            <span className="spot-meta">
                              {t('spots.meta', {
                                nearest: num(s.nearest_m / 1000, 1),
                                slope: num(s.slope_pct, 1),
                                alt: Math.round(s.elevation_m),
                              })}
                            </span>
                            {s.site && (
                              <span className="spot-meta">
                                {t('spots.access', {
                                  kind: t(ACCESS_KEY[s.site.access]) + (s.site.access_restricted ? ` (${t('spots.restricted')})` : ''),
                                  dist: Math.round(s.site.access_m),
                                  wooded: Math.round(s.site.wooded_pct),
                                })}
                              </span>
                            )}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ol>
                  {spotReport.spots.length > 0 && <p className="small muted">{t('spots.clickHint')}</p>}
                </>
              )}
            </section>

            <section className="card" data-step="02">
              <h2>{t('zone.title')}</h2>
              <button className={picking ? 'primary pulse' : center ? '' : 'primary'} onClick={() => setPicking((p) => !p)} disabled={busy}>
                {picking ? t('zone.picking') : center ? t('zone.move') : t('zone.place')}
              </button>
              {center && (
                <>
                  <p className="readout small">
                    {num(center[1], 5)}° N · {num(center[0], 5)}° E
                  </p>
                  <label className="field">
                    <span>
                      {t('zone.radius')} <strong className="num">{num(radius / 1000, 1)} km</strong>
                    </span>
                    <input type="range" min={1000} max={10000} step={500} value={radius} onChange={(e) => setRadius(Number(e.target.value))} disabled={busy} />
                  </label>
                  <div className="fields">
                    <label className="field">
                      <span>{t('zone.name')}</span>
                      <input type="text" value={name} placeholder={t('zone.namePlaceholder')} onChange={(e) => setName(e.target.value)} disabled={busy} />
                    </label>
                    <label className="field">
                      <span>
                        {t('zone.date')} <Hint>{t('zone.dateHint')}</Hint>
                      </span>
                      <input type="date" value={date} onChange={(e) => setDate(e.target.value)} disabled={busy} />
                    </label>
                  </div>
                  {est && <EstimateCard est={est} loading={estimating} />}
                  {est &&
                    !busy &&
                    (existing?.status === 'complete' ? (
                      <button className="primary" onClick={() => open3d(est.id)}>
                        {t('zone.open3d')}
                      </button>
                    ) : (
                      <button className="primary" onClick={startDownload} disabled={estimating}>
                        {existing ? t('zone.resume') : t('zone.download')}
                      </button>
                    ))}
                </>
              )}
              {busy && <ProgressBlock progress={progress} onCancel={() => job && api.cancelDownload(job)} />}
            </section>

            <ZoneList zones={zones} busy={busy} onOpen={open3d} onResume={resume} onDelete={deleteZone} />
          </>
        ) : (
          opened &&
          mapRef.current && (
            <>
              <SimPanel key={opened.summary.id} map={mapRef.current} zone={opened} onBack={backToExplore} />
              {openTiming && (
                <p className="readout small muted">
                  {t('app.openTiming', { pack: Math.round(openTiming.rust), idle: Math.round(openTiming.idle) })}
                </p>
              )}
            </>
          )
        )}

        <footer className="muted small">{t('app.credits')}</footer>
      </aside>
      <div className="map-wrap">
        <main ref={mapDiv} className="map" />
        {mode === 'explore' && (
          <>
            <LayersPanel
              basemap={basemap}
              setBasemap={setBasemap}
              names={names}
              setNames={(v) => {
                setNames(v);
                writePref('soundmap.names', v);
              }}
              parcels={showParcels}
              setParcels={(v) => {
                setShowParcels(v);
                writePref('soundmap.parcels', v);
                if (!v) closeInspector();
              }}
              parcelsStatus={owners}
              police={showPolice}
              setPolice={(v) => {
                setShowPolice(v);
                writePref('soundmap.police', v);
              }}
              policeList={police?.list ?? null}
              policeErr={policeErr}
              policeFrom={center ? t('layers.fromSono') : t('layers.fromMap')}
              lidar={lidar}
              setLidar={setLidar}
              offline={offline}
              zoomTo={(z) => mapRef.current?.easeTo({ zoom: z })}
              flyTo={(lon, lat) => mapRef.current?.flyTo({ center: [lon, lat], zoom: 14 })}
            />
            <ParcelInspector parcel={parcel} busy={parcelBusy} error={parcelErr} selected={owners.selected} onClose={closeInspector} />
          </>
        )}
      </div>
    </div>
  );
}

function SiteCriteria({ site, setSite }: { site: SiteSpec; setSite: (p: Partial<SiteSpec>) => void }) {
  const { t } = useT();
  const sel = (key: 'clear_radius_m' | 'max_access_m' | 'search_m', values: number[], zero?: string) => (
    <select value={site[key]} onChange={(e) => setSite({ [key]: Number(e.target.value) })}>
      {values.map((v) => (
        <option key={v} value={v}>
          {v === 0 && zero ? zero : `${v} m`}
        </option>
      ))}
    </select>
  );
  const check = (key: keyof SiteSpec, label: TKey, note?: TKey) => (
    <label className="check">
      <input type="checkbox" checked={site[key] as boolean} onChange={(e) => setSite({ [key]: e.target.checked })} />
      {t(label)} {note && <span className="muted">{t(note)}</span>}
    </label>
  );
  return (
    <details className="sub">
      <summary>{t('site.criteria')}</summary>
      <div>
        <div className="fields">
          <label className="field">
            <span>
              {t('site.clearing')} <Hint>{t('site.clearingHint')}</Hint>
            </span>
            {sel('clear_radius_m', [25, 50, 75, 100, 150, 200])}
          </label>
          <label className="field">
            <span>
              {t('site.accessMax')} <Hint>{t('site.accessMaxHint')}</Hint>
            </span>
            {sel('max_access_m', [50, 100, 200, 300, 500, 1000])}
          </label>
          <label className="field">
            <span>
              {t('site.shift')} <Hint>{t('site.shiftHint')}</Hint>
            </span>
            {sel('search_m', [0, 100, 250, 500, 1000], t('site.shiftNone'))}
          </label>
        </div>
        <p className="sublabel">
          {t('site.ways')} <Hint>{t('site.waysHint')}</Hint>
        </p>
        <div className="checks">
          {check('access_route', 'site.wayRoute')}
          {check('access_piste', 'site.wayPiste', 'site.wayPisteNote')}
          {check('access_chemin', 'site.wayChemin', 'site.wayCheminNote')}
          {check('allow_restricted', 'site.wayRestricted', 'site.wayRestrictedNote')}
        </div>
        <p className="sublabel">
          {t('site.vegetation')}{' '}
          <Hint label={t('site.vegetation')}>
            <p>{t('site.vegetationHint1')}</p>
            <p>{t('site.vegetationHint2')}</p>
          </Hint>
        </p>
        <p className="small muted">{t('site.forestAlways')}</p>
        <div className="checks">
          {check('block_open_forest', 'site.openForest', 'site.openForestNote')}
          {check('block_crops', 'site.crops')}
          {check('block_scrub', 'site.scrub')}
        </div>
        <button className="link small" onClick={() => setSite(DEFAULT_SITE)}>
          {t('site.reset')}
        </button>
      </div>
    </details>
  );
}

function SearchBox({ onSelect, offline }: { onSelect: (r: GeocodeResult) => void; offline: boolean }) {
  const { t } = useT();
  const [q, setQ] = useState('');
  const [results, setResults] = useState<GeocodeResult[]>([]);
  const [open, setOpen] = useState(false);
  // Libellé choisi dans la liste : ne relance pas de recherche (sinon la liste se rouvre).
  const picked = useRef<string | null>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);
  useEffect(() => {
    if (q.trim().length < 3 || offline) {
      setResults([]);
      return;
    }
    if (q === picked.current) return;
    const h = setTimeout(() => {
      api
        .geocode(q)
        .then((r) => {
          setResults(r);
          setOpen(true);
        })
        .catch(() => setResults([]));
    }, 300);
    return () => clearTimeout(h);
  }, [q, offline]);
  const choose = (r: GeocodeResult) => {
    picked.current = r.label;
    setQ(r.label);
    setOpen(false);
    onSelect(r);
  };
  return (
    <div className="search" ref={box}>
      <input
        type="search"
        placeholder={offline ? t('search.offline') : t('search.placeholder')}
        value={q}
        onChange={(e) => {
          picked.current = null;
          setQ(e.target.value);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') setOpen(false);
          else if (e.key === 'Enter' && results[0]) choose(results[0]);
        }}
        disabled={offline}
        aria-label={t('search.label')}
      />
      {open && results.length > 0 && (
        <ul className="results" role="listbox">
          {results.map((r) => (
            <li key={`${r.label}-${r.lon}-${r.lat}`}>
              <button onClick={() => choose(r)}>
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
  const { t, num } = useT();
  const e = est.estimate;
  return (
    <div className={loading ? 'estimate loading' : 'estimate'}>
      <table>
        <tbody>
          {e.categories.map((c) => (
            <tr key={c.category}>
              <td>{t(`estimate.cat.${c.category}`)}</td>
              <td className="num muted">{num(c.items, 0)}</td>
              <td className="num">{fmtBytes(c.bytes)}</td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <td>{t('estimate.download')}</td>
            <td className="num muted">{num(e.total_items, 0)}</td>
            <td className="num">
              <strong>{fmtBytes(e.total_bytes)}</strong>
            </td>
          </tr>
          <tr>
            <td colSpan={2}>{t('estimate.disk')}</td>
            <td className="num">{fmtBytes(e.disk_bytes)}</td>
          </tr>
          <tr>
            <td colSpan={2}>{t('estimate.duration')}</td>
            <td className="num">≈ {fmtDuration(e.est_seconds)}</td>
          </tr>
        </tfoot>
      </table>
      <p className="muted small">
        {t('estimate.rings')}{' '}
        {est.rings.map((r) => t('estimate.ring', { cell: r.cell_m, km: num(r.half_size_m / 1000, 1) })).join(' · ')}
      </p>
    </div>
  );
}

function ProgressBlock({ progress: p, onCancel }: { progress: PackProgress | null; onCancel: () => void }) {
  const { t, num } = useT();
  const pct = p && p.total_items > 0 ? (100 * p.done_items) / p.total_items : 0;
  const label =
    p?.stage === 'finalizing'
      ? t('progress.finalizing', { message: p.message })
      : p
        ? t('progress.items', { done: num(p.done_items, 0), total: num(p.total_items, 0), bytes: fmtBytes(p.bytes) })
        : t('progress.preparing');
  return (
    <div className="progress">
      <div className="bar" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
        <div style={{ width: `${p?.stage === 'finalizing' ? 100 : pct}%` }} className={p?.stage === 'finalizing' ? 'indeterminate' : ''} />
      </div>
      <p className="small readout">{label}</p>
      {p && p.stage === 'downloading' && (
        <p className="muted small">
          {t('progress.elapsed', { time: fmtDuration(p.elapsed_s) })}
          {p.failed_items > 0 && ` · ${t('progress.failed', { count: p.failed_items })}`}
        </p>
      )}
      <button className="ghost danger" onClick={onCancel}>
        {t('progress.cancel')}
      </button>
    </div>
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
  const { t, num } = useT();
  if (zones.length === 0) return null;
  return (
    <section className="card" data-step="03">
      <h2>{t('zones.title')}</h2>
      <ul className="zones">
        {zones.map((z) => (
          <li key={z.id}>
            <div>
              <strong>{z.name}</strong>
              <div className="readout small muted">
                {num(z.radius_m / 1000, 1)} km · {fmtBytes(z.downloaded_bytes)}{' '}
                <span className={`chip ${z.status}`}>
                  {z.status === 'complete'
                    ? t('zones.complete')
                    : z.status === 'downloaded'
                      ? t('zones.toFinalize')
                      : t('zones.partial', { done: z.done_items, total: z.total_items })}
                </span>
              </div>
            </div>
            <div className="actions">
              {z.status === 'complete' ? (
                <button className="primary small" onClick={() => onOpen(z.id)} disabled={busy}>
                  {t('zones.open')}
                </button>
              ) : (
                <button className="small" onClick={() => onResume(z.id)} disabled={busy}>
                  {t('zones.resume')}
                </button>
              )}
              <button className="ghost small danger" onClick={() => onDelete(z.id)} disabled={busy} aria-label={t('zones.deleteLabel', { name: z.name })}>
                {t('zones.delete')}
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
