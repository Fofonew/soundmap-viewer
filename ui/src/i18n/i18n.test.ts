import { describe, expect, it } from 'vitest';
import { _dicts, FALLBACK, LANGUAGES, translate } from '.';

type Json = { [k: string]: string | Json };

function leaves(d: Json, prefix = ''): string[] {
  return Object.entries(d).flatMap(([k, v]) => (typeof v === 'string' ? [`${prefix}${k}`] : leaves(v, `${prefix}${k}.`)));
}

const vars = (s: string) => [...s.matchAll(/\{\{\s*(\w+)\s*\}\}/g)].map((m) => m[1]).sort();

describe('traductions', () => {
  it('chaque langue est détectée depuis son fichier', () => {
    expect(LANGUAGES.map((l) => l.code)).toEqual(expect.arrayContaining(['fr', 'en']));
    for (const l of LANGUAGES) expect(l.name).not.toBe(l.code);
  });

  it('chaque langue a toutes les clés du français, avec les mêmes variables', () => {
    const ref = _dicts[FALLBACK];
    const refKeys = leaves(ref);
    for (const [code, d] of Object.entries(_dicts)) {
      const keys = new Set(leaves(d));
      const missing = refKeys.filter((k) => !keys.has(k) && !/_(zero|one|two|few|many)$/.test(k));
      expect(missing, `${code} : clés manquantes`).toEqual([]);
      const extra = [...keys].filter((k) => !refKeys.includes(k) && !/_(zero|one|two|few|many|other)$/.test(k));
      expect(extra, `${code} : clés inconnues`).toEqual([]);
      for (const k of refKeys) {
        const a = k.split('.').reduce<string | Json | undefined>((o, p) => (o && typeof o === 'object' ? o[p] : undefined), ref);
        const b = k.split('.').reduce<string | Json | undefined>((o, p) => (o && typeof o === 'object' ? o[p] : undefined), d);
        if (typeof a === 'string' && typeof b === 'string') expect(vars(b), `${code} : variables de ${k}`).toEqual(vars(a));
      }
    }
  });

  it('interpolation, pluriels et repli', () => {
    expect(translate('fr', 'spots.exposed', { count: 1 })).toBe('1 habitant exposé');
    expect(translate('fr', 'spots.exposed', { count: 12 })).toBe('12 habitants exposés');
    expect(translate('fr', 'spots.exposed', { count: 0 })).toBe("Personne d'exposé");
    expect(translate('en', 'spots.exposed', { count: 2 })).toBe('2 residents exposed');
    // Nombres formatés selon la langue.
    expect(translate('fr', 'layers.policeDistance', { km: 1.5 })).toBe("1,5 km à vol d'oiseau");
    expect(translate('en', 'layers.policeDistance', { km: 1.5 })).toBe('1.5 km straight-line');
    // Langue inconnue : français.
    expect(translate('xx', 'common.close')).toBe('Fermer');
  });
});
