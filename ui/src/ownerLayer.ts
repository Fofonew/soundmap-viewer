// Carte des propriétés : parcelles du cadastre (tuiles PCI) colorées selon le
// propriétaire quand c'est une personne morale (fichiers DGFiP), cliquables.
import * as maplibregl from 'maplibre-gl';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api, type OwnerCategory, type ParcelOwner, type ParcelRef } from './api';
import { num, t } from './i18n';

export const PCI_MIN_ZOOM = 15;

export const OWNER_COLORS: Record<OwnerCategory | 'particulier', string> = {
  commune: '#22c55e',
  section: '#14b8a6',
  etat: '#3b82f6',
  public: '#a78bfa',
  agricole: '#facc15',
  forestier: '#b45309',
  societe: '#f472b6',
  particulier: 'rgba(255,255,255,0.08)',
};

export type OwnerKind = OwnerCategory | 'particulier';
export const OWNER_KINDS = Object.keys(OWNER_COLORS) as OwnerKind[];

/** Libellé traduit d'une catégorie de propriétaire. */
export function ownerLabel(k: OwnerKind): string {
  return t(`owners.cat.${k}`);
}

const SRC = { source: 'pci', sourceLayer: 'parcelle' } as const;

/** Couleur de remplissage selon l'état « cat » posé par la jointure DGFiP. */
export function ownerFillColor(): maplibregl.ExpressionSpecification {
  return [
    'match',
    ['coalesce', ['feature-state', 'cat'], 'particulier'],
    ...(Object.entries(OWNER_COLORS).filter(([k]) => k !== 'particulier').flat() as string[]),
    OWNER_COLORS.particulier,
  ] as unknown as maplibregl.ExpressionSpecification;
}

export interface SelectedOwner {
  owner: ParcelOwner | null;
  /** Parcelles visibles de ce propriétaire et leur surface cumulée. */
  count: number;
  area_m2: number;
}

function refOf(p: Record<string, unknown>): ParcelRef | null {
  const s = (k: string) => (p[k] === undefined || p[k] === null ? '' : String(p[k]));
  const idu = s('idu');
  if (!idu || !s('code_dep')) return null;
  return { idu, code_dep: s('code_dep'), code_com: s('code_com'), com_abs: s('com_abs'), section: s('section'), numero: s('numero') };
}

/** Surface lisible (m² ou ha) dans la langue courante. */
export function fmtArea(m2: number): string {
  return m2 >= 10_000 ? `${num(m2 / 10_000, 2)} ha` : `${num(Math.round(m2), 0)} m²`;
}

function hoverText(p: Record<string, unknown>, o: ParcelOwner | null | undefined): [string, string] {
  const sec = String(p.section ?? '').replace(/^0+/, '');
  const no = String(p.numero ?? '').replace(/^0+/, '');
  const sub = t('parcel.hoverSub', { commune: String(p.nom_com ?? ''), section: sec, number: no, area: fmtArea(Number(p.contenance ?? 0)) });
  if (o === undefined) return ['…', sub];
  if (o === null) return [ownerLabel('particulier'), sub];
  return [`${o.name} (${o.forme})`, sub];
}

/**
 * Branche la couche `pci-*` du style Exploration : jointure des propriétaires
 * des parcelles visibles, survol, clic (sélection de toute la propriété).
 * `canClick` : faux pendant un autre outil (placement de la sono…).
 */
export function useOwnerLayer(
  map: maplibregl.Map | null,
  enabled: boolean,
  canClick: () => boolean,
  onPick: (lngLat: [number, number]) => void,
) {
  const cache = useRef(new Map<string, ParcelOwner | null>());
  const pending = useRef(new Set<string>());
  /** Sélection : tout un propriétaire (personne morale) ou une seule parcelle. */
  const selId = useRef<string | null>(null);
  const selParcel = useRef<string | null>(null);
  const selOn = useRef(new Set<string>());
  const [zoomOk, setZoomOk] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState({ total: 0, pm: 0 });
  const [selected, setSelected] = useState<SelectedOwner | null>(null);
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const canClickRef = useRef(canClick);
  canClickRef.current = canClick;
  const onPickRef = useRef(onPick);
  onPickRef.current = onPick;

  const visibleFeatures = useCallback(() => {
    if (!map || !map.getSource('pci')) return [];
    return map.querySourceFeatures('pci', { sourceLayer: 'parcelle' });
  }, [map]);

  /** Pose les états (catégorie, sélection) sur les parcelles chargées. */
  const paint = useCallback(() => {
    if (!map || !map.getSource('pci')) return;
    const feats = visibleFeatures();
    const seen = new Set<string>();
    let pm = 0;
    let selCount = 0;
    let selArea = 0;
    const nowSel = new Set<string>();
    for (const f of feats) {
      const idu = String(f.properties?.idu ?? '');
      if (!idu || seen.has(idu)) continue;
      seen.add(idu);
      const o = cache.current.get(idu);
      if (idu === selParcel.current) {
        nowSel.add(idu);
        selCount++;
        selArea += Number(f.properties?.contenance ?? 0);
      }
      if (o) {
        pm++;
        map.setFeatureState({ ...SRC, id: idu }, { cat: o.category });
        if (selId.current && o.owner_id === selId.current) {
          nowSel.add(idu);
          selCount++;
          selArea += Number(f.properties?.contenance ?? 0);
        }
      }
    }
    for (const id of selOn.current) if (!nowSel.has(id)) map.setFeatureState({ ...SRC, id }, { sel: false });
    for (const id of nowSel) map.setFeatureState({ ...SRC, id }, { sel: true });
    selOn.current = nowSel;
    setVisible({ total: seen.size, pm });
    if (selId.current) setSelected((s) => (s ? { ...s, count: selCount, area_m2: selArea } : s));
  }, [map, visibleFeatures]);

  /** Interroge les propriétaires des parcelles visibles encore inconnues. */
  const refresh = useCallback(async () => {
    if (!map || !enabledRef.current) return;
    const z = map.getZoom() >= PCI_MIN_ZOOM;
    setZoomOk(z);
    if (!z) return;
    const todo = new Map<string, ParcelRef>();
    for (const f of visibleFeatures()) {
      const r = refOf((f.properties ?? {}) as Record<string, unknown>);
      if (r && !cache.current.has(r.idu) && !pending.current.has(r.idu)) todo.set(r.idu, r);
    }
    if (todo.size > 0) {
      for (const k of todo.keys()) pending.current.add(k);
      setLoading(true);
      try {
        const found = new Map((await api.parcelOwners([...todo.values()])).map((o) => [o.idu, o]));
        for (const k of todo.keys()) cache.current.set(k, found.get(k) ?? null);
        setError(null);
      } catch (e) {
        setError(typeof e === 'string' ? e : String(e));
      } finally {
        for (const k of todo.keys()) pending.current.delete(k);
        setLoading(false);
      }
    }
    paint();
  }, [map, visibleFeatures, paint]);

  // Rafraîchit après chaque déplacement (et chargement de tuiles).
  useEffect(() => {
    if (!map || !enabled) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const kick = () => {
      clearTimeout(t);
      t = setTimeout(() => void refresh(), 250);
    };
    const onData = (e: maplibregl.MapSourceDataEvent) => {
      if (e.sourceId === 'pci' && e.isSourceLoaded) kick();
    };
    map.on('moveend', kick);
    map.on('sourcedata', onData);
    kick();
    return () => {
      clearTimeout(t);
      map.off('moveend', kick);
      map.off('sourcedata', onData);
    };
  }, [map, enabled, refresh]);

  // Survol et clic : écouteurs par couche, rattachés à chaque instance de carte
  // (ils survivent aux changements de style).
  const paintRef = useRef(paint);
  paintRef.current = paint;
  useEffect(() => {
    if (!map) return;
    const pop = new maplibregl.Popup({ closeButton: false, closeOnClick: false, className: 'hover-pop', offset: 14 });
    const onMove = (e: maplibregl.MapLayerMouseEvent) => {
      if (!enabledRef.current) return;
      const f = e.features?.[0];
      if (!f) return;
      const p = (f.properties ?? {}) as Record<string, unknown>;
      const [title, sub] = hoverText(p, cache.current.get(String(p.idu ?? '')));
      const el = document.createElement('div');
      const t = document.createElement('div');
      t.className = 'hp-title';
      t.textContent = title;
      const s = document.createElement('div');
      s.className = 'hp-sub';
      s.textContent = sub;
      el.append(t, s);
      pop.setLngLat(e.lngLat).setDOMContent(el).addTo(map);
      if (canClickRef.current()) map.getCanvas().style.cursor = 'pointer';
    };
    const onLeave = () => {
      pop.remove();
      if (canClickRef.current()) map.getCanvas().style.cursor = '';
    };
    const onClick = (e: maplibregl.MapLayerMouseEvent) => {
      if (!enabledRef.current || !canClickRef.current()) return;
      const f = e.features?.[0];
      if (!f) return;
      const idu = String(f.properties?.idu ?? '');
      const o = cache.current.get(idu) ?? null;
      // Personne morale : toute sa propriété ; particulier : la seule parcelle.
      selId.current = o?.owner_id ?? null;
      selParcel.current = o ? null : idu;
      setSelected({ owner: o, count: 1, area_m2: Number(f.properties?.contenance ?? 0) });
      paintRef.current();
      onPickRef.current([e.lngLat.lng, e.lngLat.lat]);
    };
    map.on('mousemove', 'pci-fill', onMove);
    map.on('mouseleave', 'pci-fill', onLeave);
    map.on('click', 'pci-fill', onClick);
    return () => {
      pop.remove();
      map.off('mousemove', 'pci-fill', onMove);
      map.off('mouseleave', 'pci-fill', onLeave);
      map.off('click', 'pci-fill', onClick);
    };
  }, [map]);

  const clearSelection = useCallback(() => {
    selId.current = null;
    selParcel.current = null;
    if (map && map.getSource('pci')) for (const id of selOn.current) map.setFeatureState({ ...SRC, id }, { sel: false });
    selOn.current = new Set();
    setSelected(null);
  }, [map]);

  // Couche désactivée : on oublie la sélection.
  useEffect(() => {
    if (!enabled) clearSelection();
  }, [enabled, clearSelection]);

  return { zoomOk, loading, error, visible, selected, clearSelection };
}
