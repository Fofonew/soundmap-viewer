// Calques de la carte (panneau flottant) et fiche de la parcelle cliquée.
import { useState, type ReactNode } from 'react';
import type { ParcelInfo, PoliceNear } from './api';
import { Hint } from './Hint';
import { useT } from './i18n';
import { fmtArea, OWNER_COLORS, OWNER_KINDS, ownerLabel, PCI_MIN_ZOOM, type SelectedOwner } from './ownerLayer';

export type Basemap = 'ortho' | 'plan';

function Toggle({
  on,
  onChange,
  disabled,
  swatch,
  children,
  aside,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  swatch?: ReactNode;
  children: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <label className={`layer${on ? ' on' : ''}${disabled ? ' disabled' : ''}`}>
      <input type="checkbox" checked={on} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="layer-sw" aria-hidden>
        {swatch}
      </span>
      <span className="layer-name">{children}</span>
      {aside && <span className="layer-aside">{aside}</span>}
    </label>
  );
}

export interface LayersProps {
  basemap: Basemap;
  setBasemap: (b: Basemap) => void;
  names: boolean;
  setNames: (v: boolean) => void;
  parcels: boolean;
  setParcels: (v: boolean) => void;
  parcelsStatus: { zoomOk: boolean; loading: boolean; error: string | null; visible: { total: number; pm: number } };
  police: boolean;
  setPolice: (v: boolean) => void;
  policeList: PoliceNear[] | null;
  policeErr: string | null;
  policeFrom: string;
  lidar: boolean;
  setLidar: (v: boolean) => void;
  offline: boolean;
  zoomTo: (zoom: number) => void;
  flyTo: (lon: number, lat: number) => void;
}

export function LayersPanel(p: LayersProps) {
  const { t, num } = useT();
  const [open, setOpen] = useState(true);
  return (
    <div className={`map-panel layers${open ? '' : ' closed'}`}>
      <button className="map-panel-head" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span>{t('layers.title')}</span>
        <span className="chev" aria-hidden>
          {open ? '−' : '+'}
        </span>
      </button>
      {open && (
        <div className="map-panel-body">
          <div className="segmented" role="radiogroup" aria-label={t('layers.basemap')}>
            {(['ortho', 'plan'] as Basemap[]).map((b) => (
              <button key={b} role="radio" aria-checked={p.basemap === b} className={p.basemap === b ? 'on' : ''} onClick={() => p.setBasemap(b)}>
                {b === 'ortho' ? t('layers.satellite') : t('layers.plan')}
              </button>
            ))}
          </div>

          <Toggle on={p.names && p.basemap === 'ortho'} disabled={p.basemap !== 'ortho'} onChange={p.setNames} swatch={<b className="sw-text">Aa</b>}>
            {t('layers.names')}
          </Toggle>

          <Toggle
            on={p.parcels}
            disabled={p.offline}
            onChange={p.setParcels}
            swatch={<i className="sw-quad" />}
            aside={
              <Hint label={t('layers.parcels')}>
                <p>{t('layers.parcelsHint1')}</p>
                <p className="muted">{t('layers.parcelsHint2')}</p>
              </Hint>
            }
          >
            {t('layers.parcels')}
          </Toggle>
          {p.parcels && (
            <div className="layer-detail">
              {!p.parcelsStatus.zoomOk ? (
                <button className="link" onClick={() => p.zoomTo(PCI_MIN_ZOOM + 0.5)}>
                  {t('layers.zoomForParcels')}
                </button>
              ) : (
                <p className="readout">
                  {t('layers.parcelsCount', { total: p.parcelsStatus.visible.total, pm: p.parcelsStatus.visible.pm })}
                  {p.parcelsStatus.loading && <span className="busy-dot" aria-label={t('common.searching')} />}
                </p>
              )}
              {p.parcelsStatus.error && <p className="warn-text">{p.parcelsStatus.error}</p>}
              <ul className="owner-legend">
                {OWNER_KINDS.map((k) => (
                  <li key={k}>
                    <i style={{ background: OWNER_COLORS[k] }} />
                    {ownerLabel(k)}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <Toggle on={p.police} disabled={p.offline} onChange={p.setPolice} swatch={<i className="sw-dot police" />}>
            {t('layers.police')} <span className="muted">40 km</span>
          </Toggle>
          {p.police && (
            <div className="layer-detail">
              {p.policeErr && <p className="warn-text">{p.policeErr}</p>}
              {p.policeList && (
                <ol className="near">
                  {p.policeList.slice(0, 3).map((x) => (
                    <li key={`${x.lon},${x.lat}`}>
                      <button className="link" onClick={() => p.flyTo(x.lon, x.lat)}>
                        {x.name.replace(/^Brigade de Proximité de /, 'BP ')}
                      </button>
                      <span className="num">{num(x.distance_m / 1000, 1)} km</span>
                    </li>
                  ))}
                  {p.policeList.length === 0 && <li className="muted">{t('layers.policeNone')}</li>}
                </ol>
              )}
              <p className="muted">{t('layers.policeFrom', { from: p.policeFrom })}</p>
            </div>
          )}

          <Toggle on={p.lidar} onChange={p.setLidar} swatch={<i className="sw-quad lidar" />}>
            {t('layers.lidar')}
          </Toggle>
        </div>
      )}
    </div>
  );
}

export function ParcelInspector({
  parcel,
  busy,
  error,
  selected,
  onClose,
}: {
  parcel: ParcelInfo | null;
  busy: boolean;
  error: string | null;
  selected: SelectedOwner | null;
  onClose: () => void;
}) {
  const { t } = useT();
  if (!parcel && !busy && !error) return null;
  const c = parcel?.parcel;
  const main = parcel?.owners.find((o) => o.droit.startsWith('P')) ?? parcel?.owners[0];
  return (
    <aside className="map-panel inspector" aria-live="polite">
      <div className="map-panel-head static">
        <span>{t('parcel.title')}</span>
        <button className="icon-btn" onClick={onClose} aria-label={t('common.close')}>
          ×
        </button>
      </div>
      <div className="map-panel-body">
        {busy && <p className="muted">{t('parcel.searching')}</p>}
        {error && <p className="warn-text">{error}</p>}
        {parcel && !busy && !c && <p className="muted">{t('parcel.none')}</p>}
        {parcel && !busy && c && (
          <>
            <p className="insp-title">{main ? main.denomination : t('parcel.private')}</p>
            <p className="insp-sub">
              {main ? `${main.forme} · ${main.droit.replace(/^\w+ - /, '')}` : t('parcel.privateNote')}
              {main?.siren && <span className="num"> · SIREN {main.siren}</span>}
            </p>
            <dl className="insp-kv">
              <dt>{t('parcel.commune')}</dt>
              <dd>{c.commune}</dd>
              <dt>{t('parcel.reference')}</dt>
              <dd className="num">
                {c.section} {c.numero}
              </dd>
              <dt>{t('parcel.area')}</dt>
              <dd className="num">{fmtArea(c.contenance_m2)}</dd>
              {selected?.owner && selected.count > 1 && (
                <>
                  <dt>{t('parcel.property')}</dt>
                  <dd className="num">
                    {t('parcel.propertyValue', { count: selected.count, area: fmtArea(selected.area_m2) })}
                  </dd>
                </>
              )}
              {main?.culture && (
                <>
                  <dt>{t('parcel.nature')}</dt>
                  <dd>{main.culture}</dd>
                </>
              )}
              {parcel.public_forest && (
                <>
                  <dt>{t('parcel.forest')}</dt>
                  <dd>{t('parcel.forestValue', { name: parcel.public_forest })}</dd>
                </>
              )}
            </dl>
            {parcel.owners.length > 1 && (
              <ul className="owners">
                {parcel.owners
                  .filter((o) => o !== main)
                  .map((o, i) => (
                    <li key={i}>
                      {o.denomination} <span className="muted">· {o.droit.replace(/^\w+ - /, '')}</span>
                    </li>
                  ))}
              </ul>
            )}
            {!main &&
              (parcel.owners_error ? (
                <p className="warn-text">{t('parcel.ownersError', { error: parcel.owners_error })}</p>
              ) : (
                <p className="muted small">{t('parcel.howToKnow')}</p>
              ))}
            <p className="insp-foot num">Idu {c.idu}</p>
          </>
        )}
      </div>
    </aside>
  );
}
