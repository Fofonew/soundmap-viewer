import { describe, expect, it } from 'vitest';
import { bearing, boxFootprint, cutoffMinus3, octaveSplFromDatasheet, parseResponse, rotateAround, thermalSpl } from './speakerMath';

describe('fiche technique → modèle', () => {
  it('SPL crête demi-espace large bande → continu champ libre par octave', () => {
    // 2 × 18" : 138 dB crête demi-espace, 35–140 Hz (2 octaves) → 138 − 6 − 6 − 3 = 123.
    const l = octaveSplFromDatasheet({ spl_db: 138, peak: true, half_space: true, f_lo: 35, f_hi: 140 });
    expect(l).toBeCloseTo(123, 1);
    // Bande étroite (< 1 octave) : pas de correction de largeur.
    expect(octaveSplFromDatasheet({ spl_db: 120, peak: false, half_space: false, f_lo: 40, f_hi: 70 })).toBeCloseTo(120, 6);
  });

  it('sensibilité + puissance', () => {
    expect(thermalSpl(100, 1000)).toBeCloseTo(130, 6);
  });

  it('coupure −10 dB → −3 dB', () => {
    // Pente 24 dB/oct : 7 dB = 0,29 octave.
    expect(cutoffMinus3(30, 10, 24, 'lo')).toBeCloseTo(30 * 2 ** (7 / 24), 6);
    expect(cutoffMinus3(150, 10, 24, 'hi')).toBeLessThan(150);
    expect(cutoffMinus3(40, 3, 24, 'lo')).toBe(40);
  });

  it('réponse collée depuis un tableur', () => {
    expect(parseResponse('100\t0\n30;-6\n40 0,5\nfoo\n')).toEqual([
      [30, -6],
      [40, 0.5],
      [100, 0],
    ]);
  });
});

describe('géométrie', () => {
  it('empreinte tournée : face avant vers l\'azimut', () => {
    const p = boxFootprint(3, 44, 0.6, 0.8, 90);
    // Azimut 90° (est) : la profondeur (0,8 m) est selon x.
    const xs = p.map((q) => (q[0] - 3) * 111_320 * Math.cos((44 * Math.PI) / 180));
    expect(Math.max(...xs) - Math.min(...xs)).toBeCloseTo(0.8, 3);
  });

  it('rotation et azimut', () => {
    const c: [number, number] = [3, 44];
    const north: [number, number] = [3, 44 + 10 / 111_320];
    const r = rotateAround(north[0], north[1], c[0], c[1], 90);
    expect(bearing(c, r)).toBeCloseTo(90, 3);
    expect(bearing(c, north)).toBeCloseTo(0, 3);
  });
});
