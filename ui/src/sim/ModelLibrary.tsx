// Bibliothèque « Mes enceintes » : saisie d'un modèle depuis sa fiche technique.
import { useState } from 'react';
import { newId, simApi, type Directivity, type SpeakerKind, type SpeakerModel } from './simApi';
import { cutoffMinus3, octaveSplFromDatasheet, parseResponse, thermalSpl } from './speakerMath';
import { t as tr, useT } from '../i18n';

interface Draft {
  id: string | null;
  name: string;
  kind: SpeakerKind;
  notes: string;
  w_cm: number;
  h_cm: number;
  d_cm: number;
  spl: number;
  peak: boolean;
  half_space: boolean;
  f_lo: number;
  f_hi: number;
  at10: boolean;
  slope: number;
  power_w: number;
  sensitivity: number | '';
  dir: Directivity['type'];
  rear_db: number;
  h_cov: number;
  v_cov: number;
  n: number;
  spacing_cm: number;
  response: string;
}

const EMPTY: Draft = {
  id: null,
  name: '',
  kind: 'sub',
  notes: '',
  w_cm: 60,
  h_cm: 65,
  d_cm: 80,
  spl: 135,
  peak: true,
  half_space: true,
  f_lo: 35,
  f_hi: 120,
  at10: false,
  slope: 24,
  power_w: 1200,
  sensitivity: '',
  dir: 'omni',
  rear_db: 15,
  h_cov: 90,
  v_cov: 50,
  n: 8,
  spacing_cm: 35,
  response: '',
};

function toModel(d: Draft): SpeakerModel {
  const f_lo = cutoffMinus3(d.f_lo, d.at10 ? 10 : 3, d.slope, 'lo');
  const f_hi = cutoffMinus3(d.f_hi, d.at10 ? 10 : 3, d.slope, 'hi');
  const directivity: Directivity =
    d.dir === 'cardioid'
      ? { type: 'cardioid', rear_rejection_db: d.rear_db }
      : d.dir === 'horn'
        ? { type: 'horn', h_cov_deg: d.h_cov, v_cov_deg: d.v_cov, f_control: 1200, radius_m: Math.max(0.05, d.w_cm / 200) }
        : d.dir === 'line'
          ? { type: 'line', n: d.n, spacing_m: d.spacing_cm / 100, h_cov_deg: d.h_cov }
          : { type: 'omni' };
  return {
    id: d.id ?? newId('perso'),
    name: d.name.trim() || tr('models.defaultName'),
    kind: d.kind,
    max_spl_1m: Math.round(octaveSplFromDatasheet({ spl_db: d.spl, peak: d.peak, half_space: d.half_space, f_lo, f_hi }) * 10) / 10,
    f_lo: Math.round(f_lo * 10) / 10,
    f_hi: Math.round(f_hi * 10) / 10,
    rolloff_db_oct: d.slope,
    directivity,
    power_w: d.power_w,
    dims: { width_m: d.w_cm / 100, height_m: d.h_cm / 100, depth_m: d.d_cm / 100 },
    response: parseResponse(d.response),
    notes: d.notes,
    custom: true,
  };
}

/** Brouillon à partir d'un modèle existant (les valeurs sont celles du modèle, déjà converties). */
function fromModel(m: SpeakerModel): Draft {
  const dir = m.directivity;
  return {
    ...EMPTY,
    id: m.id,
    name: m.name,
    kind: m.kind,
    notes: m.notes ?? '',
    w_cm: Math.round((m.dims?.width_m ?? 0.6) * 100),
    h_cm: Math.round((m.dims?.height_m ?? 0.65) * 100),
    d_cm: Math.round((m.dims?.depth_m ?? 0.8) * 100),
    spl: m.max_spl_1m,
    peak: false,
    half_space: false,
    f_lo: m.f_lo,
    f_hi: m.f_hi,
    at10: false,
    slope: m.rolloff_db_oct,
    power_w: m.power_w,
    dir: dir.type,
    rear_db: dir.type === 'cardioid' ? dir.rear_rejection_db : 15,
    h_cov: dir.type === 'horn' || dir.type === 'line' ? dir.h_cov_deg : 90,
    v_cov: dir.type === 'horn' ? dir.v_cov_deg : 50,
    n: dir.type === 'line' ? dir.n : 8,
    spacing_cm: dir.type === 'line' ? Math.round(dir.spacing_m * 100) : 35,
    response: (m.response ?? []).map(([f, v]) => `${f}\t${v}`).join('\n'),
  };
}

export function ModelLibrary({ models, onChanged }: { models: SpeakerModel[]; onChanged: () => void }) {
  const { t, num: fmt } = useT();
  const mine = models.filter((m) => m.custom);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setDraft((d) => (d ? { ...d, [k]: v } : d));
  const num = (k: keyof Draft, step = 1) => (
    <input type="number" step={step} value={draft ? (draft[k] as number) : 0} onChange={(e) => set(k, Number(e.target.value) as never)} />
  );
  const preview = draft ? toModel(draft) : null;

  const save = async () => {
    if (!draft) return;
    setErr(null);
    try {
      await simApi.saveModel(toModel(draft));
      setDraft(null);
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <section className="card">
      <h2>{t('models.title')}</h2>
      {mine.length === 0 && !draft && <p className="muted small">{t('models.empty')}</p>}
      <ul className="spk-list">
        {mine.map((m) => (
          <li key={m.id}>
            <span>
              <i className={`dot ${m.kind}`} /> {m.name}{' '}
              <span className="muted small">
                {fmt(m.max_spl_1m, 0)} dB/oct · {fmt(m.f_lo, 0)}–{fmt(m.f_hi, 0)} Hz
                {m.dims ? ` · ${Math.round(m.dims.width_m * 100)}×${Math.round(m.dims.height_m * 100)}×${Math.round(m.dims.depth_m * 100)} cm` : ''}
              </span>
            </span>
            <span>
              <button className="ghost small" onClick={() => setDraft(fromModel(m))}>
                {t('models.edit')}
              </button>
              <button
                className="ghost small danger"
                onClick={() =>
                  simApi
                    .deleteModel(m.id)
                    .then(onChanged)
                    .catch((e) => setErr(String(e)))
                }
              >
                {t('zones.delete')}
              </button>
            </span>
          </li>
        ))}
      </ul>
      {!draft && <button onClick={() => setDraft({ ...EMPTY })}>{t('models.new')}</button>}
      {draft && preview && (
        <div className="model-form">
          <label className="field">
            <span>{t('zone.name')}</span>
            <input value={draft.name} placeholder={t('models.namePh')} onChange={(e) => set('name', e.target.value)} />
          </label>
          <div className="grid3">
            <label className="field">
              <span>{t('models.type')}</span>
              <select value={draft.kind} onChange={(e) => set('kind', e.target.value as SpeakerKind)}>
                <option value="sub">{t('sim.kind.sub')}</option>
                <option value="top">{t('sim.kind.top')}</option>
                <option value="line_array">{t('sim.kind.line_array')}</option>
              </select>
            </label>
            <label className="field">
              <span>{t('sim.spk.power')}</span>
              {num('power_w', 100)}
            </label>
            <label className="field">
              <span>{t('models.sensitivity')}</span>
              <input
                type="number"
                step={0.5}
                value={draft.sensitivity}
                placeholder={t('models.optional')}
                onChange={(e) => set('sensitivity', e.target.value === '' ? '' : Number(e.target.value))}
              />
            </label>
          </div>
          <p className="small muted">{t('models.dims')}</p>
          <div className="grid3">
            <label className="field">
              <span>{t('models.width')}</span>
              {num('w_cm')}
            </label>
            <label className="field">
              <span>{t('models.height')}</span>
              {num('h_cm')}
            </label>
            <label className="field">
              <span>{t('models.depth')}</span>
              {num('d_cm')}
            </label>
          </div>
          <p className="small muted">{t('models.splTitle')}</p>
          <div className="grid3">
            <label className="field">
              <span>{t('models.spl')}</span>
              {num('spl', 0.5)}
            </label>
            <label className="field">
              <span>{t('models.value')}</span>
              <select value={draft.peak ? 'peak' : 'cont'} onChange={(e) => set('peak', e.target.value === 'peak')}>
                <option value="peak">{t('models.peak')}</option>
                <option value="cont">{t('models.cont')}</option>
              </select>
            </label>
            <label className="field">
              <span>{t('models.measure')}</span>
              <select value={draft.half_space ? 'half' : 'free'} onChange={(e) => set('half_space', e.target.value === 'half')}>
                <option value="half">{t('models.half')}</option>
                <option value="free">{t('models.free')}</option>
              </select>
            </label>
          </div>
          {draft.sensitivity !== '' && (
            <p className="small muted">
              {t('models.check', { db: fmt(thermalSpl(draft.sensitivity, draft.power_w), 1) })}
            </p>
          )}
          <div className="grid3">
            <label className="field">
              <span>{t('sim.spk.fLo')}</span>
              {num('f_lo')}
            </label>
            <label className="field">
              <span>{t('sim.spk.fHi')}</span>
              {num('f_hi')}
            </label>
            <label className="field">
              <span>{t('models.cutoffs')}</span>
              <select value={draft.at10 ? '10' : '3'} onChange={(e) => set('at10', e.target.value === '10')}>
                <option value="3">−3 dB</option>
                <option value="10">−10 dB</option>
              </select>
            </label>
            <label className="field">
              <span>{t('sim.spk.slope')}</span>
              {num('slope', 6)}
            </label>
            <label className="field">
              <span>{t('models.directivity')}</span>
              <select value={draft.dir} onChange={(e) => set('dir', e.target.value as Directivity['type'])}>
                <option value="omni">{t('models.dirOmni')}</option>
                <option value="cardioid">{t('models.dirCardioid')}</option>
                <option value="horn">{t('models.dirHorn')}</option>
                <option value="line">{t('models.dirLine')}</option>
              </select>
            </label>
            {draft.dir === 'cardioid' && (
              <label className="field">
                <span>{t('models.rear')}</span>
                {num('rear_db')}
              </label>
            )}
            {(draft.dir === 'horn' || draft.dir === 'line') && (
              <label className="field">
                <span>{t('models.covH')}</span>
                {num('h_cov', 5)}
              </label>
            )}
            {draft.dir === 'horn' && (
              <label className="field">
                <span>{t('models.covV')}</span>
                {num('v_cov', 5)}
              </label>
            )}
            {draft.dir === 'line' && (
              <>
                <label className="field">
                  <span>{t('models.elements')}</span>
                  {num('n')}
                </label>
                <label className="field">
                  <span>{t('models.spacing')}</span>
                  {num('spacing_cm')}
                </label>
              </>
            )}
          </div>
          <label className="field">
            <span>{t('models.response')}</span>
            <textarea rows={3} value={draft.response} placeholder={'30\t-6\n40\t0\n100\t0\n150\t-10'} onChange={(e) => set('response', e.target.value)} />
          </label>
          <label className="field">
            <span>{t('models.notes')}</span>
            <input value={draft.notes} onChange={(e) => set('notes', e.target.value)} />
          </label>
          <p className="small">
            {t('models.preview', { db: fmt(preview.max_spl_1m, 1), lo: fmt(preview.f_lo, 0), hi: fmt(preview.f_hi, 0) })}
            {preview.response && preview.response.length >= 2 ? ` ${t('models.previewResponse', { count: preview.response.length })}` : ''}
          </p>
          {err && <p className="small warn-text">{err}</p>}
          <div className="row">
            <button className="primary" onClick={save}>
              {t('models.save')}
            </button>
            <button className="ghost" onClick={() => setDraft(null)}>
              {t('common.cancel')}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
