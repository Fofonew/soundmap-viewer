// Traductions. Une langue = un fichier `src/locales/<code>.json` (format
// compatible i18next : clés imbriquées, `{{var}}`, pluriels `_one` / `_other`).
// Le français est la référence : une clé absente d'une autre langue retombe
// dessus. Ajouter une langue : déposer le fichier, rien d'autre.
import { useSyncExternalStore } from 'react';
import fr from '../locales/fr.json';

type Json = { [k: string]: string | Json };
type Dict = typeof fr;

/** Chemins « a.b.c » des feuilles d'un dictionnaire. */
type Leaves<T, P extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${P}${K}` : Leaves<T[K], `${P}${K}.`>;
}[keyof T & string];

type PluralSuffix = 'zero' | 'one' | 'two' | 'few' | 'many' | 'other';
type Strip<K> = K extends `${infer B}_${PluralSuffix}` ? B : K;
/** Clé de traduction (les clés plurielles s'appellent sans suffixe). */
export type TKey = Strip<Exclude<Leaves<Dict>, `_meta.${string}`>>;
export type TParams = Record<string, string | number>;

const modules = import.meta.glob<Json>('../locales/*.json', { eager: true, import: 'default' });
const dicts: Record<string, Json> = {};
for (const [path, d] of Object.entries(modules)) {
  const code = path.replace(/^.*\/([^/]+)\.json$/, '$1');
  dicts[code] = d;
}

export const FALLBACK = 'fr';

/** Langues disponibles (code et nom affiché, lu dans `_meta.name`). */
export const LANGUAGES: { code: string; name: string }[] = Object.entries(dicts)
  .map(([code, d]) => ({ code, name: typeof d._meta === 'object' && typeof d._meta.name === 'string' ? d._meta.name : code }))
  .sort((a, b) => a.name.localeCompare(b.name));

function lookup(d: Json | undefined, path: string): string | undefined {
  let cur: string | Json | undefined = d;
  for (const part of path.split('.')) {
    if (cur === undefined || typeof cur === 'string') return undefined;
    cur = cur[part];
  }
  return typeof cur === 'string' ? cur : undefined;
}

function pick(code: string): string {
  if (dicts[code]) return code;
  const base = code.split('-')[0];
  return dicts[base] ? base : FALLBACK;
}

function initial(): string {
  try {
    const saved = localStorage.getItem('soundmap.lang');
    if (saved && dicts[saved]) return saved;
  } catch {
    // stockage indisponible
  }
  return pick(typeof navigator === 'undefined' ? FALLBACK : navigator.language);
}

let lang = initial();
const listeners = new Set<() => void>();

export function getLang(): string {
  return lang;
}

export function setLang(code: string) {
  lang = pick(code);
  try {
    localStorage.setItem('soundmap.lang', lang);
  } catch {
    // préférence non mémorisée
  }
  if (typeof document !== 'undefined') document.documentElement.lang = lang;
  for (const l of listeners) l();
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

const plurals = new Map<string, Intl.PluralRules>();
function pluralOf(code: string, n: number): PluralSuffix {
  let r = plurals.get(code);
  if (!r) {
    r = new Intl.PluralRules(code);
    plurals.set(code, r);
  }
  return r.select(n) as PluralSuffix;
}

/** Traduit `key` dans la langue `code` (repli sur le français, puis sur la clé). */
export function translate(code: string, key: TKey, params?: TParams): string {
  const count = params?.count;
  const candidates =
    typeof count === 'number' ? [`${key}_${count === 0 ? 'zero' : pluralOf(code, count)}`, `${key}_${pluralOf(code, count)}`, `${key}_other`, key] : [key];
  let s: string | undefined;
  for (const d of [dicts[code], dicts[FALLBACK]]) {
    for (const k of candidates) {
      s = lookup(d, k);
      if (s !== undefined) break;
    }
    if (s !== undefined) break;
  }
  if (s === undefined) return key;
  if (!params) return s;
  return s.replace(/\{\{\s*(\w+)\s*\}\}/g, (m, name: string) => {
    const v = params[name];
    if (v === undefined) return m;
    return typeof v === 'number' ? formatNumber(code, v) : v;
  });
}

/** Traduction dans la langue courante (hors composants React). */
export function t(key: TKey, params?: TParams): string {
  return translate(lang, key, params);
}

function formatNumber(code: string, v: number, digits?: number): string {
  return new Intl.NumberFormat(code, digits === undefined ? { maximumFractionDigits: 2 } : { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(v);
}

/** Nombre formaté selon la langue (séparateur décimal, milliers). */
export function num(v: number, digits?: number): string {
  return formatNumber(lang, v, digits);
}

/** Hook : se re-rend au changement de langue. */
export function useT() {
  const code = useSyncExternalStore(subscribe, getLang, getLang);
  return {
    lang: code,
    t: (key: TKey, params?: TParams) => translate(code, key, params),
    num: (v: number, digits?: number) => formatNumber(code, v, digits),
  };
}

/** Pour les tests : dictionnaires chargés. */
export const _dicts = dicts;

if (typeof document !== 'undefined') document.documentElement.lang = lang;
