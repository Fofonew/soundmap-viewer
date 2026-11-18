// « Comment connaître ce niveau ? » : mesure au sonomètre ou fiche technique.
import { useState } from 'react';
import { Hint } from './Hint';
import { useT } from './i18n';
import { levelFromDatasheet, levelFromMeasurement } from './levelMath';

export function LevelHelper({ onUse }: { onUse: (level: number) => void }) {
  const { t, num } = useT();
  const [measured, setMeasured] = useState(95);
  const [dist, setDist] = useState(30);
  const [sens, setSens] = useState(99);
  const [power, setPower] = useState(1000);
  const [count, setCount] = useState(4);
  const [half, setHalf] = useState(true);
  const fromMeasure = levelFromMeasurement(measured, dist);
  const fromSheet = levelFromDatasheet({ sensitivity_db: sens, power_w: power, count, half_space: half });
  const use = (v: number) => onUse(Math.round(v));

  return (
    <details className="sub">
      <summary>{t('level.title')}</summary>
      <div>
        <p className="sublabel">
          {t('level.measureTitle')}{' '}
          <Hint label={t('level.measureTitle')}>
            <p>{t('level.measureHint1')}</p>
            <p>{t('level.measureHint2')}</p>
            <p className="muted">{t('level.measureHint3')}</p>
          </Hint>
        </p>
        <div className="fields">
          <label className="field">
            <span>{t('level.measured')}</span>
            <input type="number" step={0.5} value={measured} onChange={(e) => setMeasured(Number(e.target.value))} />
          </label>
          <label className="field">
            <span>{t('level.distance')}</span>
            <input type="number" min={1} value={dist} onChange={(e) => setDist(Number(e.target.value))} />
          </label>
          <button onClick={() => use(fromMeasure)}>{t('level.use', { level: num(fromMeasure, 0) })}</button>
        </div>
        <p className="sublabel">
          {t('level.sheetTitle')}{' '}
          <Hint label={t('level.sheetTitle')}>
            <p>{t('level.sheetHint1')}</p>
            <p>{t('level.sheetHint2')}</p>
            <p>{t('level.sheetHint3')}</p>
            <p>{t('level.sheetHint4')}</p>
            <p className="muted">{t('level.sheetHint5')}</p>
          </Hint>
        </p>
        <div className="fields">
          <label className="field">
            <span>{t('level.sensitivity')}</span>
            <input type="number" step={0.5} value={sens} onChange={(e) => setSens(Number(e.target.value))} />
          </label>
          <label className="field">
            <span>{t('level.power')}</span>
            <input type="number" step={100} min={1} value={power} onChange={(e) => setPower(Number(e.target.value))} />
          </label>
          <label className="field">
            <span>{t('level.count')}</span>
            <input type="number" min={1} value={count} onChange={(e) => setCount(Number(e.target.value))} />
          </label>
        </div>
        <label className="check">
          <input type="checkbox" checked={half} onChange={(e) => setHalf(e.target.checked)} />
          {t('level.halfSpace')} <span className="muted">{t('level.halfSpaceNote')}</span>
        </label>
        <button onClick={() => use(fromSheet)}>{t('level.use', { level: num(fromSheet, 0) })}</button>
      </div>
    </details>
  );
}
