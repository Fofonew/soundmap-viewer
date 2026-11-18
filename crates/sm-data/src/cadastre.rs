//! Cadastre : parcelle en un point (Parcellaire Express IGN, WFS), forêt
//! publique (BD TOPO, ONF) et propriétaires **personnes morales** (fichiers
//! DGFiP en open data : communes, sections de commune, État, GFA, SCI…).
//!
//! Les propriétaires personnes physiques ne sont pas publiés (secret fiscal) :
//! ils s'obtiennent auprès du service de publicité foncière ou de la mairie.
//!
//! Les fichiers DGFiP sont deux zip nationaux (~250 Mo chacun) ; on lit leur
//! répertoire central par requêtes HTTP Range puis on ne télécharge que le CSV
//! du département (~3 Mo compressé).

use crate::http::{GeoClient, Service, urlencode};
use crate::{BboxL93, DataError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

/// Millésime des fichiers des personnes morales utilisés.
pub const PM_YEAR: u32 = 2024;

const PM_BASE: &str = "https://data.economie.gouv.fr/api/v2/catalog/datasets/\
fichiers-des-locaux-et-des-parcelles-des-personnes-morales/attachments";
/// Zips des parcelles (départements 01 à 60, puis 61 à 976).
pub const PM_ZIPS: [&str; 2] = [
    "fichier_des_parcelles_situation_2024_dpts_01_a_60_zip",
    "fichiers_des_parcelles_situation_2024_dpts_61_a_976_zip",
];

pub const LAYER_PARCELLE: &str = "CADASTRALPARCELS.PARCELLAIRE_EXPRESS:parcelle";
pub const LAYER_FORET_PUBLIQUE: &str = "BDTOPO_V3:foret_publique";

/// Parcelle cadastrale (Parcellaire Express).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Parcel {
    /// Identifiant unique (14 caractères), ex. `121680000C0579`.
    pub idu: String,
    pub code_dep: String,
    pub code_insee: String,
    pub code_com: String,
    pub com_abs: String,
    pub nom_com: String,
    pub section: String,
    pub numero: String,
    pub contenance_m2: f64,
    /// Contour extérieur (Lambert-93).
    pub ring: Vec<(f64, f64)>,
}

impl Parcel {
    /// Clé de jointure avec les fichiers DGFiP.
    pub fn pm_key(&self) -> String {
        pm_key(&self.code_com, &self.com_abs, &self.section, &self.numero)
    }
}

/// Clé normalisée : commune (3) + préfixe (3) + section (2) + numéro (4).
pub fn pm_key(code_com: &str, prefix: &str, section: &str, numero: &str) -> String {
    let pad = |s: &str, n: usize| {
        let t = s.trim();
        format!("{}{t}", "0".repeat(n.saturating_sub(t.len())))
    };
    format!(
        "{}{}{}{}",
        pad(code_com, 3),
        pad(prefix, 3),
        pad(section, 2),
        pad(numero, 4)
    )
}

fn wfs_point_url(base: &str, layer: &str, x: f64, y: f64, half: f64) -> String {
    let b = BboxL93::new(x - half, y - half, x + half, y + half);
    format!(
        "{base}/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES={layer}\
         &OUTPUTFORMAT=application/json&SRSNAME=EPSG:2154&BBOX={},{},{},{},EPSG:2154&COUNT=50",
        b.xmin, b.ymin, b.xmax, b.ymax
    )
}

/// Anneaux extérieurs d'un Polygon / MultiPolygon.
fn outer_rings(geom: &Value) -> Vec<Vec<(f64, f64)>> {
    let ring = |v: &Value| -> Option<Vec<(f64, f64)>> {
        let r: Vec<(f64, f64)> = v
            .as_array()?
            .iter()
            .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
            .collect();
        (r.len() >= 3).then_some(r)
    };
    let c = geom.get("coordinates");
    match geom.get("type").and_then(Value::as_str) {
        Some("Polygon") => c
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(ring)
            .into_iter()
            .collect(),
        Some("MultiPolygon") => c
            .and_then(Value::as_array)
            .map(|ps| {
                ps.iter()
                    .filter_map(|p| p.as_array()?.first().and_then(ring))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn contains(r: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        let ((xi, yi), (xj, yj)) = (r[i], r[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn features(bytes: &[u8]) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(bytes)?;
    match v.get("features") {
        Some(Value::Array(a)) => Ok(a.clone()),
        _ => Err(DataError::Parse("WFS : pas de `features`".into())),
    }
}

/// Parcelle contenant (x, y) parmi des objets WFS.
pub fn parcel_from_features(feats: &[Value], x: f64, y: f64) -> Option<Parcel> {
    feats.iter().find_map(|f| {
        let ring = outer_rings(f.get("geometry")?)
            .into_iter()
            .find(|r| contains(r, x, y))?;
        let p = f.get("properties")?;
        let s = |k: &str| match p.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        Some(Parcel {
            idu: s("idu"),
            code_dep: s("code_dep"),
            code_insee: s("code_insee"),
            code_com: s("code_com"),
            com_abs: s("com_abs"),
            nom_com: s("nom_com"),
            section: s("section"),
            numero: s("numero"),
            contenance_m2: p.get("contenance").and_then(Value::as_f64).unwrap_or(0.0),
            ring,
        })
    })
}

/// Parcelle cadastrale au point (x, y) Lambert-93.
pub async fn parcel_at(
    client: &GeoClient,
    x: f64,
    y: f64,
    cancel: &CancellationToken,
) -> Result<Option<Parcel>> {
    let url = wfs_point_url(&client.endpoints().geopf, LAYER_PARCELLE, x, y, 1.0);
    let body = client.get(Service::Wfs, &url, cancel).await?;
    Ok(parcel_from_features(&features(&body.bytes)?, x, y))
}

/// Forêt publique (domaniale, communale, sectionale…) au point, par son nom.
pub async fn public_forest_at(
    client: &GeoClient,
    x: f64,
    y: f64,
    cancel: &CancellationToken,
) -> Result<Option<String>> {
    let url = wfs_point_url(&client.endpoints().geopf, LAYER_FORET_PUBLIQUE, x, y, 1.0);
    let body = client.get(Service::Wfs, &url, cancel).await?;
    Ok(features(&body.bytes)?.iter().find_map(|f| {
        outer_rings(f.get("geometry")?)
            .iter()
            .any(|r| contains(r, x, y))
            .then(|| {
                let p = f.get("properties")?;
                p.get("toponyme")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| p.get("nature").and_then(Value::as_str))
                    .map(str::to_owned)
            })?
    }))
}

/// Droit d'une personne morale sur une parcelle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OwnerRight {
    /// Ex. « P - Propriétaire », « U - Usufruitier ».
    pub droit: String,
    pub denomination: String,
    /// Forme juridique abrégée (COM, SCI, GFA, ETAT…).
    pub forme: String,
    pub siren: String,
    /// Nature de culture (« L - Landes », « T - Terres »…).
    pub culture: String,
    /// Groupe de personne DGFiP (1 État, 2 région, 3 département, 4 commune,
    /// 5 office HLM, 9 établissement public, 0 autre personne morale…).
    #[serde(default)]
    pub groupe: String,
}

/// Catégorie de propriétaire, pour la carte des propriétés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerCategory {
    /// Commune.
    Commune,
    /// Section de commune (biens des habitants d'un hameau).
    Section,
    /// État.
    Etat,
    /// Région, département, intercommunalité, établissement public (ONF, EPIC…).
    Public,
    /// GFA, GAEC, EARL, SCEA, SAFER…
    Agricole,
    /// Groupement forestier.
    Forestier,
    /// SCI, sociétés, associations, autres personnes morales.
    Societe,
}

impl OwnerRight {
    pub fn category(&self) -> OwnerCategory {
        let f = self.forme.as_str();
        match (self.groupe.as_str(), f) {
            (_, "COLL") => OwnerCategory::Section,
            ("4", _) | (_, "COM") => OwnerCategory::Commune,
            ("1", _) | (_, "ETAT") => OwnerCategory::Etat,
            (_, "GFO") => OwnerCategory::Forestier,
            (
                _,
                "GFA" | "GFR" | "GAF" | "GAEC" | "EARL" | "SCEA" | "SAFR" | "COAG" | "CUMA" | "SCA",
            ) => OwnerCategory::Agricole,
            ("2" | "3" | "9", _)
            | (
                _,
                "DEPT" | "CCOM" | "CCAM" | "SIVU" | "SIVO" | "SYMI" | "SYMC" | "SYCO" | "EPIC"
                | "EPA",
            ) => OwnerCategory::Public,
            _ => OwnerCategory::Societe,
        }
    }

    /// Identifiant du propriétaire (SIREN, sinon dénomination).
    pub fn owner_id(&self) -> &str {
        if self.siren.is_empty() {
            &self.denomination
        } else {
            &self.siren
        }
    }
}

/// Droit principal d'une parcelle : le propriétaire (« P »), sinon le premier.
pub fn main_right(rights: &[OwnerRight]) -> Option<&OwnerRight> {
    rights
        .iter()
        .find(|r| r.droit.starts_with('P'))
        .or_else(|| rights.first())
}

/// Index des parcelles de personnes morales d'un département, par `pm_key`.
pub type PmIndex = HashMap<String, Vec<OwnerRight>>;

/// Lit un CSV DGFiP « parcelles des personnes morales » (séparateur `;`).
pub fn parse_pm_csv(text: &str) -> PmIndex {
    let mut lines = text.lines();
    let Some(head) = lines.next() else {
        return PmIndex::new();
    };
    let cols: Vec<&str> = head.split(';').collect();
    let find = |pred: &dyn Fn(&str) -> bool| cols.iter().position(|c| pred(c));
    let (Some(com), Some(pre), Some(sec), Some(num)) = (
        find(&|c| c == "Code Commune"),
        find(&|c| c.starts_with("Pr") && c.ends_with("fixe")),
        find(&|c| c == "Section"),
        find(&|c| c.contains("plan")),
    ) else {
        return PmIndex::new();
    };
    let droit = find(&|c| c.starts_with("Code droit"));
    let den = find(&|c| c.starts_with("D") && c.contains("nomination"));
    let forme = find(&|c| c.contains("abr"));
    let siren = find(&|c| c.contains("SIREN"));
    let culture = find(&|c| c == "Nature culture");
    let groupe = find(&|c| c.starts_with("Groupe personne"));
    let mut idx = PmIndex::new();
    for l in lines {
        let f: Vec<&str> = l.split(';').collect();
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).map_or("", |s| s.trim());
        if f.len() <= num {
            continue;
        }
        let key = pm_key(f[com], f[pre], f[sec], f[num]);
        let right = OwnerRight {
            droit: get(droit).to_owned(),
            denomination: get(den).to_owned(),
            forme: get(forme).to_owned(),
            siren: get(siren).to_owned(),
            culture: get(culture).to_owned(),
            groupe: get(groupe).to_owned(),
        };
        // Une ligne par subdivision fiscale : on fusionne les natures de culture.
        let v = idx.entry(key).or_default();
        if let Some(r) = v.iter_mut().find(|r| {
            r.droit == right.droit && r.denomination == right.denomination && r.siren == right.siren
        }) {
            if !right.culture.is_empty() && !r.culture.split(", ").any(|c| c == right.culture) {
                if !r.culture.is_empty() {
                    r.culture.push_str(", ");
                }
                r.culture.push_str(&right.culture);
            }
        } else {
            v.push(right);
        }
    }
    idx
}

/// Entrée du répertoire central d'un zip.
#[derive(Debug, Clone, PartialEq)]
pub struct ZipEntry {
    pub name: String,
    pub method: u16,
    pub compressed: u64,
    pub size: u64,
    pub local_offset: u64,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// Position (offset, taille) du répertoire central d'après la fin du fichier.
pub fn central_directory(tail: &[u8]) -> Result<(u64, u64)> {
    let bad = || DataError::Parse("zip : fin de répertoire introuvable".into());
    let eocd = tail
        .windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .ok_or_else(bad)?;
    let size = u64::from(u32_at(tail, eocd + 12).ok_or_else(bad)?);
    let off = u64::from(u32_at(tail, eocd + 16).ok_or_else(bad)?);
    if off != u64::from(u32::MAX) && size != u64::from(u32::MAX) {
        return Ok((off, size));
    }
    // Zip64.
    let z = tail
        .windows(4)
        .rposition(|w| w == b"PK\x06\x06")
        .ok_or_else(bad)?;
    Ok((
        u64_at(tail, z + 48).ok_or_else(bad)?,
        u64_at(tail, z + 40).ok_or_else(bad)?,
    ))
}

/// Entrées du répertoire central.
pub fn zip_entries(cd: &[u8]) -> Vec<ZipEntry> {
    let mut out = Vec::new();
    let mut p = 0;
    while u32_at(cd, p) == Some(0x0201_4b50) {
        let (Some(method), Some(csz), Some(usz), Some(fl), Some(el), Some(cl), Some(off)) = (
            u16_at(cd, p + 10),
            u32_at(cd, p + 20),
            u32_at(cd, p + 24),
            u16_at(cd, p + 28),
            u16_at(cd, p + 30),
            u16_at(cd, p + 32),
            u32_at(cd, p + 42),
        ) else {
            break;
        };
        let (fl, el, cl) = (usize::from(fl), usize::from(el), usize::from(cl));
        let name = cd
            .get(p + 46..p + 46 + fl)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        out.push(ZipEntry {
            name,
            method,
            compressed: u64::from(csz),
            size: u64::from(usz),
            local_offset: u64::from(off),
        });
        p += 46 + fl + el + cl;
    }
    out
}

/// Fichiers du département `dep` (« 12 », « 2A », « 974 ») : `…_NB_<dep><direction>.csv`.
pub fn is_department_entry(name: &str, dep: &str) -> bool {
    let Some(stem) = name
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".csv"))
        .and_then(|f| f.rsplit_once("_NB_").map(|(_, s)| s.to_owned()))
    else {
        return false;
    };
    let dep = dep.to_ascii_uppercase();
    let stem = stem.to_ascii_uppercase();
    (stem.len() == dep.len() + 1 && stem.starts_with(&dep)) || (dep.len() == 3 && stem == dep)
}

/// Décompresse une entrée à partir de son en-tête local et de ses données.
pub fn inflate_entry(method: u16, data: &[u8], size_hint: u64) -> Result<Vec<u8>> {
    match method {
        0 => Ok(data.to_vec()),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(
            data,
            usize::try_from(size_hint.max(1))
                .unwrap_or(usize::MAX)
                .min(512 << 20),
        )
        .map_err(|e| DataError::Parse(format!("zip : décompression ({e:?})"))),
        m => Err(DataError::Parse(format!("zip : méthode {m} non gérée"))),
    }
}

/// Télécharge le CSV des parcelles de personnes morales d'un département.
pub async fn fetch_pm_department(
    client: &GeoClient,
    dep: &str,
    cancel: &CancellationToken,
) -> Result<String> {
    for zip in PM_ZIPS {
        let url = format!("{PM_BASE}/{zip}");
        let total = client
            .head(Service::Dgfip, &url, cancel)
            .await?
            .total_len
            .ok_or_else(|| DataError::Parse("zip DGFiP : taille inconnue".into()))?;
        let start = total.saturating_sub(65_536);
        let tail = client
            .get_range(Service::Dgfip, &url, start, total - 1, cancel)
            .await?;
        let (cd_off, cd_len) = central_directory(&tail.bytes)?;
        let cd = client
            .get_range(Service::Dgfip, &url, cd_off, cd_off + cd_len - 1, cancel)
            .await?;
        let entries: Vec<ZipEntry> = zip_entries(&cd.bytes)
            .into_iter()
            .filter(|e| is_department_entry(&e.name, dep))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let mut text = String::new();
        for e in entries {
            let h = client
                .get_range(
                    Service::Dgfip,
                    &url,
                    e.local_offset,
                    e.local_offset + 29,
                    cancel,
                )
                .await?;
            let (Some(fl), Some(el)) = (u16_at(&h.bytes, 26), u16_at(&h.bytes, 28)) else {
                return Err(DataError::Parse("zip : en-tête local invalide".into()));
            };
            let data_start = e.local_offset + 30 + u64::from(fl) + u64::from(el);
            let data = client
                .get_range(
                    Service::Dgfip,
                    &url,
                    data_start,
                    data_start + e.compressed.max(1) - 1,
                    cancel,
                )
                .await?;
            let raw = inflate_entry(e.method, &data.bytes, e.size)?;
            let s = String::from_utf8(raw)
                .map_err(|e| DataError::Parse(format!("CSV DGFiP : UTF-8 ({e})")))?;
            if text.is_empty() {
                text = s;
            } else {
                // Fichiers suivants : sans leur ligne d'en-tête.
                text.push('\n');
                text.push_str(s.split_once('\n').map_or("", |(_, rest)| rest));
            }
        }
        return Ok(text);
    }
    Err(DataError::Invalid(format!(
        "département {dep} absent des fichiers DGFiP {PM_YEAR}"
    )))
}

/// URL de consultation du cadastre (cadastre.gouv.fr) pour une commune.
pub fn cadastre_gouv_url(code_insee: &str) -> String {
    format!(
        "https://www.cadastre.gouv.fr/scpc/rechercherPlan.do?codeCommune={}",
        urlencode(code_insee)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_match_between_ign_and_dgfip() {
        // IGN : section « 0C », com_abs « 000 » ; DGFiP : « ␣C », préfixe vide.
        assert_eq!(
            pm_key("168", "000", "0C", "0579"),
            pm_key("168", "", " C", "0579")
        );
        assert_eq!(pm_key("168", "", " C", "579"), "1680000C0579");
        assert_eq!(pm_key("1", "", "AB", "12"), "001000AB0012");
    }

    #[test]
    fn parses_pm_csv() {
        let csv = "Département;Code Direction;Code Commune;Nom de la commune;Préfixe;Section;N° plan;N° voirie;Indice de répétition;Code voie MAJIC;Code voie Rivoli;Nature voie;Nom voie;Contenance;SUF;Nature culture;Contenance;Code droit - par;N° Majic - par;N° SIREN - par;Groupe personne - par;Forme juridique - par;Forme juridique abrégée - par;Dénomination - par\n\
12;0;168;NANT;; B;0463;;;;;;;2952;;AG - Terrains d'agrément;2952;P - Propriétaire;X;211201680;4;7210;COM;COMMUNE DE NANT\n\
12;0;168;NANT;; B;0463;;;;;;;2952;;L - Landes;2952;P - Propriétaire;X;211201680;4;7210;COM;COMMUNE DE NANT\n";
        let idx = parse_pm_csv(csv);
        let o = &idx[&pm_key("168", "000", "0B", "0463")];
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].denomination, "COMMUNE DE NANT");
        assert_eq!(o[0].forme, "COM");
        assert_eq!(o[0].droit, "P - Propriétaire");
        assert_eq!(o[0].culture, "AG - Terrains d'agrément, L - Landes");
        assert_eq!(o[0].groupe, "4");
        assert_eq!(o[0].category(), OwnerCategory::Commune);
        assert_eq!(o[0].owner_id(), "211201680");
    }

    #[test]
    fn owner_categories() {
        let r = |groupe: &str, forme: &str, droit: &str| OwnerRight {
            droit: droit.into(),
            denomination: "X".into(),
            forme: forme.into(),
            siren: String::new(),
            culture: String::new(),
            groupe: groupe.into(),
        };
        assert_eq!(r("4", "COLL", "P").category(), OwnerCategory::Section);
        assert_eq!(r("1", "ETAT", "P").category(), OwnerCategory::Etat);
        assert_eq!(r("9", "EPIC", "P").category(), OwnerCategory::Public);
        assert_eq!(r("0", "GFA", "P").category(), OwnerCategory::Agricole);
        assert_eq!(r("0", "GFO", "P").category(), OwnerCategory::Forestier);
        assert_eq!(r("0", "SCI", "P").category(), OwnerCategory::Societe);
        let rights = [
            r("0", "SCI", "G - Gérant"),
            r("4", "COM", "P - Propriétaire"),
        ];
        assert_eq!(
            main_right(&rights).map(OwnerRight::category),
            Some(OwnerCategory::Commune)
        );
        assert_eq!(r("0", "SCI", "P").owner_id(), "X");
    }

    #[test]
    fn department_entries() {
        let n = "Fichier des parcelles (situation 2024)- dpts 01 à 60/PM_24_NB_120.csv";
        assert!(is_department_entry(n, "12"));
        assert!(!is_department_entry(n, "1"));
        assert!(!is_department_entry(n, "13"));
        assert!(is_department_entry("x/PM_24_NB_2A0.csv", "2a"));
        assert!(is_department_entry("x/PM_24_NB_974.csv", "974"));
    }

    /// Zip minimal en mémoire (une entrée déflatée).
    fn tiny_zip(name: &str, content: &[u8]) -> Vec<u8> {
        let data = miniz_oxide::deflate::compress_to_vec(content, 6);
        let mut z = Vec::new();
        let n = name.as_bytes();
        let le16 = |v: u16| v.to_le_bytes();
        let le32 = |v: u32| v.to_le_bytes();
        // En-tête local.
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0]);
        z.extend_from_slice(&le16(8));
        z.extend_from_slice(&[0; 8]);
        z.extend_from_slice(&le32(data.len() as u32));
        z.extend_from_slice(&le32(content.len() as u32));
        z.extend_from_slice(&le16(n.len() as u16));
        z.extend_from_slice(&le16(0));
        z.extend_from_slice(n);
        z.extend_from_slice(&data);
        let cd_off = z.len();
        // Répertoire central.
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        z.extend_from_slice(&le16(8));
        z.extend_from_slice(&[0; 8]);
        z.extend_from_slice(&le32(data.len() as u32));
        z.extend_from_slice(&le32(content.len() as u32));
        z.extend_from_slice(&le16(n.len() as u16));
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&le32(0));
        z.extend_from_slice(n);
        let cd_len = z.len() - cd_off;
        z.extend_from_slice(b"PK\x05\x06");
        z.extend_from_slice(&[0; 4]);
        z.extend_from_slice(&le16(1));
        z.extend_from_slice(&le16(1));
        z.extend_from_slice(&le32(cd_len as u32));
        z.extend_from_slice(&le32(cd_off as u32));
        z.extend_from_slice(&le16(0));
        z
    }

    #[test]
    fn reads_zip_directory_and_inflates() {
        let content = "a;b\n1;2\n".repeat(50);
        let z = tiny_zip("d/PM_24_NB_120.csv", content.as_bytes());
        let (off, len) = central_directory(&z).unwrap();
        let e = zip_entries(&z[off as usize..(off + len) as usize]);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "d/PM_24_NB_120.csv");
        let lo = e[0].local_offset as usize;
        let fl = usize::from(u16_at(&z, lo + 26).unwrap());
        let start = lo + 30 + fl;
        let raw = inflate_entry(8, &z[start..start + e[0].compressed as usize], e[0].size).unwrap();
        assert_eq!(raw, content.as_bytes());
    }

    #[test]
    fn finds_parcel_containing_point() {
        let feats = vec![
            json!({"properties": {"idu": "A", "contenance": 10},
                "geometry": {"type": "Polygon", "coordinates": [[[0,0],[10,0],[10,10],[0,10],[0,0]]]}}),
            json!({"properties": {"idu": "B", "code_com": "168", "com_abs": "000", "section": "0C",
                "numero": "0579", "contenance": 8327},
                "geometry": {"type": "MultiPolygon", "coordinates": [[[[10,0],[20,0],[20,10],[10,10],[10,0]]]]}}),
        ];
        let p = parcel_from_features(&feats, 15.0, 5.0).unwrap();
        assert_eq!(p.idu, "B");
        assert_eq!(p.pm_key(), "1680000C0579");
        assert!(parcel_from_features(&feats, 50.0, 5.0).is_none());
    }

    #[tokio::test]
    #[ignore = "réseau : Géoplateforme et data.economie.gouv.fr"]
    async fn live_parcel_and_owner() {
        let c = GeoClient::public().unwrap();
        let t = CancellationToken::new();
        let p = parcel_at(&c, 720_450.0, 6_325_450.0, &t)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.code_dep, "12");
        let csv = fetch_pm_department(&c, "12", &t).await.unwrap();
        let idx = parse_pm_csv(&csv);
        assert!(idx.len() > 100_000, "{}", idx.len());
        assert!(
            idx.values()
                .flatten()
                .any(|o| o.denomination == "COMMUNE DE NANT")
        );
    }
}
