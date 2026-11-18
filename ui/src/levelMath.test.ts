import { describe, expect, it } from 'vitest';
import { levelFromDatasheet, levelFromMeasurement, sub18Equivalent } from './levelMath';

describe('niveau du système à 1 m', () => {
  it('depuis une mesure', () => {
    // 95 dB(C) à 30 m → 95 + 29,5 − 3.
    expect(levelFromMeasurement(95, 30)).toBeCloseTo(121.54, 2);
    expect(levelFromMeasurement(110, 1)).toBeCloseTo(107, 6);
  });

  it('depuis la fiche technique', () => {
    // Sub 18" : 100 dB demi-espace, 1000 W → 100 + 30 − 6 − 3 = 121.
    const one = levelFromDatasheet({ sensitivity_db: 100, power_w: 1000, count: 1, half_space: true });
    expect(one).toBeCloseTo(121, 6);
    // 4 subs groupés : +12 dB.
    expect(levelFromDatasheet({ sensitivity_db: 100, power_w: 1000, count: 4, half_space: true }) - one).toBeCloseTo(12.04, 2);
  });

  it('équivalent en subs 18"', () => {
    expect(sub18Equivalent(119)).toBeCloseTo(1, 6);
    expect(sub18Equivalent(131)).toBeCloseTo(3.98, 2);
  });
});
