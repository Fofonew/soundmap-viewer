import { describe, expect, it } from 'vitest';
import { boundsAround, circlePolygon, orthoGroups } from './geo';

describe('geo', () => {
  it('cercle fermé, rayon respecté', () => {
    const p = circlePolygon(3.09, 44.1, 5000, 64);
    const ring = p.coordinates[0];
    expect(ring.length).toBe(65);
    expect(ring[0]).toEqual(ring[64]);
    const dLat = (ring[16][1] - 44.1) * 111_320; // a = π/2 → nord
    expect(dLat).toBeCloseTo(5000, 0);
  });

  it('emprise symétrique', () => {
    const [w, , e, n] = boundsAround(3, 44, 1000);
    expect(e - 3).toBeCloseTo(3 - w, 10);
    expect(n - 44).toBeCloseTo(1000 / 111_320, 8);
  });

  it('groupes de zooms ortho', () => {
    const levels = [10, 11, 12, 13, 14, 15, 16, 17]
      .map((zoom) => ({ zoom, radius_m: 5000 }))
      .concat([{ zoom: 18, radius_m: 1000 }, { zoom: 19, radius_m: 300 }]);
    expect(orthoGroups(levels)).toEqual([
      { minzoom: 10, maxzoom: 17, radius_m: 5000 },
      { minzoom: 18, maxzoom: 18, radius_m: 1000 },
      { minzoom: 19, maxzoom: 19, radius_m: 300 },
    ]);
  });
});
