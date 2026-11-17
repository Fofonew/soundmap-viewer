import * as maplibregl from 'maplibre-gl';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { PROTOCOL_BASE as B, fmtDuration, type OpenZoneResponse } from '../api';
import {
  METRICS,
  PALETTE,
  defaultSettings,
  freePartySystem,
  newId,
  offsetLonLat,
  simApi,
  type Auralization,
  type CalibrationReport,
  type Candidate,
  type Catalog,
  type ImpactReport,
  type JobProgress,
  type Layer,
  type LowFreqResult,
  type PreviewResult,
  type Project,
  type SceneInfo,
  type Section,
  type SizingReport,
  type SpeakerModel,
  type SpeakerSpec,
} from './simApi';
import { SectionChart } from './SectionChart';

type Tab = 'sono' | 'carte' | 'bf' | 'impact' | 'dim' | 'projet';
type Tool = 'none' | 'place' | 'dancefloor' | 'section';

const TABS: [Tab, string][] = [
  ['sono', 'Sono'],
  ['carte', 'Carte'],
  ['bf', 'Basses fréq.'],
  ['impact', 'Impact'],
  ['dim', 'Dimens.'],
  ['projet', 'Projet'],
];

const KIND_LABEL = { sub: 'Sub', top: 'Tête', line_array: 'Line array' } as const;

function errText(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

function n1(v: number | null | undefined, d = 1): string {
  return v === null || v === undefined || !Number.isFinite(v) ? '—' : v.toFixed(d);
}

function signed(v: number, d = 1): string {
  return `${v >= 0 ? '+' : ''}${v.toFixed(d)}`;
}

/** Expression MapLibre de couleur par niveau (palette décalée). */
function colorExpr(offset: number): maplibregl.ExpressionSpecification {
  const e: unknown[] = ['step', ['get', 'db'], '#d4f2d4'];
  for (const [t, c] of PALETTE.slice(1)) e.push(t + offset, c);
  return e as maplibregl.ExpressionSpecification;
}

export function SimPanel({ map, zone, onBack }: { map: maplibregl.Map; zone: OpenZoneResponse; onBack: () => void }) {
  const zoneId = zone.summary.id;
  const [center] = useState<[number, number]>(zone.products.center_lonlat);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [scene, setScene] = useState<SceneInfo | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [tab, setTab] = useState<Tab>('sono');
  const [tool, setTool] = useState<Tool>('none');
  const [placeModel, setPlaceModel] = useState<string>('sub18');
  const [selected, setSelected] = useState<string | null>(null);
  const [moveAll, setMoveAll] = useState(false);
  const [draftFloor, setDraftFloor] = useState<[number, number][]>([]);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [lowfreq, setLowfreq] = useState<LowFreqResult | null>(null);
  const [lfStale, setLfStale] = useState(false);
  const [useLf, setUseLf] = useState(false);
  const [metric, setMetric] = useState('A');
  const [variant, setVariant] = useState('main');
  const [opacity, setOpacity] = useState(0.65);
  const [showIso, setShowIso] = useState(true);
  const [layer, setLayer] = useState<Layer | null>(null);
  const [section, setSection] = useState<Section | null>(null);
  const [sectionPt, setSectionPt] = useState<[number, number] | null>(null);
  const [job, setJob] = useState<{ kind: string; p: JobProgress | null } | null>(null);
  const [impact, setImpact] = useState<ImpactReport | null>(null);
  const [showImpact, setShowImpact] = useState(true);
  const [sizing, setSizing] = useState<SizingReport | null>(null);
  const [candidates, setCandidates] = useState<{ list: Candidate[]; seconds: number } | null>(null);
  const [calib, setCalib] = useState<CalibrationReport | null>(null);
  const [vegRadius, setVegRadius] = useState(1000);
  const [aural, setAural] = useState<Auralization | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const resultId = useLf && lowfreq ? lowfreq.result_id : (preview?.result_id ?? null);

  // ---------- Chargement ----------
  useEffect(() => {
    let alive = true;
    Promise.all([simApi.catalog(), simApi.loadScene(zoneId), simApi.autoload(zoneId)])
      .then(([c, s, p]) => {
        if (!alive) return;
        setCatalog(c);
        setScene(s);
        setProject(p ?? { version: 1, name: zone.spec.name, zone_id: zoneId, speakers: [], dancefloor: null, settings: defaultSettings() });
      })
      .catch((e) => setError(errText(e)));
    return () => {
      alive = false;
    };
  }, [zoneId, zone.spec.name]);

  // ---------- Couches de la carte ----------
  const layersReady = useRef(false);
  const ensureLayers = useCallback(() => {
    if (layersReady.current || !map.getSource('zone')) return layersReady.current;
    const empty: GeoJSON.FeatureCollection = { type: 'FeatureCollection', features: [] };
    for (const id of ['iso', 'df', 'imp', 'sec', 'cal']) map.addSource(id, { type: 'geojson', data: empty });
    map.addLayer({ id: 'iso', type: 'line', source: 'iso', paint: { 'line-color': '#ffffff', 'line-width': 1.2, 'line-opacity': 0.9 } });
    map.addLayer({ id: 'df-fill', type: 'fill', source: 'df', paint: { 'fill-color': '#ff7a1a', 'fill-opacity': 0.25 } });
    map.addLayer({ id: 'df-line', type: 'line', source: 'df', paint: { 'line-color': '#ff7a1a', 'line-width': 2 } });
    map.addLayer({
      id: 'imp',
      type: 'circle',
      source: 'imp',
      paint: {
        'circle-radius': ['interpolate', ['linear'], ['zoom'], 12, 3, 17, 8],
        'circle-color': ['match', ['get', 'class'], 0, '#4cc38a', 1, '#f5b84a', 2, '#ef7d2d', '#e4302c'],
        'circle-stroke-color': '#14161a',
        'circle-stroke-width': 1,
      },
    });
    map.addLayer({ id: 'sec', type: 'line', source: 'sec', paint: { 'line-color': '#00e5ff', 'line-width': 2, 'line-dasharray': [2, 1] } });
    // Mesures de calage : couleur selon l'écart mesure − modèle calé (dB(A)).
    map.addLayer({
      id: 'cal',
      type: 'circle',
      source: 'cal',
      paint: {
        'circle-radius': ['interpolate', ['linear'], ['zoom'], 12, 4, 17, 9],
        'circle-color': ['interpolate', ['linear'], ['get', 'r'], -6, '#3b82f6', 0, '#f4f4f5', 6, '#e4302c'],
        'circle-stroke-color': '#14161a',
        'circle-stroke-width': 1.5,
      },
    });
    layersReady.current = true;
    return true;
  }, [map]);

  useEffect(() => {
    if (!ensureLayers()) {
      const on = () => ensureLayers();
      map.on('styledata', on);
      return () => {
        map.off('styledata', on);
      };
    }
  }, [map, ensureLayers]);

  // ---------- Aperçu (ingénierie), recalculé à chaque modification ----------
  const computing = useRef(false);
  const pending = useRef<Project | null>(null);
  const runPreview = useCallback(
    (p: Project) => {
      if (p.speakers.filter((s) => !s.muted).length === 0) return;
      if (computing.current) {
        pending.current = p;
        return;
      }
      computing.current = true;
      simApi
        .preview(p)
        .then((r) => {
          setPreview(r);
          setError(null);
        })
        .catch((e) => setError(errText(e)))
        .finally(() => {
          computing.current = false;
          const next = pending.current;
          pending.current = null;
          if (next) runPreview(next);
        });
    },
    [],
  );

  useEffect(() => {
    if (!project) return;
    const t = setTimeout(() => runPreview(project), 120);
    const s = setTimeout(() => simApi.autosave(project).catch(() => undefined), 1000);
    return () => {
      clearTimeout(t);
      clearTimeout(s);
    };
  }, [project, runPreview]);

  const update = useCallback((f: (p: Project) => Project) => {
    setProject((p) => (p ? f(p) : p));
    setLfStale(true);
    setUseLf(false);
  }, []);

  // ---------- Couche de niveaux ----------
  useEffect(() => {
    if (!resultId || !ensureLayers()) return;
    let alive = true;
    const v = useLf ? variant : 'main';
    simApi
      .layer(resultId, v, metric)
      .then((l) => {
        if (!alive) return;
        setLayer(l);
        const url = `${B}/result/${resultId}/${v}/${metric}.png`;
        const coords = l.corners as [[number, number], [number, number], [number, number], [number, number]];
        const src = map.getSource('spl') as maplibregl.ImageSource | undefined;
        if (src) src.updateImage({ url, coordinates: coords });
        else {
          map.addSource('spl', { type: 'image', url, coordinates: coords });
          map.addLayer(
            { id: 'spl', type: 'raster', source: 'spl', paint: { 'raster-opacity': opacity, 'raster-fade-duration': 0 } },
            'buildings-3d',
          );
        }
        (map.getSource('iso') as maplibregl.GeoJSONSource | undefined)?.setData(l.isophones);
        if (map.getLayer('iso')) map.setPaintProperty('iso', 'line-color', colorExpr(l.palette_offset));
      })
      .catch((e) => setError(errText(e)));
    return () => {
      alive = false;
    };
    // L'opacité est appliquée par un effet séparé.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resultId, metric, variant, useLf, map, ensureLayers]);

  useEffect(() => {
    if (map.getLayer('spl')) map.setPaintProperty('spl', 'raster-opacity', opacity);
  }, [opacity, map, layer]);
  useEffect(() => {
    if (map.getLayer('iso')) map.setLayoutProperty('iso', 'visibility', showIso ? 'visible' : 'none');
  }, [showIso, map, layer]);

  // ---------- Dancefloor, impact, coupe ----------
  useEffect(() => {
    if (!ensureLayers()) return;
    const ring = project?.dancefloor ?? (draftFloor.length ? draftFloor : null);
    const feats: GeoJSON.Feature[] = [];
    if (ring && ring.length >= 3) feats.push({ type: 'Feature', properties: {}, geometry: { type: 'Polygon', coordinates: [[...ring, ring[0]]] } });
    else if (ring && ring.length >= 1)
      feats.push({ type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: ring.length === 1 ? [ring[0], ring[0]] : ring } });
    (map.getSource('df') as maplibregl.GeoJSONSource | undefined)?.setData({ type: 'FeatureCollection', features: feats });
  }, [project?.dancefloor, draftFloor, map, ensureLayers, layer]);

  useEffect(() => {
    if (!ensureLayers()) return;
    const feats: GeoJSON.Feature[] =
      impact && showImpact
        ? impact.buildings.map((b) => ({
            type: 'Feature',
            properties: { class: b.class, e: b.emergence_dba },
            geometry: { type: 'Point', coordinates: [b.lon, b.lat] },
          }))
        : [];
    (map.getSource('imp') as maplibregl.GeoJSONSource | undefined)?.setData({ type: 'FeatureCollection', features: feats });
  }, [impact, showImpact, map, ensureLayers, layer]);

  useEffect(() => {
    if (!ensureLayers()) return;
    const feats: GeoJSON.Feature[] = (calib?.points ?? []).map((pt) => ({
      type: 'Feature',
      properties: { r: (pt.la ?? pt.lc ?? 0) - (pt.la != null ? pt.model_la_after : pt.model_lc_after) },
      geometry: { type: 'Point', coordinates: [pt.lon, pt.lat] },
    }));
    (map.getSource('cal') as maplibregl.GeoJSONSource | undefined)?.setData({ type: 'FeatureCollection', features: feats });
  }, [calib, map, ensureLayers, layer]);

  useEffect(() => {
    if (!ensureLayers()) return;
    const feats: GeoJSON.Feature[] = sectionPt
      ? [{ type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: [center, sectionPt] } }]
      : [];
    (map.getSource('sec') as maplibregl.GeoJSONSource | undefined)?.setData({ type: 'FeatureCollection', features: feats });
  }, [sectionPt, center, map, ensureLayers, layer]);

  // ---------- Clics sur la carte selon l'outil ----------
  const toolRef = useRef(tool);
  toolRef.current = tool;
  const projectRef = useRef(project);
  projectRef.current = project;
  const placeRef = useRef(placeModel);
  placeRef.current = placeModel;
  const resultRef = useRef(resultId);
  resultRef.current = resultId;

  useEffect(() => {
    const onClick = (e: maplibregl.MapMouseEvent) => {
      const { lng, lat } = e.lngLat;
      const t = toolRef.current;
      if (t === 'place' && catalog) {
        const m = catalog.speakers.find((s) => s.id === placeRef.current);
        if (!m) return;
        const p = projectRef.current;
        const az = p?.speakers[0]?.azimuth_deg ?? 0;
        const sp: SpeakerSpec = {
          id: newId(m.kind),
          name: `${KIND_LABEL[m.kind]} ${(p?.speakers.filter((s) => s.model.kind === m.kind).length ?? 0) + 1}`,
          model: m,
          lon: lng,
          lat,
          height_m: m.kind === 'sub' ? 0.5 : m.kind === 'line_array' ? 6 : 2.5,
          azimuth_deg: az,
          tilt_deg: 0,
          delay_ms: 0,
          polarity: 1,
          gain_db: 0,
          muted: false,
        };
        update((pp) => ({ ...pp, speakers: [...pp.speakers, sp] }));
        setSelected(sp.id);
      } else if (t === 'dancefloor') {
        setDraftFloor((d) => [...d, [lng, lat]]);
      } else if (t === 'section') {
        const p = projectRef.current;
        if (!p) return;
        setSectionPt([lng, lat]);
        simApi
          .section(p, resultRef.current, lng, lat)
          .then(setSection)
          .catch((er) => setError(errText(er)));
      }
    };
    map.on('click', onClick);
    return () => {
      map.off('click', onClick);
    };
  }, [map, catalog, update]);

  useEffect(() => {
    map.getCanvas().style.cursor = tool === 'none' ? '' : 'crosshair';
  }, [tool, map]);

  // ---------- Marqueurs des enceintes (gizmo position + azimut) ----------
  const markers = useRef(new Map<string, maplibregl.Marker>());
  const dragOrigin = useRef<{ id: string; lon: number; lat: number; all: SpeakerSpec[] } | null>(null);
  useEffect(() => {
    if (!project) return;
    const seen = new Set<string>();
    for (const s of project.speakers) {
      seen.add(s.id);
      let mk = markers.current.get(s.id);
      if (!mk) {
        const el = document.createElement('div');
        el.className = `spk spk-${s.model.kind}`;
        el.innerHTML = '<span class="spk-arrow"></span><span class="spk-body"></span>';
        el.addEventListener('click', (ev) => {
          ev.stopPropagation();
          setSelected(s.id);
          setTab('sono');
        });
        mk = new maplibregl.Marker({ element: el, draggable: true, rotationAlignment: 'map', pitchAlignment: 'map' })
          .setLngLat([s.lon, s.lat])
          .addTo(map);
        const id = s.id;
        mk.on('dragstart', () => {
          const p = projectRef.current;
          const me = p?.speakers.find((x) => x.id === id);
          if (p && me) dragOrigin.current = { id, lon: me.lon, lat: me.lat, all: p.speakers };
        });
        const onMove = (final: boolean) => {
          const o = dragOrigin.current;
          const m = markers.current.get(id);
          if (!o || !m) return;
          const ll = m.getLngLat();
          const dLon = ll.lng - o.lon;
          const dLat = ll.lat - o.lat;
          setProject((p) =>
            p
              ? {
                  ...p,
                  speakers: o.all.map((x) =>
                    x.id === id || moveAllRef.current ? { ...x, lon: x.lon + dLon, lat: x.lat + dLat } : (p.speakers.find((y) => y.id === x.id) ?? x),
                  ),
                }
              : p,
          );
          setLfStale(true);
          setUseLf(false);
          if (final) dragOrigin.current = null;
        };
        mk.on('drag', () => onMove(false));
        mk.on('dragend', () => onMove(true));
        markers.current.set(s.id, mk);
      }
      mk.setLngLat([s.lon, s.lat]);
      mk.setRotation(s.azimuth_deg);
      const el = mk.getElement();
      el.classList.toggle('selected', s.id === selected);
      el.classList.toggle('muted', s.muted);
      el.title = `${s.name} — ${s.model.name}`;
    }
    for (const [id, mk] of markers.current) {
      if (!seen.has(id)) {
        mk.remove();
        markers.current.delete(id);
      }
    }
  }, [project, map, selected]);
  const moveAllRef = useRef(moveAll);
  moveAllRef.current = moveAll;
  useEffect(
    () => () => {
      for (const mk of markers.current.values()) mk.remove();
      markers.current.clear();
    },
    [],
  );

  // ---------- Actions ----------
  const sel = project?.speakers.find((s) => s.id === selected) ?? null;
  const editSel = (f: (s: SpeakerSpec) => SpeakerSpec) =>
    update((p) => ({ ...p, speakers: p.speakers.map((s) => (s.id === selected ? f(s) : s)) }));

  const runJob = async <T,>(kind: string, f: (cb: (p: JobProgress) => void) => Promise<T>): Promise<T | null> => {
    setJob({ kind, p: null });
    setError(null);
    try {
      return await f((p) => setJob({ kind, p }));
    } catch (e) {
      const t = errText(e);
      if (!/annul/.test(t)) setError(t);
      return null;
    } finally {
      setJob(null);
    }
  };

  const runLowFreq = async () => {
    if (!project) return;
    const r = await runJob('lowfreq', (cb) => simApi.lowfreq(project, cb));
    if (r) {
      setLowfreq(r);
      setLfStale(false);
      setUseLf(true);
      setVariant('main');
      setMetric('C');
    }
  };

  const runImpact = async () => {
    if (!project) return;
    setBusy('impact');
    try {
      setImpact(await simApi.impact(project, resultId));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const runSizing = async () => {
    if (!project) return;
    setBusy('sizing');
    try {
      setSizing(await simApi.sizing(project));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const runOptimize = async () => {
    if (!project) return;
    const r = await runJob('optimize', (cb) => simApi.optimize(project, cb));
    if (r) setCandidates({ list: r.candidates, seconds: r.seconds });
  };

  const saveAs = async () => {
    if (!project) return;
    const path = await saveDialog({ defaultPath: `${project.name}.smproj`, filters: [{ name: 'Projet SoundMap', extensions: ['smproj'] }] });
    if (path) await simApi.saveProject(path, project).catch((e) => setError(errText(e)));
  };
  const openProject = async () => {
    const path = await openDialog({ filters: [{ name: 'Projet SoundMap', extensions: ['smproj'] }], multiple: false });
    if (typeof path === 'string') {
      try {
        const p = await simApi.loadProject(path);
        setProject({ ...p, zone_id: zoneId });
      } catch (e) {
        setError(errText(e));
      }
    }
  };
  const fetchGround = async () => {
    setBusy('ground');
    try {
      await simApi.fetchGround(zoneId);
      setScene(await simApi.loadScene(zoneId));
      if (project) runPreview(project);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const fetchVegetation = async () => {
    const r = await runJob('vegetation', (cb) => simApi.fetchVegetation(zoneId, vegRadius, cb));
    if (!r) return;
    try {
      setScene(await simApi.loadScene(zoneId));
    } catch (e) {
      setError(errText(e));
    }
    if (project) runPreview(project);
  };

  const importMeasurements = async () => {
    if (!project) return;
    const path = await openDialog({ filters: [{ name: 'Mesures (CSV, GPX)', extensions: ['csv', 'gpx', 'txt'] }], multiple: false });
    if (typeof path !== 'string') return;
    setBusy('calibrate');
    try {
      setCalib(await simApi.calibrate(project, path));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  const doExport = async (kind: 'geotiff' | 'geojson' | 'html') => {
    if (!project || !resultId) return;
    const ext = kind === 'geotiff' ? 'tif' : kind;
    const path = await saveDialog({ defaultPath: `${project.name}-${metric}.${ext}`, filters: [{ name: kind, extensions: [ext] }] });
    if (!path) return;
    await simApi
      .exportResult({ kind, path, project, resultId, variant: useLf ? variant : 'main', metric, impact, sizing })
      .catch((e) => setError(errText(e)));
  };

  const thresholdsText = useMemo(() => project?.settings.thresholds_db.join(', ') ?? '', [project?.settings.thresholds_db]);

  if (!project || !catalog) {
    return (
      <section className="card">
        <h2>{zone.spec.name}</h2>
        <p className="muted small">{error ?? 'Chargement de la scène…'}</p>
      </section>
    );
  }

  const set = (f: (s: Project['settings']) => Project['settings']) => update((p) => ({ ...p, settings: f(p.settings) }));
  const progressPct = job?.p && job.p.total > 0 ? (100 * job.p.done) / job.p.total : 0;

  return (
    <>
      <nav className="tabs" role="tablist">
        {TABS.map(([t, label]) => (
          <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? 'on' : ''} onClick={() => setTab(t)}>
            {label}
          </button>
        ))}
      </nav>

      {error && (
        <div className="alert" role="alert">
          <span>{error}</span>
          <button className="ghost" onClick={() => setError(null)} aria-label="Fermer">
            ×
          </button>
        </div>
      )}

      {job && (
        <section className="card">
          <h2>{job.kind === 'lowfreq' ? 'Calcul basses fréquences (GFPE)' : 'Optimisation'}</h2>
          <div className="bar" role="progressbar" aria-valuenow={Math.round(progressPct)} aria-valuemin={0} aria-valuemax={100}>
            <div style={{ width: `${progressPct}%` }} />
          </div>
          <p className="small">
            {job.p ? `${job.p.done.toLocaleString('fr-FR')} / ${job.p.total.toLocaleString('fr-FR')} — ${job.p.message}` : 'Préparation…'}
          </p>
          <button className="ghost danger" onClick={() => simApi.cancel(job.kind)}>
            Annuler
          </button>
        </section>
      )}

      {preview && (
        <p className="mono muted small status">
          {useLf && lowfreq ? `GFPE · ${variant}` : `aperçu ${preview.total_ms.toFixed(0)} ms`}
          {!useLf && preview.timing.cached_excess ? ' (incrémental)' : ''} · {n1(preview.air.t_c)} °C · {n1(preview.air.rh, 0)} %
          {layer ? ` · max ${n1(layer.stats.max_db)} dB` : ''}
        </p>
      )}

      {tab === 'sono' && (
        <>
          <section className="card">
            <h2>Placer des enceintes</h2>
            <div className="row">
              <select value={placeModel} onChange={(e) => setPlaceModel(e.target.value)} aria-label="Modèle à placer">
                {catalog.speakers.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
              <button className={tool === 'place' ? 'primary pulse' : 'primary'} onClick={() => setTool(tool === 'place' ? 'none' : 'place')}>
                {tool === 'place' ? 'Cliquer sur la carte' : 'Placer'}
              </button>
            </div>
            <button
              onClick={() =>
                update((p) => ({ ...p, speakers: [...p.speakers, ...freePartySystem(catalog, center[0], center[1], p.speakers[0]?.azimuth_deg ?? 0)] }))
              }
            >
              + Système type (4 subs 18" + 2 têtes) au point sono
            </button>
            <label className="check">
              <input type="checkbox" checked={moveAll} onChange={(e) => setMoveAll(e.target.checked)} />
              Glisser une enceinte déplace tout le système
            </label>
            <div className="row">
              <span className="small muted">Orienter tout le système</span>
              <input
                type="range"
                min={0}
                max={359}
                value={Math.round(project.speakers[0]?.azimuth_deg ?? 0)}
                onChange={(e) => {
                  const az = Number(e.target.value);
                  update((p) => {
                    const d = az - (p.speakers[0]?.azimuth_deg ?? 0);
                    if (p.speakers.length === 0) return p;
                    const n = p.speakers.length;
                    const clon = p.speakers.reduce((a, s) => a + s.lon, 0) / n;
                    const clat = p.speakers.reduce((a, s) => a + s.lat, 0) / n;
                    const k = 111_320;
                    const kx = k * Math.cos((clat * Math.PI) / 180);
                    const r = (d * Math.PI) / 180;
                    return {
                      ...p,
                      speakers: p.speakers.map((s) => {
                        const x = (s.lon - clon) * kx;
                        const y = (s.lat - clat) * k;
                        const [lo, la] = offsetLonLat(clon, clat, x * Math.cos(r) + y * Math.sin(r), -x * Math.sin(r) + y * Math.cos(r));
                        return { ...s, lon: lo, lat: la, azimuth_deg: (s.azimuth_deg + d + 360) % 360 };
                      }),
                    };
                  });
                }}
                aria-label="Azimut du système"
              />
            </div>
          </section>

          <section className="card">
            <h2>Enceintes ({project.speakers.length})</h2>
            {project.speakers.length === 0 && <p className="muted small">Aucune enceinte. Placez-en sur la carte ou ajoutez le système type.</p>}
            <ul className="spk-list">
              {project.speakers.map((s) => (
                <li key={s.id} className={s.id === selected ? 'on' : ''}>
                  <button className="ghost" onClick={() => setSelected(s.id)}>
                    <i className={`dot ${s.model.kind}`} /> {s.name}
                    <span className="muted small">
                      {' '}
                      {s.azimuth_deg.toFixed(0)}° · {s.height_m} m{s.delay_ms ? ` · ${s.delay_ms.toFixed(2)} ms` : ''}
                      {s.polarity < 0 ? ' · inv.' : ''}
                      {s.muted ? ' · coupée' : ''}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>

          {sel && (
            <SpeakerEditor
              spk={sel}
              models={catalog.speakers}
              onChange={editSel}
              onDelete={() => {
                update((p) => ({ ...p, speakers: p.speakers.filter((s) => s.id !== selected) }));
                setSelected(null);
              }}
            />
          )}

          <section className="card">
            <h2>Dancefloor</h2>
            {tool === 'dancefloor' ? (
              <div className="row">
                <span className="small">{draftFloor.length} points</span>
                <button
                  className="primary"
                  disabled={draftFloor.length < 3}
                  onClick={() => {
                    update((p) => ({ ...p, dancefloor: draftFloor }));
                    setDraftFloor([]);
                    setTool('none');
                  }}
                >
                  Terminer
                </button>
                <button
                  className="ghost"
                  onClick={() => {
                    setDraftFloor([]);
                    setTool('none');
                  }}
                >
                  Annuler
                </button>
              </div>
            ) : (
              <div className="row">
                <button
                  onClick={() => {
                    setDraftFloor([]);
                    setTool('dancefloor');
                  }}
                >
                  {project.dancefloor ? 'Redessiner' : 'Dessiner le polygone'}
                </button>
                {project.dancefloor && (
                  <button className="ghost danger" onClick={() => update((p) => ({ ...p, dancefloor: null }))}>
                    Effacer
                  </button>
                )}
              </div>
            )}
          </section>
        </>
      )}

      {tab === 'carte' && (
        <>
          <section className="card">
            <h2>Indicateur</h2>
            <div className="chips">
              {METRICS.map((m) => (
                <button key={m.key} className={metric === m.key ? 'chipbtn on' : 'chipbtn'} onClick={() => setMetric(m.key)}>
                  {m.label}
                </button>
              ))}
            </div>
            {useLf && layer && layer.variants.length > 1 && (
              <label className="field">
                <span>Variante (enveloppe de scénarios)</span>
                <select value={variant} onChange={(e) => setVariant(e.target.value)}>
                  {layer.variants.map((v) => (
                    <option key={v} value={v}>
                      {v === 'main' ? 'Médiane des scénarios' : v === 'p90' ? 'P90 (90e centile)' : (lowfreq?.scenarios.find((s) => s.id === v)?.label ?? v)}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {lowfreq && (
              <label className="check">
                <input type="checkbox" checked={useLf} onChange={(e) => setUseLf(e.target.checked)} disabled={lfStale} />
                Afficher le calcul GFPE {lfStale ? '(obsolète : système modifié)' : ''}
              </label>
            )}
            <label className="field">
              <span>Opacité {Math.round(opacity * 100)} %</span>
              <input type="range" min={0.1} max={1} step={0.05} value={opacity} onChange={(e) => setOpacity(Number(e.target.value))} />
            </label>
            <label className="check">
              <input type="checkbox" checked={showIso} onChange={(e) => setShowIso(e.target.checked)} />
              Isophones (pas de 5 dB)
            </label>
            {layer && <Legend offset={layer.palette_offset} />}
            {layer && <p className="muted small">{layer.method}</p>}
          </section>
          <section className="card">
            <h2>Coupe du terrain</h2>
            <button className={tool === 'section' ? 'primary pulse' : ''} onClick={() => setTool(tool === 'section' ? 'none' : 'section')}>
              {tool === 'section' ? 'Cliquer un point de la carte' : 'Choisir un point'}
            </button>
            {section && <SectionChart s={section} />}
            {sectionPt && (
              <div className="aural">
                <button
                  onClick={async () => {
                    setBusy('aural');
                    try {
                      setAural(await simApi.auralize(project, resultId, sectionPt[0], sectionPt[1]));
                    } catch (e) {
                      setError(errText(e));
                    } finally {
                      setBusy(null);
                    }
                  }}
                  disabled={busy === 'aural'}
                >
                  {busy === 'aural' ? 'Synthèse…' : 'Écouter ce point'}
                </button>
                {aural && (
                  <>
                    <p className="small">
                      Dancefloor {n1(aural.dancefloor_dba, 0)} dB(A) → point ({(aural.distance_m / 1000).toFixed(2)} km){' '}
                      {n1(aural.point_dba, 0)} dB(A) / {n1(aural.point_dbc, 0)} dB(C), bruit résiduel {n1(aural.residual_dba, 0)} dB(A).
                    </p>
                    <label className="small">Au dancefloor</label>
                    <audio controls src={`data:audio/wav;base64,${aural.dancefloor_wav}`} />
                    <label className="small">Chez le riverain (timbre et bruit de fond ; niveau ramené, en réalité −{n1(aural.level_drop_dba, 0)} dB)</label>
                    <audio controls src={`data:audio/wav;base64,${aural.point_wav}`} />
                  </>
                )}
              </div>
            )}
          </section>
        </>
      )}

      {tab === 'bf' && (
        <section className="card">
          <h2>Basses fréquences (GFPE)</h2>
          <p className="small muted">
            Équation parabolique sur 360 azimuts, tiers d'octave 25–200 Hz, relief LiDAR, sol par segment, profil météo AROME
            (Monin-Obukhov) et 3 scénarios de référence. Fusion avec la méthode d'ingénierie au-delà de 200 Hz.
          </p>
          <button className="primary" disabled={!!job || project.speakers.length === 0} onClick={runLowFreq}>
            Lancer le calcul (≈ {scene ? '10–60 s' : '…'})
          </button>
          {lowfreq && (
            <>
              <table className="tbl">
                <thead>
                  <tr>
                    <th>Scénario</th>
                    <th className="num">max dB(C)</th>
                    <th className="num">≥ 55 dB(C)</th>
                  </tr>
                </thead>
                <tbody>
                  {lowfreq.scenarios.map((s) => (
                    <tr key={s.id}>
                      <td>
                        {s.label}
                        <div className="muted small">
                          u* {s.u_star.toFixed(2)} m/s · 1/L {s.inv_l.toFixed(3)} m⁻¹ · {s.seconds.toFixed(1)} s
                        </div>
                      </td>
                      <td className="num">{n1(s.max_dbc)}</td>
                      <td className="num">{n1(s.area_55_dbc_km2, 2)} km²</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="small">
                Total {fmtDuration(lowfreq.total_s)}. {lfStale && <span className="warn-text">Le système a changé depuis : relancer.</span>}
              </p>
              <p className="muted small">
                Incertitude affichée : la carte « médiane » et la carte « P90 » de l'onglet Carte encadrent les scénarios. Précision
                attendue : ±3 dB à moins de 500 m, ±5 à 10 dB à plusieurs km de nuit.
              </p>
            </>
          )}
        </section>
      )}

      {tab === 'impact' && (
        <>
          <section className="card">
            <h2>Seuils et bruit résiduel</h2>
            <label className="field">
              <span>Seuils d'émergence (dB), libres</span>
              <input
                type="text"
                defaultValue={thresholdsText}
                onBlur={(e) => {
                  const v = e.target.value
                    .split(/[;, ]+/)
                    .map(Number)
                    .filter((x) => Number.isFinite(x) && x > 0);
                  if (v.length) set((s) => ({ ...s, thresholds_db: v.sort((a, b) => a - b) }));
                }}
              />
            </label>
            <label className="field">
              <span>Résiduel rural de nuit : {project.settings.residual.rural_night_dba} dB(A)</span>
              <input
                type="range"
                min={15}
                max={45}
                value={project.settings.residual.rural_night_dba}
                onChange={(e) => set((s) => ({ ...s, residual: { ...s.residual, rural_night_dba: Number(e.target.value) } }))}
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={project.settings.residual.include_roads}
                onChange={(e) => set((s) => ({ ...s, residual: { ...s.residual, include_roads: e.target.checked } }))}
              />
              Majorer près des routes
            </label>
            <label className="field">
              <span>Mesure du résiduel (dB(A), remplace le modèle)</span>
              <input
                type="text"
                placeholder="vide = modèle"
                defaultValue={project.settings.residual.measured_dba ?? ''}
                onBlur={(e) => {
                  const v = e.target.value.trim() === '' ? null : Number(e.target.value);
                  set((s) => ({ ...s, residual: { ...s.residual, measured_dba: v !== null && Number.isFinite(v) ? v : null } }));
                }}
              />
            </label>
            <button className="primary" onClick={runImpact} disabled={busy === 'impact' || project.speakers.length === 0}>
              {busy === 'impact' ? 'Calcul…' : useLf ? 'Évaluer l\'impact (GFPE)' : 'Évaluer l\'impact'}
            </button>
          </section>
          {impact && <ImpactView r={impact} show={showImpact} setShow={setShowImpact} onFocus={(b) => map.flyTo({ center: [b.lon, b.lat], zoom: 16.5 })} />}
        </>
      )}

      {tab === 'dim' && (
        <>
          <section className="card">
            <h2>Dimensionnement inverse</h2>
            {!project.dancefloor && <p className="muted small">Dessinez d'abord le dancefloor (onglet Sono).</p>}
            <div className="row">
              <label className="field">
                <span>Cible dB(A)</span>
                <input type="number" value={project.settings.target.dba} onChange={(e) => set((s) => ({ ...s, target: { ...s.target, dba: Number(e.target.value) } }))} />
              </label>
              <label className="field">
                <span>Cible dB(C)</span>
                <input type="number" value={project.settings.target.dbc} onChange={(e) => set((s) => ({ ...s, target: { ...s.target, dbc: Number(e.target.value) } }))} />
              </label>
            </div>
            <button className="primary" disabled={!project.dancefloor || busy === 'sizing'} onClick={runSizing}>
              Calculer
            </button>
            {sizing && <SizingView r={sizing} />}
          </section>
          <section className="card">
            <h2>Optimiseur</h2>
            <p className="muted small">
              Position (±60 m), azimut (pas de 15°) et montage des subs (tel quel, cardioïde, end-fire) : maximise la marge au bâtiment
              le plus exposé sous contrainte du niveau dancefloor.
            </p>
            <button className="primary" disabled={!!job || project.speakers.length === 0} onClick={runOptimize}>
              Lancer l'optimisation
            </button>
            {candidates && (
              <>
                <p className="small muted">{candidates.list.length} meilleurs candidats en {candidates.seconds.toFixed(1)} s.</p>
                <ul className="cands">
                  {candidates.list.slice(0, 5).map((c, i) => (
                    <li key={i}>
                      <div>
                        <strong>
                          {c.config === 'as_is' ? 'Tel quel' : c.config === 'cardioid' ? 'Cardioïde' : 'End-fire'} · {c.azimuth_deg.toFixed(0)}°
                        </strong>
                        <span className="muted small">
                          {' '}
                          Δ {c.dx.toFixed(0)} / {c.dy.toFixed(0)} m
                        </span>
                        <div className="small">
                          émergence max {signed(c.worst_emergence_dba)} dB(A) · marge {signed(c.margin_db)} dB
                          {Number.isFinite(c.dancefloor_dba) ? ` · dancefloor ${n1(c.dancefloor_dba)}/${n1(c.dancefloor_dbc)}` : ''}
                        </div>
                        {c.gfpe_margin_db !== null && (
                          <div className="small ok-text">
                            vérifié GFPE : émergence max {signed(c.gfpe_worst_emergence_dba ?? 0)} dB(A) · marge {signed(c.gfpe_margin_db)} dB
                          </div>
                        )}
                      </div>
                      <button className="small" onClick={() => update((p) => ({ ...p, speakers: c.speakers }))}>
                        Appliquer
                      </button>
                    </li>
                  ))}
                </ul>
                <p className="muted small">Recherche grossière par la méthode d'ingénierie, puis vérification GFPE (scénario prévision) des 3 meilleurs.</p>
              </>
            )}
          </section>
        </>
      )}

      {tab === 'projet' && (
        <>
          <section className="card">
            <h2>Projet</h2>
            <label className="field">
              <span>Nom</span>
              <input type="text" value={project.name} onChange={(e) => update((p) => ({ ...p, name: e.target.value }))} />
            </label>
            <label className="field">
              <span>Programme musical</span>
              <select value={project.settings.program} onChange={(e) => set((s) => ({ ...s, program: e.target.value }))}>
                {catalog.programs.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>Gain général {signed(project.settings.master_gain_db)} dB</span>
              <input type="range" min={-30} max={6} step={0.5} value={project.settings.master_gain_db} onChange={(e) => set((s) => ({ ...s, master_gain_db: Number(e.target.value) }))} />
            </label>
            <div className="row">
              <label className="field">
                <span>Hauteur des récepteurs</span>
                <select value={project.settings.receiver_height_m} onChange={(e) => set((s) => ({ ...s, receiver_height_m: Number(e.target.value) }))}>
                  <option value={1.5}>1,5 m</option>
                  <option value={4}>4 m</option>
                </select>
              </label>
              <label className="field">
                <span>Conditions</span>
                <select value={project.settings.condition} onChange={(e) => set((s) => ({ ...s, condition: e.target.value as 'favorable' | 'homogeneous' }))}>
                  <option value="favorable">Favorables (nuit)</option>
                  <option value="homogeneous">Homogènes</option>
                </select>
              </label>
            </div>
            <div className="row">
              <label className="field">
                <span>Heure météo AROME</span>
                <select value={project.settings.meteo_hour} onChange={(e) => set((s) => ({ ...s, meteo_hour: Number(e.target.value) }))}>
                  {Array.from({ length: 24 }, (_, h) => (
                    <option key={h} value={h}>
                      {String(h).padStart(2, '0')} h
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Impédance du sol</span>
                <select value={project.settings.impedance_model} onChange={(e) => set((s) => ({ ...s, impedance_model: e.target.value as 'DelanyBazley' | 'Miki' }))}>
                  <option value="DelanyBazley">Delany-Bazley</option>
                  <option value="Miki">Miki</option>
                </select>
              </label>
            </div>
            <div className="row">
              <button onClick={saveAs}>Enregistrer sous… (.smproj)</button>
              <button onClick={openProject}>Ouvrir…</button>
            </div>
            <p className="muted small">Le projet est aussi enregistré automatiquement dans le dossier de la zone.</p>
          </section>
          <section className="card">
            <h2>Sol : géologie et état</h2>
            <p className="muted small">
              Falaises (pente ≥ 45°) traitées en roche. Géologie BRGM (lithologie 1/1 000 000) : sols minces sur roche dure
              durcis. État du sol à l'heure du calcul (Open-Meteo) : l'humidité durcit le sol, la neige le rend très absorbant.
            </p>
            <p className="small">
              Falaises : {n1(scene?.ground.cliffs_km2, 2)} km²
              {scene?.ground.formations
                ? ` ; sols durcis : ${n1(scene.ground.hardened_km2, 2)} km² ; formations : ${scene.ground.formations
                    .map(([d, k]) => `${d} (${k === 'hard' ? 'dure' : k === 'mixed' ? 'mixte' : 'meuble'})`)
                    .join(' ; ')}`
                : ' ; géologie non téléchargée'}
            </p>
            {scene && (
              <p className="small">
                Relief : {scene.valley.valley ? 'fond de vallée' : 'pas de vallée'} (position {signed(scene.valley.tpi_m, 0)} m par rapport
                aux environs, pente régionale {n1(scene.valley.slope_deg, 1)}° vers {Math.round(scene.valley.downslope_toward_deg)}°)
                {scene.valley.valley ? ' — scénario « lac d’air froid + vent catabatique » ajouté au calcul basses fréquences.' : '.'}
              </p>
            )}
            {(() => {
              const h = scene?.ground_hours?.find((g) => g.hour === project.settings.meteo_hour);
              return h ? (
                <p className="small">
                  {String(h.hour).padStart(2, '0')} h : humidité {h.state.moisture != null ? `${Math.round(h.state.moisture * 100)} %` : '—'}, pluie
                  24 h {n1(h.state.rain_24h_mm, 1)} mm, neige {h.state.snow_m != null ? `${Math.round(h.state.snow_m * 100)} cm` : '—'} → {h.snow ? 'sol enneigé (classe A)' : `σ × ${n1(h.sigma_factor, 2)}`}
                </p>
              ) : (
                <p className="muted small">État du sol non téléchargé pour cette date.</p>
              );
            })()}
            <div className="row">
              <label className="check">
                <input
                  type="checkbox"
                  checked={project.settings.use_ground_state !== false}
                  onChange={(e) => set((s) => ({ ...s, use_ground_state: e.target.checked }))}
                />
                Tenir compte de l'état du sol
              </label>
              <button onClick={fetchGround} disabled={busy === 'ground'}>
                {busy === 'ground' ? 'Téléchargement…' : scene?.ground_hours ? 'Mettre à jour' : 'Télécharger'}
              </button>
            </div>
          </section>
          <section className="card">
            <h2>Végétation LiDAR HD</h2>
            <p className="muted small">
              Télécharge les nuages de points COPC autour de la sono (niveau de détail ≈ 2 m, environ 40 Mo par km²) : densité du
              feuillage par voxels de 5 m × 2 m et rugosité du sol. Le feuillage mesuré remplace le forfait « feuillage dense » de
              l'ISO 9613-2.
            </p>
            {scene?.vegetation ? (
              <p className="small">
                {scene.vegetation.tiles} dalles, {(scene.vegetation.points / 1e6).toFixed(1)} M points dans{' '}
                {(scene.vegetation.radius_m / 1000).toFixed(1)} km — feuillage dense {n1(scene.vegetation.dense_foliage_km2, 2)} km²
                (facteur moyen {n1(scene.vegetation.mean_foliage, 2)}), rugosité moyenne {n1(scene.vegetation.mean_roughness_m * 100, 0)} cm.
              </p>
            ) : (
              <p className="muted small">Pas encore téléchargée : forfait ISO (MNH ≥ 3 m = feuillage dense).</p>
            )}
            <div className="row">
              <label className="field">
                <span>Rayon</span>
                <select value={vegRadius} onChange={(e) => setVegRadius(Number(e.target.value))}>
                  {[500, 1000, 1500, 2000, 3000].map((r) => (
                    <option key={r} value={r}>
                      {r / 1000} km (≈ {Math.round((Math.PI * (r / 1000) ** 2 * 40) / 5) * 5} Mo)
                    </option>
                  ))}
                </select>
              </label>
              <button onClick={fetchVegetation} disabled={!!job}>
                {scene?.vegetation ? 'Retélécharger' : 'Télécharger'}
              </button>
            </div>
          </section>
          <section className="card">
            <h2>Calage sur mesures</h2>
            <p className="muted small">
              CSV (colonnes lat, lon, LAeq et/ou LCeq) ou GPX de sonomètre. Ajuste la dureté du sol, le gradient de célérité
              (stabilité, vent) et un décalage de niveau sur la méthode d'ingénierie.
            </p>
            {project.settings.calibration && (
              <p className="small">
                Calage actif : sol σ × {n1(project.settings.calibration.sigma_scale, 2)}, gradient{' '}
                {n1(project.settings.calibration.refraction.grad_t, 3)} + {n1(project.settings.calibration.refraction.grad_w, 3)} s⁻¹ (vent),{' '}
                {signed(project.settings.calibration.offset_db)} dB — écart résiduel {n1(project.settings.calibration.rms_db, 1)} dB sur{' '}
                {project.settings.calibration.n_points} points.
              </p>
            )}
            <div className="row">
              <button onClick={importMeasurements} disabled={busy === 'calibrate'}>
                {busy === 'calibrate' ? 'Calage…' : 'Importer des mesures…'}
              </button>
              {project.settings.calibration && <button onClick={() => set((s) => ({ ...s, calibration: null }))}>Retirer le calage</button>}
            </div>
            {calib && (
              <>
                <p className="small">
                  {calib.n_points} points{calib.n_ignored > 0 ? ` (${calib.n_ignored} ignorés)` : ''} — écart quadratique{' '}
                  <b>{n1(calib.rms_before_db, 1)} → {n1(calib.rms_after_db, 1)} dB</b> (biais avant {signed(calib.bias_before_db)} dB)
                </p>
                <table className="small">
                  <tbody>
                    <tr>
                      <td>Sol (facteur σ)</td>
                      <td>
                        × {n1(calib.sigma_scale.map, 2)} <span className="muted">(± {n1(calib.sigma_scale.sd, 2)} oct.)</span>
                      </td>
                    </tr>
                    <tr>
                      <td>Gradient thermique</td>
                      <td>
                        {n1(calib.grad_t.map, 3)} s⁻¹ <span className="muted">(± {n1(calib.grad_t.sd, 3)})</span>
                      </td>
                    </tr>
                    <tr>
                      <td>Gradient vent (vers {Math.round(calib.wind_toward_deg)}°)</td>
                      <td>
                        {n1(calib.grad_w.map, 3)} s⁻¹ <span className="muted">(± {n1(calib.grad_w.sd, 3)})</span>
                      </td>
                    </tr>
                    <tr>
                      <td>Décalage de niveau</td>
                      <td>
                        {signed(calib.offset_db.map)} dB <span className="muted">(± {n1(calib.offset_db.sd, 1)})</span>
                      </td>
                    </tr>
                  </tbody>
                </table>
                {calib.notes.map((n) => (
                  <p key={n} className="muted small">
                    ⚠ {n}
                  </p>
                ))}
                <div className="row">
                  <button className="primary" onClick={() => set((s) => ({ ...s, calibration: calib.calibration }))}>
                    Appliquer au projet
                  </button>
                  <button onClick={() => setCalib(null)}>Masquer</button>
                </div>
              </>
            )}
          </section>
          <section className="card">
            <h2>Exports (carte affichée : {METRICS.find((m) => m.key === metric)?.label})</h2>
            <div className="row wrap">
              <button disabled={!resultId} onClick={() => doExport('html')}>
                Rapport HTML (imprimable en PDF)
              </button>
              <button disabled={!resultId} onClick={() => doExport('geotiff')}>
                GeoTIFF
              </button>
              <button disabled={!resultId} onClick={() => doExport('geojson')}>
                GeoJSON
              </button>
            </div>
            <p className="muted small">Le rapport inclut l'impact s'il a été évalué.</p>
          </section>
          {scene && (
            <section className="card">
              <h2>Zone</h2>
              <dl className="kv">
                <dt>Bâtiments</dt>
                <dd>
                  {scene.buildings.toLocaleString('fr-FR')} dont {scene.residential.toLocaleString('fr-FR')} habitations
                </dd>
                <dt>Habitants (Filosofi)</dt>
                <dd>{scene.inhabitants.toFixed(0)}</dd>
                <dt>Lieux sensibles</dt>
                <dd>{scene.sensitive}</dd>
                <dt>ZNIEFF</dt>
                <dd>{scene.znieff}</dd>
                <dt>Sol</dt>
                <dd>{scene.landcover}</dd>
                <dt>Scène chargée en</dt>
                <dd>{scene.load_ms.toFixed(0)} ms</dd>
              </dl>
            </section>
          )}
        </>
      )}

      <button className="ghost" onClick={onBack}>
        ← Retour à l'exploration
      </button>
    </>
  );
}

function Legend({ offset }: { offset: number }) {
  return (
    <div className="legend-scale" aria-label="Légende des niveaux">
      {PALETTE.map(([t, c]) => (
        <div key={t} style={{ background: c }} title={`≥ ${t + offset} dB`}>
          <span>{t + offset}</span>
        </div>
      ))}
    </div>
  );
}

function SpeakerEditor({
  spk,
  models,
  onChange,
  onDelete,
}: {
  spk: SpeakerSpec;
  models: SpeakerModel[];
  onChange: (f: (s: SpeakerSpec) => SpeakerSpec) => void;
  onDelete: () => void;
}) {
  const [manual, setManual] = useState(false);
  const num = (k: keyof SpeakerSpec, step = 0.1, min?: number, max?: number) => (
    <input type="number" step={step} min={min} max={max} value={spk[k] as number} onChange={(e) => onChange((s) => ({ ...s, [k]: Number(e.target.value) }))} />
  );
  const m = spk.model;
  return (
    <section className="card">
      <h2>{spk.name}</h2>
      <label className="field">
        <span>Modèle</span>
        <select
          value={models.some((x) => x.id === m.id) ? m.id : '__custom'}
          onChange={(e) => {
            const nm = models.find((x) => x.id === e.target.value);
            if (nm) onChange((s) => ({ ...s, model: nm }));
          }}
        >
          {models.map((x) => (
            <option key={x.id} value={x.id}>
              {x.name}
            </option>
          ))}
          {!models.some((x) => x.id === m.id) && <option value="__custom">{m.name}</option>}
        </select>
      </label>
      <label className="field">
        <span>Azimut {spk.azimuth_deg.toFixed(0)}°</span>
        <input type="range" min={0} max={359} value={spk.azimuth_deg} onChange={(e) => onChange((s) => ({ ...s, azimuth_deg: Number(e.target.value) }))} />
      </label>
      <div className="grid3">
        <label className="field">
          <span>Hauteur (m)</span>
          {num('height_m', 0.1, 0, 30)}
        </label>
        <label className="field">
          <span>Site (°)</span>
          {num('tilt_deg', 1, -30, 30)}
        </label>
        <label className="field">
          <span>Retard (ms)</span>
          {num('delay_ms', 0.05, 0, 200)}
        </label>
        <label className="field">
          <span>Gain (dB)</span>
          {num('gain_db', 0.5, -40, 6)}
        </label>
        <label className="field">
          <span>Polarité</span>
          <button onClick={() => onChange((s) => ({ ...s, polarity: s.polarity < 0 ? 1 : -1 }))}>{spk.polarity < 0 ? 'Inversée' : 'Normale'}</button>
        </label>
        <label className="field">
          <span>État</span>
          <button onClick={() => onChange((s) => ({ ...s, muted: !s.muted }))}>{spk.muted ? 'Coupée' : 'Active'}</button>
        </label>
      </div>
      <button className="ghost small" onClick={() => setManual((v) => !v)}>
        {manual ? '▾' : '▸'} Saisie manuelle du modèle
      </button>
      {manual && (
        <div className="grid3">
          {(
            [
              ['max_spl_1m', 'SPL max 1 m (dB/oct.)'],
              ['f_lo', 'f basse (Hz)'],
              ['f_hi', 'f haute (Hz)'],
              ['rolloff_db_oct', 'Pente (dB/oct.)'],
              ['power_w', 'Puissance (W)'],
            ] as [keyof SpeakerModel, string][]
          ).map(([k, label]) => (
            <label key={k} className="field">
              <span>{label}</span>
              <input
                type="number"
                value={m[k] as number}
                onChange={(e) =>
                  onChange((s) => ({ ...s, model: { ...s.model, [k]: Number(e.target.value), id: `${s.model.id.replace(/-custom$/, '')}-custom`, name: s.model.name.replace(/ \(modifié\)$/, '') + ' (modifié)' } }))
                }
              />
            </label>
          ))}
        </div>
      )}
      <button className="ghost danger" onClick={onDelete}>
        Supprimer l'enceinte
      </button>
    </section>
  );
}

function ImpactView({
  r,
  show,
  setShow,
  onFocus,
}: {
  r: ImpactReport;
  show: boolean;
  setShow: (v: boolean) => void;
  onFocus: (b: { lon: number; lat: number }) => void;
}) {
  const worst = r.most_exposed;
  return (
    <>
      <section className="card">
        <h2>Résultat</h2>
        <p className={r.margin_db < 0 ? 'big warn-text' : 'big ok-text'}>
          Marge {signed(r.margin_db)} dB
          <span className="small muted"> avant {r.thresholds_db[0]} dB d'émergence au bâtiment le plus exposé</span>
        </p>
        <table className="tbl">
          <thead>
            <tr>
              <th>Émergence dB(A)</th>
              <th className="num">Bâtiments</th>
              <th className="num">Habitants</th>
            </tr>
          </thead>
          <tbody>
            {r.classes.map((c, i) => (
              <tr key={c.label}>
                <td>
                  <i className={`cls c${Math.min(i, 3)}`} /> {c.label}
                </td>
                <td className="num">{c.buildings}</td>
                <td className="num">{c.inhabitants.toFixed(0)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <label className="check">
          <input type="checkbox" checked={show} onChange={(e) => setShow(e.target.checked)} />
          Afficher les habitations sur la carte
        </label>
        <p className="muted small">
          {r.method} · {r.buildings.length} habitations · {r.compute_ms.toFixed(0)} ms
        </p>
      </section>
      {worst && (
        <section className="card">
          <h2>Bâtiment le plus exposé</h2>
          <dl className="kv">
            <dt>Distance</dt>
            <dd>{worst.distance_m.toFixed(0)} m</dd>
            <dt>Niveau façade</dt>
            <dd>
              {n1(worst.level_dba)} dB(A) · {n1(worst.level_dbc)} dB(C) ({worst.height_m} m)
            </dd>
            <dt>Résiduel</dt>
            <dd>
              {n1(worst.residual_dba)} dB(A) · {n1(worst.residual_dbc)} dB(C)
            </dd>
            <dt>Émergence</dt>
            <dd>
              {signed(worst.emergence_dba)} dB(A) · {signed(worst.emergence_dbc)} dB(C)
            </dd>
            <dt>Habitants</dt>
            <dd>{worst.inhabitants.toFixed(1)}</dd>
          </dl>
          <p className="small muted">Émergence par octave (31,5 Hz → 8 kHz) :</p>
          <div className="octbars">
            {worst.emergence_oct.map((e, i) => (
              <div key={i} title={`${signed(e)} dB`}>
                <div style={{ height: `${Math.min(100, Math.max(2, e * 4))}%` }} />
                <span>{['31', '63', '125', '250', '500', '1k', '2k', '4k', '8k'][i]}</span>
              </div>
            ))}
          </div>
          <button className="small" onClick={() => onFocus(worst)}>
            Centrer la carte
          </button>
        </section>
      )}
      <section className="card">
        <h2>Les plus exposés</h2>
        <table className="tbl">
          <thead>
            <tr>
              <th>Dist.</th>
              <th className="num">dB(A)</th>
              <th className="num">Ém. A</th>
              <th className="num">Ém. C</th>
            </tr>
          </thead>
          <tbody>
            {r.buildings.slice(0, 12).map((b) => (
              <tr key={b.id} className="click" onClick={() => onFocus(b)}>
                <td>{b.distance_m.toFixed(0)} m</td>
                <td className="num">{n1(b.level_dba)}</td>
                <td className="num">{signed(b.emergence_dba)}</td>
                <td className="num">{signed(b.emergence_dbc)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      {(r.sensitive.length > 0 || r.fauna.length > 0) && (
        <section className="card">
          <h2>Lieux sensibles et faune</h2>
          {r.sensitive.map((s, i) => (
            <p key={i} className="small">
              {s.name} <span className="muted">({s.kind})</span> : {n1(s.level_dba)} dB(A), émergence {signed(s.emergence_dba)}
            </p>
          ))}
          {r.fauna.map((f, i) => (
            <p key={`f${i}`} className="small">
              ZNIEFF {f.level} « {f.name} » : {f.area_above_45_km2.toFixed(2)} km² &gt; 45 dB(A), max {n1(f.max_dba)}
            </p>
          ))}
        </section>
      )}
    </>
  );
}

function SizingView({ r }: { r: SizingReport }) {
  return (
    <>
      <dl className="kv">
        <dt>Centre (actuel)</dt>
        <dd>
          {n1(r.center_dba)} dB(A) · {n1(r.center_dbc)} dB(C)
        </dd>
        <dt>Max (gains à 0)</dt>
        <dd>
          {n1(r.max_dba)} dB(A) · {n1(r.max_dbc)} dB(C)
        </dd>
        <dt>Subs nécessaires</dt>
        <dd>
          {r.subs_needed} <span className="muted">(actuel {r.subs_now})</span>
        </dd>
        <dt>Têtes nécessaires</dt>
        <dd>
          {r.tops_needed} <span className="muted">(actuel {r.tops_now})</span>
        </dd>
        <dt>Amplification</dt>
        <dd>{(r.power_w / 1000).toFixed(1)} kW</dd>
        <dt>Headroom</dt>
        <dd>
          {signed(r.headroom_dba)} dB(A) · {signed(r.headroom_dbc)} dB(C)
        </dd>
        <dt>Homogénéité</dt>
        <dd>
          σ {r.uniformity_std_db.toFixed(1)} dB · étendue {r.uniformity_range_db.toFixed(1)} dB ({r.samples} points)
        </dd>
      </dl>
      {r.notes.map((n, i) => (
        <p key={i} className="small muted">
          {n}
        </p>
      ))}
    </>
  );
}
