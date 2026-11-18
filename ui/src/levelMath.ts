// Niveau d'un système ramené à 1 m (dB(C), continu, champ libre) : ce que
// l'assistant de spot attend. Ce n'est pas la sensibilité (rendement à 1 W).

/** Repère : un sub 18" (preset du simulateur), continu, champ libre, à 1 m. */
export const SUB18_DBC_1M = 119;

/**
 * Depuis une mesure au sonomètre (pondération C, Leq ou lent) à `d` mètres sur
 * terrain dégagé : on retire la divergence sphérique (20·lg d) et le renfort
 * du sol (+3 dB) que le criblage rajoute lui-même.
 */
export function levelFromMeasurement(measured_dbc: number, distance_m: number): number {
  return measured_dbc + 20 * Math.log10(Math.max(distance_m, 1)) - 3;
}

export interface SubDatasheet {
  /** Sensibilité 1 W / 1 m (dB). */
  sensitivity_db: number;
  /** Puissance continue (RMS / AES) par sub (W). */
  power_w: number;
  /** Nombre de subs groupés. */
  count: number;
  /** Sensibilité mesurée au sol (demi-espace) : −6 dB pour le champ libre. */
  half_space: boolean;
}

/**
 * Depuis la fiche technique : sensibilité + 10·lg P (niveau thermique d'un
 * sub), −6 dB si demi-espace, −3 dB de compression thermique à pleine
 * puissance, puis +20·lg N pour N subs groupés (sommation cohérente dans le
 * grave : +6 dB par doublement).
 */
export function levelFromDatasheet(d: SubDatasheet): number {
  return (
    d.sensitivity_db +
    10 * Math.log10(Math.max(d.power_w, 1)) -
    (d.half_space ? 6 : 0) -
    3 +
    20 * Math.log10(Math.max(d.count, 1))
  );
}

/** Équivalent en nombre de subs 18" groupés. */
export function sub18Equivalent(level_dbc: number): number {
  return 10 ** ((level_dbc - SUB18_DBC_1M) / 20);
}
