import { useT } from '../i18n';
import type { Section } from './simApi';

/** Coupe du terrain : sol, bâti, végétation, ligne de visée, niveaux par bande. */
export function SectionChart({ s }: { s: Section }) {
  const { t, num } = useT();
  const W = 300;
  const H = 140;
  const pts = s.samples;
  if (pts.length < 2) return null;
  const zs = pts.flatMap((p) => [p[1], p[2], p[3]]).concat([s.source_z, s.receiver_z]);
  const zmin = Math.min(...zs) - 5;
  const zmax = Math.max(...zs) + 5;
  const d = Math.max(s.distance, 1);
  const X = (x: number) => (x / d) * W;
  const Y = (z: number) => H - ((z - zmin) / (zmax - zmin)) * H;
  const ground = `M0,${H} ` + pts.map((p) => `L${X(p[0]).toFixed(1)},${Y(p[1]).toFixed(1)}`).join(' ') + ` L${W},${H} Z`;
  const canopy = pts.map((p) => `${X(p[0]).toFixed(1)},${Y(p[3]).toFixed(1)}`).join(' ');
  const obst = pts.filter((p) => p[2] > p[1] + 0.1);
  const maxL = Math.max(...s.bands.map((b) => b[1]));
  return (
    <div className="section">
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Coupe du terrain entre la sono et le point choisi">
        <polyline points={canopy} fill="none" stroke="#3f8f4a" strokeWidth="1" opacity="0.8" />
        <path d={ground} fill="#5b4a36" />
        {obst.map((p, i) => (
          <line key={i} x1={X(p[0])} x2={X(p[0])} y1={Y(p[1])} y2={Y(p[2])} stroke="#e9e4d8" strokeWidth="1.5" />
        ))}
        <line x1={0} y1={Y(s.source_z)} x2={W} y2={Y(s.receiver_z)} stroke="#00e5ff" strokeDasharray="4 2" strokeWidth="1" />
        <circle cx={1.5} cy={Y(s.source_z)} r={3} fill="#ff7a1a" />
        <circle cx={W - 1.5} cy={Y(s.receiver_z)} r={3} fill="#ffffff" />
      </svg>
      <p className="small">
        <strong>
          {s.dba.toFixed(1)} dB(A) · {s.dbc.toFixed(1)} dB(C)
        </strong>{' '}
        {t('sim.chart.at', { m: Math.round(s.distance) })}
      </p>
      <p className="muted small">
        {s.info.n_edges > 0 ? t('sim.chart.blocked', { count: s.info.n_edges, delta: num(s.info.delta, 2) }) : t('sim.chart.clear')} ·{' '}
        {t('sim.chart.foliage', { m: Math.round(s.info.foliage_m), sigma: Math.round(s.info.sigma_kpa) })}
      </p>
      <div className="bands" aria-label={t('sim.chart.bands')}>
        {s.bands.map(([f, l]) => (
          <div key={f} title={`${f} Hz : ${l.toFixed(1)} dB`}>
            <div style={{ height: `${Math.max(2, (100 * (l - (maxL - 60))) / 60)}%` }} />
            <span>{f >= 1000 ? `${f / 1000}k` : f}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
