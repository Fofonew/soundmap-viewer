//! Repli météo : paquets GRIB2 AROME bruts sur le S3 public Météo-France.
//!
//! Phase 0 : découverte des runs et des fichiers (listing S3, HEAD). Le
//! décodage GRIB2 et l'extraction d'une petite zone sont prévus en mode avancé
//! (~680 Mo par paquet HP1 et par tranche de 6 h).

use crate::Result;
use crate::http::{GeoClient, Service};
use tokio_util::sync::CancellationToken;

/// Tranches horaires publiées (vérifié le 17/11/2026).
pub const RANGES: [&str; 9] = [
    "00H06H", "07H12H", "13H18H", "19H24H", "25H30H", "31H36H", "37H42H", "43H48H", "49H51H",
];

/// URL d'un paquet (`paquet` : SP1–3, IP1–5, HP1–3 ; `run` : « 2026-11-17T00:00:00Z »).
pub fn file_url(base: &str, run: &str, paquet: &str, range: &str) -> String {
    format!("{base}/pnt/{run}/arome/0025/{paquet}/arome__0025__{paquet}__{range}__{run}.grib2")
}

/// Runs disponibles dont le nom commence par `day` (« 2026-11-17 »), triés.
pub fn parse_runs(xml: &str) -> Vec<String> {
    let mut runs: Vec<String> = xml
        .split("<Prefix>")
        .skip(1)
        .filter_map(|s| s.split("</Prefix>").next())
        .filter_map(|p| p.strip_prefix("pnt/")?.strip_suffix('/').map(str::to_owned))
        .filter(|r| r.ends_with('Z'))
        .collect();
    runs.sort();
    runs
}

pub async fn list_runs(
    client: &GeoClient,
    day: &str,
    cancel: &CancellationToken,
) -> Result<Vec<String>> {
    let url = format!(
        "{}/?list-type=2&prefix=pnt/{day}&delimiter=/",
        client.endpoints().arome_s3
    );
    let body = client.get(Service::AromeS3, &url, cancel).await?;
    Ok(parse_runs(&String::from_utf8_lossy(&body.bytes)))
}

/// Taille d'un paquet (HEAD) ; `None` si absent.
pub async fn file_size(
    client: &GeoClient,
    url: &str,
    cancel: &CancellationToken,
) -> Result<Option<u64>> {
    match client.head(Service::AromeS3, url, cancel).await {
        Ok(b) => Ok(b.total_len),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matches_listing() {
        assert_eq!(
            file_url("https://s3", "2026-11-17T00:00:00Z", "HP1", "19H24H"),
            "https://s3/pnt/2026-11-17T00:00:00Z/arome/0025/HP1/arome__0025__HP1__19H24H__2026-11-17T00:00:00Z.grib2"
        );
    }

    #[test]
    fn parses_s3_listing() {
        let xml = "<ListBucketResult><CommonPrefixes><Prefix>pnt/2026-11-17T03:00:00Z/</Prefix></CommonPrefixes>\
                   <CommonPrefixes><Prefix>pnt/2026-11-17T00:00:00Z/</Prefix></CommonPrefixes><Prefix>pnt/2026-11-17</Prefix></ListBucketResult>";
        assert_eq!(
            parse_runs(xml),
            vec!["2026-11-17T00:00:00Z", "2026-11-17T03:00:00Z"]
        );
    }

    #[tokio::test]
    #[ignore = "réseau : S3 Météo-France"]
    async fn live_latest_run_has_hp1() {
        let c = GeoClient::public().unwrap();
        let cancel = CancellationToken::new();
        let today = crate::pack::today_utc();
        let runs = list_runs(&c, &today, &cancel).await.unwrap();
        // Le run du jour peut ne pas être encore publié tôt le matin.
        let Some(run) = runs.first() else { return };
        let url = file_url(&c.endpoints().arome_s3, run, "HP1", "00H06H");
        let size = file_size(&c, &url, &cancel).await.unwrap();
        assert!(size.unwrap_or(0) > 100_000_000);
    }
}
