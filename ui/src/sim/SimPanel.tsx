import * as maplibregl from 'maplibre-gl';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ModelLibrary } from './ModelLibrary';
import { bearing, boxFootprint, rotateAround } from './speakerMath';
import { PROTOCOL_BASE as B, fmtDuration, type OpenZoneResponse } from '../api';
import {
  METRICS,
  PALETTE,
  dimsOf,
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
import { t as tr, useT, type TKey } from '../i18n';

type Tab = 'sono' | 'carte' | 'bf' | 'impact' | 'dim' | 'projet';
type Tool = 'none' | 'place' | 'dancefloor' | 'section';

const TABS: [Tab, TKey][] = [
  ['sono', 'sim.tab.sono'],
  ['carte', 'sim.tab.carte'],
  ['bf', 'sim.tab.bf'],
  ['impact', 'sim.tab.impact'],
  ['dim', 'sim.tab.dim'],
  ['projet', 'sim.tab.projet'],
];

const KIND_KEY = { sub: 'sim.kind.sub', top: 'sim.kind.top', line_array: 'sim.kind.line_array' } as const satisfies Record<string, TKey>;

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
  const { t, num } = useT();
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
    for (const id of ['iso', 'df', 'imp', 'sec', 'cal', 'boxes']) map.addSource(id, { type: 'geojson', data: empty });
    // Enceintes dessinées à l'échelle (dimensions du modèle), posées sur le relief.
    map.addLayer({
      id: 'spk-boxes',
      type: 'fill-extrusion',
      source: 'boxes',
      paint: {
        'fill-extrusion-color': ['match', ['get', 'kind'], 'sub', '#2a2a2e', 'top', '#3a3f4a', '#4b3f5e'],
        'fill-extrusion-base': ['get', 'base'],
        'fill-extrusion-height': ['get', 'top'],
        'fill-extrusion-opacity': 0.95,
      },
    });
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

  // ---------- Survol : niveau au point, nature du bâtiment ----------
  const hoverRef = useRef({ resultId, variant: useLf ? variant : 'main', metric });
  hoverRef.current = { resultId, variant: useLf ? variant : 'main', metric };
  useEffect(() => {
    const pop = new maplibregl.Popup({ closeButton: false, closeOnClick: false, className: 'hover-pop', offset: 14, maxWidth: '280px' });
    let latest: maplibregl.MapMouseEvent | null = null;
    let busy = false;
    let inside = false;
    const show = async () => {
      if (busy || !latest) return;
      busy = true;
      const e = latest;
      latest = null;
      const h = hoverRef.current;
      const b = map.getLayer('buildings-3d') ? map.queryRenderedFeatures(e.point, { layers: ['buildings-3d'] })[0]?.properties : undefined;
      let v: number | null = null;
      if (h.resultId && map.getLayer('spl')) {
        v = await simApi.valueAt(h.resultId, h.variant, h.metric, e.lngLat.lng, e.lngLat.lat).catch(() => null);
      }
      busy = false;
      if (!inside) return;
      if (v === null && !b) {
        pop.remove();
      } else {
        pop.setLngLat(e.lngLat).setDOMContent(hoverContent(v, METRICS.find((m) => m.key === h.metric)?.label ?? 'dB', b));
        if (!pop.isOpen()) pop.addTo(map);
      }
      if (latest) void show();
    };
    const onMove = (e: maplibregl.MapMouseEvent) => {
      inside = true;
      latest = e;
      void show();
    };
    const onOut = () => {
      inside = false;
      latest = null;
      pop.remove();
    };
    map.on('mousemove', onMove);
    map.on('mouseout', onOut);
    return () => {
      map.off('mousemove', onMove);
      map.off('mouseout', onOut);
      pop.remove();
    };
  }, [map]);

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
          name: `${tr(KIND_KEY[m.kind])} ${(p?.speakers.filter((s) => s.model.kind === m.kind).length ?? 0) + 1}`,
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

  // Boîtes 3D des enceintes : empreinte L × P tournée, de base = h − H/2 (≥ 0) à base + H.
  useEffect(() => {
    if (!project || !ensureLayers()) return;
    const feats: GeoJSON.Feature[] = project.speakers.map((s) => {
      const d = dimsOf(s.model);
      const base = Math.max(0, s.height_m - d.height_m / 2);
      return {
        type: 'Feature',
        properties: { kind: s.model.kind, base, top: base + d.height_m },
        geometry: { type: 'Polygon', coordinates: [boxFootprint(s.lon, s.lat, d.width_m, d.depth_m, s.azimuth_deg)] },
      };
    });
    (map.getSource('boxes') as maplibregl.GeoJSONSource | undefined)?.setData({ type: 'FeatureCollection', features: feats });
  }, [project, map, ensureLayers]);

  // ---------- Poignée de rotation du système, accrochée au stack ----------
  // Glisser la poignée autour du centre du système le fait pivoter (Maj : pas de 15°).
  const rotMk = useRef<maplibregl.Marker | null>(null);
  const rotDrag = useRef<{ base: SpeakerSpec[]; c: [number, number]; az0: number } | null>(null);
  const shiftRef = useRef(false);
  const placeRot = useCallback(() => {
    const mk = rotMk.current;
    const p = projectRef.current;
    if (!mk) return;
    const el = mk.getElement();
    if (!p || p.speakers.length === 0) {
      el.style.display = 'none';
      return;
    }
    el.style.display = '';
    const az = p.speakers[0].azimuth_deg;
    const label = el.querySelector('span');
    if (label) label.textContent = `${Math.round(az)}°`;
    if (rotDrag.current) return;
    const n = p.speakers.length;
    const c: [number, number] = [p.speakers.reduce((a, s) => a + s.lon, 0) / n, p.speakers.reduce((a, s) => a + s.lat, 0) / n];
    const a = (az * Math.PI) / 180;
    const P = map.project(c);
    const Q = map.project(offsetLonLat(c[0], c[1], 10 * Math.sin(a), 10 * Math.cos(a)));
    // Distance au sol donnant ~72 px à l'écran (unproject est peu fiable en 3D incliné).
    const pxPer10m = Math.hypot(Q.x - P.x, Q.y - P.y);
    const d = Math.min(400, Math.max(1.5, pxPer10m > 0.01 ? (72 / pxPer10m) * 10 : 400));
    mk.setLngLat(offsetLonLat(c[0], c[1], d * Math.sin(a), d * Math.cos(a)));
  }, [map]);
  useEffect(() => {
    const el = document.createElement('div');
    el.className = 'rot-handle';
    el.title = tr('sim.rotateHandle');
    el.innerHTML = '<b>⟳</b><span></span>';
    const mk = new maplibregl.Marker({ element: el, draggable: true }).setLngLat(map.getCenter()).addTo(map);
    rotMk.current = mk;
    mk.on('dragstart', () => {
      const p = projectRef.current;
      if (!p || p.speakers.length === 0) return;
      const n = p.speakers.length;
      rotDrag.current = {
        base: p.speakers,
        c: [p.speakers.reduce((a, s) => a + s.lon, 0) / n, p.speakers.reduce((a, s) => a + s.lat, 0) / n],
        az0: p.speakers[0].azimuth_deg,
      };
    });
    mk.on('drag', () => {
      const r = rotDrag.current;
      if (!r) return;
      const ll = mk.getLngLat();
      let az = bearing(r.c, [ll.lng, ll.lat]);
      az = shiftRef.current ? Math.round(az / 15) * 15 : Math.round(az);
      const d = az - r.az0;
      setProject((p) =>
        p
          ? {
              ...p,
              speakers: r.base.map((s) => {
                const [lo, la] = rotateAround(s.lon, s.lat, r.c[0], r.c[1], d);
                return { ...s, lon: lo, lat: la, azimuth_deg: (((s.azimuth_deg + d) % 360) + 360) % 360 };
              }),
            }
          : p,
      );
      setLfStale(true);
      setUseLf(false);
    });
    mk.on('dragend', () => {
      rotDrag.current = null;
      placeRot();
    });
    const key = (e: KeyboardEvent) => {
      shiftRef.current = e.shiftKey;
    };
    window.addEventListener('keydown', key);
    window.addEventListener('keyup', key);
    map.on('move', placeRot);
    return () => {
      map.off('move', placeRot);
      window.removeEventListener('keydown', key);
      window.removeEventListener('keyup', key);
      mk.remove();
      rotMk.current = null;
    };
  }, [map, placeRot]);
  useEffect(() => {
    placeRot();
  }, [project, placeRot]);
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
    const path = await saveDialog({ defaultPath: `${project.name}.smproj`, filters: [{ name: tr('sim.projectFile'), extensions: ['smproj'] }] });
    if (path) await simApi.saveProject(path, project).catch((e) => setError(errText(e)));
  };
  const openProject = async () => {
    const path = await openDialog({ filters: [{ name: tr('sim.projectFile'), extensions: ['smproj'] }], multiple: false });
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
    const path = await openDialog({ filters: [{ name: tr('sim.measuresFile'), extensions: ['csv', 'gpx', 'txt'] }], multiple: false });
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
        <p className="muted small">{error ?? t('sim.loading')}</p>
      </section>
    );
  }

  const set = (f: (s: Project['settings']) => Project['settings']) => update((p) => ({ ...p, settings: f(p.settings) }));
  const progressPct = job?.p && job.p.total > 0 ? (100 * job.p.done) / job.p.total : 0;

  return (
    <>
      {layer && createPortal(<MapLegend offset={layer.palette_offset} unit={METRICS.find((m) => m.key === metric)?.label ?? 'dB'} />, map.getContainer())}
      <nav className="tabs" role="tablist">
        {TABS.map(([id, label]) => (
          <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'on' : ''} onClick={() => setTab(id)}>
            {t(label)}
          </button>
        ))}
      </nav>

      {error && (
        <div className="alert" role="alert">
          <span>{error}</span>
          <button className="icon-btn" onClick={() => setError(null)} aria-label={t('common.close')}>
            ×
          </button>
        </div>
      )}

      {job && (
        <section className="card">
          <h2>{job.kind === 'lowfreq' ? t('sim.job.lowfreq') : t('sim.job.optimize')}</h2>
          <div className="bar" role="progressbar" aria-valuenow={Math.round(progressPct)} aria-valuemin={0} aria-valuemax={100}>
            <div style={{ width: `${progressPct}%` }} />
          </div>
          <p className="small">
            {job.p ? `${num(job.p.done, 0)} / ${num(job.p.total, 0)} — ${job.p.message}` : t('progress.preparing')}
          </p>
          <button className="ghost danger" onClick={() => simApi.cancel(job.kind)}>
            {t('common.cancel')}
          </button>
        </section>
      )}

      {preview && (
        <p className="readout small status">
          {useLf && lowfreq ? `GFPE · ${variant}` : t('sim.status.preview', { ms: Math.round(preview.total_ms) })}
          {!useLf && preview.timing.cached_excess ? ` ${t('sim.status.incremental')}` : ''} · {n1(preview.air.t_c)} °C · {n1(preview.air.rh, 0)} %
          {layer ? ` · max ${n1(layer.stats.max_db)} dB` : ''}
        </p>
      )}

      {tab === 'sono' && (
        <>
          <section className="card">
            <h2>{t('sim.place.title')}</h2>
            <div className="row">
              <select value={placeModel} onChange={(e) => setPlaceModel(e.target.value)} aria-label={t('sim.place.model')}>
                {catalog.speakers.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
              <button className={tool === 'place' ? 'primary pulse' : 'primary'} onClick={() => setTool(tool === 'place' ? 'none' : 'place')}>
                {tool === 'place' ? t('sim.place.clickMap') : t('sim.place.place')}
              </button>
            </div>
            <button
              onClick={() =>
                update((p) => ({ ...p, speakers: [...p.speakers, ...freePartySystem(catalog, center[0], center[1], p.speakers[0]?.azimuth_deg ?? 0)] }))
              }
            >
              {t('sim.place.typical')}
            </button>
            <label className="check">
              <input type="checkbox" checked={moveAll} onChange={(e) => setMoveAll(e.target.checked)} />
              {t('sim.place.moveAll')}
            </label>
            <div className="row">
              <span className="small muted">{t('sim.place.orient')}</span>
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
                aria-label={t('sim.place.azimuth')}
              />
            </div>
          </section>

          <ModelLibrary
            models={catalog.speakers}
            onChanged={() =>
              simApi
                .catalog()
                .then(setCatalog)
                .catch((e) => setError(errText(e)))
            }
          />

          <section className="card">
            <h2>{t('sim.speakers.title', { count: project.speakers.length })}</h2>
            {project.speakers.length === 0 && <p className="muted small">{t('sim.speakers.none')}</p>}
            <ul className="spk-list">
              {project.speakers.map((s) => (
                <li key={s.id} className={s.id === selected ? 'on' : ''}>
                  <button className="ghost" onClick={() => setSelected(s.id)}>
                    <i className={`dot ${s.model.kind}`} /> {s.name}
                    <span className="muted small">
                      {' '}
                      {s.azimuth_deg.toFixed(0)}° · {s.height_m} m{s.delay_ms ? ` · ${s.delay_ms.toFixed(2)} ms` : ''}
                      {s.polarity < 0 ? ` · ${t('sim.speakers.inv')}` : ''}
                      {s.muted ? ` · ${t('sim.speakers.muted')}` : ''}
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
            <h2>{t('sim.df.title')}</h2>
            {tool === 'dancefloor' ? (
              <div className="row">
                <span className="small">{t('sim.df.points', { count: draftFloor.length })}</span>
                <button
                  className="primary"
                  disabled={draftFloor.length < 3}
                  onClick={() => {
                    update((p) => ({ ...p, dancefloor: draftFloor }));
                    setDraftFloor([]);
                    setTool('none');
                  }}
                >
                  {t('sim.df.finish')}
                </button>
                <button
                  className="ghost"
                  onClick={() => {
                    setDraftFloor([]);
                    setTool('none');
                  }}
                >
                  {t('common.cancel')}
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
                  {project.dancefloor ? t('sim.df.redraw') : t('sim.df.draw')}
                </button>
                {project.dancefloor && (
                  <button className="ghost danger" onClick={() => update((p) => ({ ...p, dancefloor: null }))}>
                    {t('sim.df.clear')}
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
            <h2>{t('sim.map.metric')}</h2>
            <div className="chips">
              {METRICS.map((m) => (
                <button key={m.key} className={metric === m.key ? 'chipbtn on' : 'chipbtn'} onClick={() => setMetric(m.key)}>
                  {m.label}
                </button>
              ))}
            </div>
            {useLf && layer && layer.variants.length > 1 && (
              <label className="field">
                <span>{t('sim.map.variant')}</span>
                <select value={variant} onChange={(e) => setVariant(e.target.value)}>
                  {layer.variants.map((v) => (
                    <option key={v} value={v}>
                      {v === 'main' ? t('sim.map.median') : v === 'p90' ? t('sim.map.p90') : (lowfreq?.scenarios.find((s) => s.id === v)?.label ?? v)}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {lowfreq && (
              <label className="check">
                <input type="checkbox" checked={useLf} onChange={(e) => setUseLf(e.target.checked)} disabled={lfStale} />
                {t('sim.map.showGfpe')} {lfStale ? t('sim.map.stale') : ''}
              </label>
            )}
            <label className="field">
              <span>{t('sim.map.opacity', { pct: Math.round(opacity * 100) })}</span>
              <input type="range" min={0.1} max={1} step={0.05} value={opacity} onChange={(e) => setOpacity(Number(e.target.value))} />
            </label>
            <label className="check">
              <input type="checkbox" checked={showIso} onChange={(e) => setShowIso(e.target.checked)} />
              {t('sim.map.iso')}
            </label>
            {layer && <Legend offset={layer.palette_offset} />}
            {layer && <p className="muted small">{layer.method}</p>}
          </section>
          <section className="card">
            <h2>{t('sim.section.title')}</h2>
            <button className={tool === 'section' ? 'primary pulse' : ''} onClick={() => setTool(tool === 'section' ? 'none' : 'section')}>
              {tool === 'section' ? t('sim.section.click') : t('sim.section.pick')}
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
                  {busy === 'aural' ? t('sim.aural.busy') : t('sim.aural.listen')}
                </button>
                {aural && (
                  <>
                    <p className="small">
                      {t('sim.aural.summary', {
                        df: n1(aural.dancefloor_dba, 0),
                        km: num(aural.distance_m / 1000, 2),
                        a: n1(aural.point_dba, 0),
                        c: n1(aural.point_dbc, 0),
                        res: n1(aural.residual_dba, 0),
                      })}
                    </p>
                    <label className="small">{t('sim.aural.atDf')}</label>
                    <audio controls src={`data:audio/wav;base64,${aural.dancefloor_wav}`} />
                    <label className="small">{t('sim.aural.atHome', { drop: n1(aural.level_drop_dba, 0) })}</label>
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
          <h2>{t('sim.bf.title')}</h2>
          <p className="small muted">{t('sim.bf.lede')}</p>
          <button className="primary" disabled={!!job || project.speakers.length === 0} onClick={runLowFreq}>
            {t('sim.bf.run', { time: scene ? '10–60 s' : '…' })}
          </button>
          {lowfreq && (
            <>
              <table className="tbl">
                <thead>
                  <tr>
                    <th>{t('sim.bf.scenario')}</th>
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
                {t('sim.bf.total', { time: fmtDuration(lowfreq.total_s) })} {lfStale && <span className="warn-text">{t('sim.bf.stale')}</span>}
              </p>
              <p className="muted small">{t('sim.bf.uncertainty')}</p>
            </>
          )}
        </section>
      )}

      {tab === 'impact' && (
        <>
          <section className="card">
            <h2>{t('sim.impact.title')}</h2>
            <label className="field">
              <span>{t('sim.impact.thresholds')}</span>
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
              <span>{t('sim.impact.rural', { db: project.settings.residual.rural_night_dba })}</span>
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
              {t('sim.impact.roads')}
            </label>
            <label className="field">
              <span>{t('sim.impact.measured')}</span>
              <input
                type="text"
                placeholder={t('sim.impact.measuredPh')}
                defaultValue={project.settings.residual.measured_dba ?? ''}
                onBlur={(e) => {
                  const v = e.target.value.trim() === '' ? null : Number(e.target.value);
                  set((s) => ({ ...s, residual: { ...s.residual, measured_dba: v !== null && Number.isFinite(v) ? v : null } }));
                }}
              />
            </label>
            <button className="primary" onClick={runImpact} disabled={busy === 'impact' || project.speakers.length === 0}>
              {busy === 'impact' ? t('sim.impact.busy') : useLf ? t('sim.impact.runGfpe') : t('sim.impact.run')}
            </button>
          </section>
          {impact && <ImpactView r={impact} show={showImpact} setShow={setShowImpact} onFocus={(b) => map.flyTo({ center: [b.lon, b.lat], zoom: 16.5 })} />}
        </>
      )}

      {tab === 'dim' && (
        <>
          <section className="card">
            <h2>{t('sim.dim.title')}</h2>
            {!project.dancefloor && <p className="muted small">{t('sim.dim.needDf')}</p>}
            <div className="row">
              <label className="field">
                <span>{t('sim.dim.targetA')}</span>
                <input type="number" value={project.settings.target.dba} onChange={(e) => set((s) => ({ ...s, target: { ...s.target, dba: Number(e.target.value) } }))} />
              </label>
              <label className="field">
                <span>{t('sim.dim.targetC')}</span>
                <input type="number" value={project.settings.target.dbc} onChange={(e) => set((s) => ({ ...s, target: { ...s.target, dbc: Number(e.target.value) } }))} />
              </label>
            </div>
            <button className="primary" disabled={!project.dancefloor || busy === 'sizing'} onClick={runSizing}>
              {t('sim.dim.compute')}
            </button>
            {sizing && <SizingView r={sizing} />}
          </section>
          <section className="card">
            <h2>{t('sim.opt.title')}</h2>
            <p className="muted small">{t('sim.opt.lede')}</p>
            <button className="primary" disabled={!!job || project.speakers.length === 0} onClick={runOptimize}>
              {t('sim.opt.run')}
            </button>
            {candidates && (
              <>
                <p className="small muted">{t('sim.opt.found', { count: candidates.list.length, s: num(candidates.seconds, 1) })}</p>
                <ul className="cands">
                  {candidates.list.slice(0, 5).map((c, i) => (
                    <li key={i}>
                      <div>
                        <strong>
                          {c.config === 'as_is' ? t('sim.opt.asIs') : c.config === 'cardioid' ? t('sim.opt.cardioid') : t('sim.opt.endfire')} · {c.azimuth_deg.toFixed(0)}°
                        </strong>
                        <span className="muted small">
                          {' '}
                          Δ {c.dx.toFixed(0)} / {c.dy.toFixed(0)} m
                        </span>
                        <div className="small">
                          {t('sim.opt.line', { em: signed(c.worst_emergence_dba), margin: signed(c.margin_db) })}
                          {Number.isFinite(c.dancefloor_dba) ? ` · dancefloor ${n1(c.dancefloor_dba)}/${n1(c.dancefloor_dbc)}` : ''}
                        </div>
                        {c.gfpe_margin_db !== null && (
                          <div className="small ok-text">
                            {t('sim.opt.gfpe', { em: signed(c.gfpe_worst_emergence_dba ?? 0), margin: signed(c.gfpe_margin_db) })}
                          </div>
                        )}
                      </div>
                      <button className="small" onClick={() => update((p) => ({ ...p, speakers: c.speakers }))}>
                        {t('sim.opt.apply')}
                      </button>
                    </li>
                  ))}
                </ul>
                <p className="muted small">{t('sim.opt.method')}</p>
              </>
            )}
          </section>
        </>
      )}

      {tab === 'projet' && (
        <>
          <section className="card">
            <h2>{t('sim.project.title')}</h2>
            <label className="field">
              <span>{t('zone.name')}</span>
              <input type="text" value={project.name} onChange={(e) => update((p) => ({ ...p, name: e.target.value }))} />
            </label>
            <label className="field">
              <span>{t('sim.project.program')}</span>
              <select value={project.settings.program} onChange={(e) => set((s) => ({ ...s, program: e.target.value }))}>
                {catalog.programs.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>{t('sim.project.gain', { db: signed(project.settings.master_gain_db) })}</span>
              <input type="range" min={-30} max={6} step={0.5} value={project.settings.master_gain_db} onChange={(e) => set((s) => ({ ...s, master_gain_db: Number(e.target.value) }))} />
            </label>
            <div className="row">
              <label className="field">
                <span>{t('sim.project.receiver')}</span>
                <select value={project.settings.receiver_height_m} onChange={(e) => set((s) => ({ ...s, receiver_height_m: Number(e.target.value) }))}>
                  <option value={1.5}>{num(1.5, 1)} m</option>
                  <option value={4}>4 m</option>
                </select>
              </label>
              <label className="field">
                <span>{t('sim.project.conditions')}</span>
                <select value={project.settings.condition} onChange={(e) => set((s) => ({ ...s, condition: e.target.value as 'favorable' | 'homogeneous' }))}>
                  <option value="favorable">{t('sim.project.favorable')}</option>
                  <option value="homogeneous">{t('sim.project.homogeneous')}</option>
                </select>
              </label>
            </div>
            <div className="row">
              <label className="field">
                <span>{t('sim.project.hour')}</span>
                <select value={project.settings.meteo_hour} onChange={(e) => set((s) => ({ ...s, meteo_hour: Number(e.target.value) }))}>
                  {Array.from({ length: 24 }, (_, h) => (
                    <option key={h} value={h}>
                      {String(h).padStart(2, '0')} h
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>{t('sim.project.impedance')}</span>
                <select value={project.settings.impedance_model} onChange={(e) => set((s) => ({ ...s, impedance_model: e.target.value as 'DelanyBazley' | 'Miki' }))}>
                  <option value="DelanyBazley">Delany-Bazley</option>
                  <option value="Miki">Miki</option>
                </select>
              </label>
            </div>
            <div className="row">
              <button onClick={saveAs}>{t('sim.project.saveAs')}</button>
              <button onClick={openProject}>{t('sim.project.open')}</button>
            </div>
            <p className="muted small">{t('sim.project.autosave')}</p>
          </section>
          <section className="card">
            <h2>{t('sim.ground.title')}</h2>
            <p className="muted small">{t('sim.ground.lede')}</p>
            <p className="small">
              {t('sim.ground.cliffs', { km2: n1(scene?.ground.cliffs_km2, 2) })}
              {scene?.ground.formations
                ? ` ; ${t('sim.ground.hardened', { km2: n1(scene.ground.hardened_km2, 2) })} ; ${t('sim.ground.formations')} ${scene.ground.formations
                    .map(([d, k]) => `${d} (${t(k === 'hard' ? 'sim.ground.hard' : k === 'mixed' ? 'sim.ground.mixed' : 'sim.ground.soft')})`)
                    .join(' ; ')}`
                : ` ; ${t('sim.ground.noGeology')}`}
            </p>
            {scene && (
              <p className="small">
                {t('sim.ground.relief', {
                  kind: scene.valley.valley ? t('sim.ground.valley') : t('sim.ground.noValley'),
                  tpi: signed(scene.valley.tpi_m, 0),
                  slope: n1(scene.valley.slope_deg, 1),
                  toward: Math.round(scene.valley.downslope_toward_deg),
                })}
                {scene.valley.valley ? ` — ${t('sim.ground.coldPool')}` : '.'}
              </p>
            )}
            {(() => {
              const h = scene?.ground_hours?.find((g) => g.hour === project.settings.meteo_hour);
              return h ? (
                <p className="small">
                  {t('sim.ground.state', {
                    hour: String(h.hour).padStart(2, '0'),
                    moisture: h.state.moisture != null ? `${Math.round(h.state.moisture * 100)} %` : '—',
                    rain: n1(h.state.rain_24h_mm, 1),
                    snow: h.state.snow_m != null ? `${Math.round(h.state.snow_m * 100)} cm` : '—',
                  })}{' '}
                  → {h.snow ? t('sim.ground.snowy') : `σ × ${n1(h.sigma_factor, 2)}`}
                </p>
              ) : (
                <p className="muted small">{t('sim.ground.noState')}</p>
              );
            })()}
            <div className="row">
              <label className="check">
                <input
                  type="checkbox"
                  checked={project.settings.use_ground_state !== false}
                  onChange={(e) => set((s) => ({ ...s, use_ground_state: e.target.checked }))}
                />
                {t('sim.ground.use')}
              </label>
              <button onClick={fetchGround} disabled={busy === 'ground'}>
                {busy === 'ground' ? t('sim.dl.busy') : scene?.ground_hours ? t('sim.dl.update') : t('sim.dl.download')}
              </button>
            </div>
          </section>
          <section className="card">
            <h2>{t('sim.veg.title')}</h2>
            <p className="muted small">{t('sim.veg.lede')}</p>
            {scene?.vegetation ? (
              <p className="small">
                {t('sim.veg.summary', {
                  tiles: scene.vegetation.tiles,
                  mpts: num(scene.vegetation.points / 1e6, 1),
                  km: num(scene.vegetation.radius_m / 1000, 1),
                  dense: n1(scene.vegetation.dense_foliage_km2, 2),
                  factor: n1(scene.vegetation.mean_foliage, 2),
                  rough: n1(scene.vegetation.mean_roughness_m * 100, 0),
                })}
              </p>
            ) : (
              <p className="muted small">{t('sim.veg.none')}</p>
            )}
            <div className="row">
              <label className="field">
                <span>{t('spots.radius')}</span>
                <select value={vegRadius} onChange={(e) => setVegRadius(Number(e.target.value))}>
                  {[500, 1000, 1500, 2000, 3000].map((r) => (
                    <option key={r} value={r}>
                      {num(r / 1000, 1)} km (≈ {Math.round((Math.PI * (r / 1000) ** 2 * 40) / 5) * 5} {t('units.mb')})
                    </option>
                  ))}
                </select>
              </label>
              <button onClick={fetchVegetation} disabled={!!job}>
                {scene?.vegetation ? t('sim.dl.again') : t('sim.dl.download')}
              </button>
            </div>
          </section>
          <section className="card">
            <h2>{t('sim.calib.title')}</h2>
            <p className="muted small">{t('sim.calib.lede')}</p>
            {project.settings.calibration && (
              <p className="small">
                {t('sim.calib.active', {
                  sigma: n1(project.settings.calibration.sigma_scale, 2),
                  gt: n1(project.settings.calibration.refraction.grad_t, 3),
                  gw: n1(project.settings.calibration.refraction.grad_w, 3),
                  offset: signed(project.settings.calibration.offset_db),
                  rms: n1(project.settings.calibration.rms_db, 1),
                  n: project.settings.calibration.n_points,
                })}
              </p>
            )}
            <div className="row">
              <button onClick={importMeasurements} disabled={busy === 'calibrate'}>
                {busy === 'calibrate' ? t('sim.calib.busy') : t('sim.calib.import')}
              </button>
              {project.settings.calibration && <button onClick={() => set((s) => ({ ...s, calibration: null }))}>{t('sim.calib.remove')}</button>}
            </div>
            {calib && (
              <>
                <p className="small">
                  {t('sim.calib.points', { count: calib.n_points })}
                  {calib.n_ignored > 0 ? ` ${t('sim.calib.ignored', { count: calib.n_ignored })}` : ''} — {t('sim.calib.rms')}{' '}
                  <b>
                    {n1(calib.rms_before_db, 1)} → {n1(calib.rms_after_db, 1)} dB
                  </b>{' '}
                  {t('sim.calib.bias', { db: signed(calib.bias_before_db) })}
                </p>
                <table className="small">
                  <tbody>
                    <tr>
                      <td>{t('sim.calib.sigma')}</td>
                      <td>
                        × {n1(calib.sigma_scale.map, 2)} <span className="muted">(± {n1(calib.sigma_scale.sd, 2)} oct.)</span>
                      </td>
                    </tr>
                    <tr>
                      <td>{t('sim.calib.gradT')}</td>
                      <td>
                        {n1(calib.grad_t.map, 3)} s⁻¹ <span className="muted">(± {n1(calib.grad_t.sd, 3)})</span>
                      </td>
                    </tr>
                    <tr>
                      <td>{t('sim.calib.gradW', { deg: Math.round(calib.wind_toward_deg) })}</td>
                      <td>
                        {n1(calib.grad_w.map, 3)} s⁻¹ <span className="muted">(± {n1(calib.grad_w.sd, 3)})</span>
                      </td>
                    </tr>
                    <tr>
                      <td>{t('sim.calib.offset')}</td>
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
                    {t('sim.calib.apply')}
                  </button>
                  <button onClick={() => setCalib(null)}>{t('sim.calib.hide')}</button>
                </div>
              </>
            )}
          </section>
          <section className="card">
            <h2>{t('sim.export.title', { metric: METRICS.find((m) => m.key === metric)?.label ?? '' })}</h2>
            <div className="row wrap">
              <button disabled={!resultId} onClick={() => doExport('html')}>
                {t('sim.export.html')}
              </button>
              <button disabled={!resultId} onClick={() => doExport('geotiff')}>
                GeoTIFF
              </button>
              <button disabled={!resultId} onClick={() => doExport('geojson')}>
                GeoJSON
              </button>
            </div>
            <p className="muted small">{t('sim.export.note')}</p>
          </section>
          {scene && (
            <section className="card">
              <h2>{t('sim.zone.title')}</h2>
              <dl className="kv">
                <dt>{t('sim.zone.buildings')}</dt>
                <dd>{t('sim.zone.buildingsValue', { all: scene.buildings, res: scene.residential })}</dd>
                <dt>{t('sim.zone.people')}</dt>
                <dd>{num(scene.inhabitants, 0)}</dd>
                <dt>{t('sim.zone.sensitive')}</dt>
                <dd>{scene.sensitive}</dd>
                <dt>ZNIEFF</dt>
                <dd>{scene.znieff}</dd>
                <dt>{t('sim.zone.ground')}</dt>
                <dd>{scene.landcover}</dd>
                <dt>{t('sim.zone.loaded')}</dt>
                <dd>{num(scene.load_ms, 0)} ms</dd>
              </dl>
            </section>
          )}
        </>
      )}

      <button className="ghost" onClick={onBack}>
        ← {t('sim.back')}
      </button>
    </>
  );
}

/** Bulle de survol : niveau au point et, sur un bâtiment, sa nature (BD TOPO). */
function hoverContent(v: number | null, unit: string, b: Record<string, unknown> | undefined): HTMLElement {
  const root = document.createElement('div');
  const line = (text: string, cls?: string) => {
    const d = document.createElement('div');
    d.textContent = text;
    if (cls) d.className = cls;
    root.appendChild(d);
  };
  if (b) {
    const usage = typeof b.usage === 'string' && b.usage && !b.usage.startsWith('Indifférenci') ? b.usage : null;
    const nature = typeof b.nature === 'string' && b.nature && !b.nature.startsWith('Indifférenci') ? b.nature : null;
    line(usage ?? nature ?? tr('sim.hover.building'), 'hp-title');
    const details: string[] = [];
    if (usage && nature) details.push(nature);
    const lg = Number(b.logements);
    if (Number.isFinite(lg) && lg > 0) details.push(tr('sim.hover.dwellings', { count: lg }));
    const hh = Number(b.h);
    if (Number.isFinite(hh) && hh > 0) details.push(tr('sim.hover.height', { m: Math.round(hh) }));
    if (details.length) line(details.join(' · '), 'hp-sub');
    line(b.res === true || b.res === 'true' ? tr('sim.hover.dwelling') : tr('sim.hover.nonRes'), 'hp-sub');
  }
  if (v !== null) line(`${v.toFixed(1)} ${unit}`, 'hp-db');
  return root;
}

/** Légende flottante sur la carte : couleur → niveau. */
function MapLegend({ offset, unit }: { offset: number; unit: string }) {
  return (
    <div className="map-legend" aria-label={tr('sim.legend')}>
      <div className="ml-title">{unit}</div>
      {[...PALETTE].reverse().map(([t, c], i) => (
        <div key={t} className="ml-row">
          <i style={{ background: c }} />
          <span>{i === PALETTE.length - 1 ? `< ${PALETTE[1][0] + offset}` : `${t + offset}–${t + offset + 5}`}</span>
        </div>
      ))}
    </div>
  );
}

function Legend({ offset }: { offset: number }) {
  return (
    <div className="legend-scale" aria-label={tr('sim.legend')}>
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
  const { t } = useT();
  const [manual, setManual] = useState(false);
  const num = (k: keyof SpeakerSpec, step = 0.1, min?: number, max?: number) => (
    <input type="number" step={step} min={min} max={max} value={spk[k] as number} onChange={(e) => onChange((s) => ({ ...s, [k]: Number(e.target.value) }))} />
  );
  const m = spk.model;
  return (
    <section className="card">
      <h2>{spk.name}</h2>
      <label className="field">
        <span>{t('sim.spk.model')}</span>
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
        <span>{t('sim.spk.azimuth', { deg: Math.round(spk.azimuth_deg) })}</span>
        <input type="range" min={0} max={359} value={spk.azimuth_deg} onChange={(e) => onChange((s) => ({ ...s, azimuth_deg: Number(e.target.value) }))} />
      </label>
      <div className="grid3">
        <label className="field">
          <span>{t('sim.spk.height')}</span>
          {num('height_m', 0.1, 0, 30)}
        </label>
        <label className="field">
          <span>{t('sim.spk.tilt')}</span>
          {num('tilt_deg', 1, -30, 30)}
        </label>
        <label className="field">
          <span>{t('sim.spk.delay')}</span>
          {num('delay_ms', 0.05, 0, 200)}
        </label>
        <label className="field">
          <span>{t('sim.spk.gain')}</span>
          {num('gain_db', 0.5, -40, 6)}
        </label>
        <label className="field">
          <span>{t('sim.spk.polarity')}</span>
          <button onClick={() => onChange((s) => ({ ...s, polarity: s.polarity < 0 ? 1 : -1 }))}>{spk.polarity < 0 ? t('sim.spk.inverted') : t('sim.spk.normal')}</button>
        </label>
        <label className="field">
          <span>{t('sim.spk.state')}</span>
          <button onClick={() => onChange((s) => ({ ...s, muted: !s.muted }))}>{spk.muted ? t('sim.spk.muted') : t('sim.spk.active')}</button>
        </label>
      </div>
      <button className="ghost small" onClick={() => setManual((v) => !v)}>
        {manual ? '▾' : '▸'} {t('sim.spk.manual')}
      </button>
      {manual && (
        <div className="grid3">
          {(
            [
              ['max_spl_1m', 'sim.spk.maxSpl'],
              ['f_lo', 'sim.spk.fLo'],
              ['f_hi', 'sim.spk.fHi'],
              ['rolloff_db_oct', 'sim.spk.slope'],
              ['power_w', 'sim.spk.power'],
            ] as [keyof SpeakerModel, TKey][]
          ).map(([k, label]) => (
            <label key={k} className="field">
              <span>{t(label)}</span>
              <input
                type="number"
                value={m[k] as number}
                onChange={(e) =>
                  onChange((s) => ({ ...s, model: { ...s.model, [k]: Number(e.target.value), id: `${s.model.id.replace(/-custom$/, '')}-custom`, name: s.model.name.replace(/ \(.+\)$/, '') + ` (${t('sim.spk.modified')})` } }))
                }
              />
            </label>
          ))}
        </div>
      )}
      <button className="ghost danger" onClick={onDelete}>
        {t('sim.spk.delete')}
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
  const { t, num } = useT();
  const worst = r.most_exposed;
  return (
    <>
      <section className="card">
        <h2>{t('sim.res.title')}</h2>
        <p className={r.margin_db < 0 ? 'big warn-text' : 'big ok-text'}>
          {t('sim.res.margin', { db: signed(r.margin_db) })}
          <span className="small muted"> {t('sim.res.marginNote', { db: r.thresholds_db[0] })}</span>
        </p>
        <table className="tbl">
          <thead>
            <tr>
              <th>{t('sim.res.emergence')}</th>
              <th className="num">{t('sim.zone.buildings')}</th>
              <th className="num">{t('sim.res.people')}</th>
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
          {t('sim.res.show')}
        </label>
        <p className="muted small">
          {r.method} · {t('sim.res.dwellings', { count: r.buildings.length })} · {num(r.compute_ms, 0)} ms
        </p>
      </section>
      {worst && (
        <section className="card">
          <h2>{t('sim.worst.title')}</h2>
          <dl className="kv">
            <dt>{t('sim.worst.distance')}</dt>
            <dd>{worst.distance_m.toFixed(0)} m</dd>
            <dt>{t('sim.worst.facade')}</dt>
            <dd>
              {n1(worst.level_dba)} dB(A) · {n1(worst.level_dbc)} dB(C) ({worst.height_m} m)
            </dd>
            <dt>{t('sim.worst.residual')}</dt>
            <dd>
              {n1(worst.residual_dba)} dB(A) · {n1(worst.residual_dbc)} dB(C)
            </dd>
            <dt>{t('sim.worst.emergence')}</dt>
            <dd>
              {signed(worst.emergence_dba)} dB(A) · {signed(worst.emergence_dbc)} dB(C)
            </dd>
            <dt>{t('sim.res.people')}</dt>
            <dd title={worst.inhabitants_estimated ? t('sim.worst.estimatedHint') : t('sim.worst.filosofiHint')}>
              {worst.inhabitants_estimated ? t('sim.worst.estimated', { n: num(worst.inhabitants, 1) }) : num(worst.inhabitants, 1)}
            </dd>
          </dl>
          <p className="small muted">{t('sim.worst.octaves')}</p>
          <div className="octbars">
            {worst.emergence_oct.map((e, i) => (
              <div key={i} title={`${signed(e)} dB`}>
                <div style={{ height: `${Math.min(100, Math.max(2, e * 4))}%` }} />
                <span>{['31', '63', '125', '250', '500', '1k', '2k', '4k', '8k'][i]}</span>
              </div>
            ))}
          </div>
          <button className="small" onClick={() => onFocus(worst)}>
            {t('sim.worst.center')}
          </button>
        </section>
      )}
      <section className="card">
        <h2>{t('sim.top.title')}</h2>
        <table className="tbl">
          <thead>
            <tr>
              <th>{t('sim.top.dist')}</th>
              <th className="num">dB(A)</th>
              <th className="num">{t('sim.top.emA')}</th>
              <th className="num">{t('sim.top.emC')}</th>
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
          <h2>{t('sim.sens.title')}</h2>
          {r.sensitive.map((s, i) => (
            <p key={i} className="small">
              {s.name} <span className="muted">({s.kind})</span> : {t('sim.sens.line', { db: n1(s.level_dba), em: signed(s.emergence_dba) })}
            </p>
          ))}
          {r.fauna.map((f, i) => (
            <p key={`f${i}`} className="small">
              {t('sim.sens.znieff', { level: f.level, name: f.name, km2: num(f.area_above_45_km2, 2), max: n1(f.max_dba) })}
            </p>
          ))}
        </section>
      )}
    </>
  );
}

function SizingView({ r }: { r: SizingReport }) {
  const { t, num } = useT();
  return (
    <>
      <dl className="kv">
        <dt>{t('sim.size.center')}</dt>
        <dd>
          {n1(r.center_dba)} dB(A) · {n1(r.center_dbc)} dB(C)
        </dd>
        <dt>{t('sim.size.max')}</dt>
        <dd>
          {n1(r.max_dba)} dB(A) · {n1(r.max_dbc)} dB(C)
        </dd>
        <dt>{t('sim.size.subs')}</dt>
        <dd>
          {r.subs_needed} <span className="muted">{t('sim.size.now', { n: r.subs_now })}</span>
        </dd>
        <dt>{t('sim.size.tops')}</dt>
        <dd>
          {r.tops_needed} <span className="muted">{t('sim.size.now', { n: r.tops_now })}</span>
        </dd>
        <dt>{t('sim.size.amp')}</dt>
        <dd>{num(r.power_w / 1000, 1)} kW</dd>
        <dt>{t('sim.size.headroom')}</dt>
        <dd>
          {signed(r.headroom_dba)} dB(A) · {signed(r.headroom_dbc)} dB(C)
        </dd>
        <dt>{t('sim.size.uniformity')}</dt>
        <dd>
          {t('sim.size.uniformityValue', { sd: num(r.uniformity_std_db, 1), range: num(r.uniformity_range_db, 1), n: r.samples })}
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
