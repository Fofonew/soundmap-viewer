//! Profil météo vertical AROME via Open-Meteo (`/v1/meteofrance`), sans clé.
//!
//! T à 2/20/50/100/150 m et vent à 10/20/50/100/150 m : c'est la tranche qui
//! pilote la réfraction des basses fréquences.

use crate::http::{GeoClient, Service};
use crate::{DataError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const T_HEIGHTS: [u16; 5] = [2, 20, 50, 100, 150];
pub const WIND_HEIGHTS: [u16; 5] = [10, 20, 50, 100, 150];
pub const MODEL: &str = "meteofrance_arome_france";

fn hourly_vars() -> String {
    let mut v: Vec<String> = T_HEIGHTS
        .iter()
        .map(|h| format!("temperature_{h}m"))
        .collect();
    for h in WIND_HEIGHTS {
        v.push(format!("wind_speed_{h}m"));
        v.push(format!("wind_direction_{h}m"));
    }
    v.extend(["cloud_cover".into(), "relative_humidity_2m".into()]);
    v.join(",")
}

/// URL pour une journée (`date` au format AAAA-MM-JJ), heures locales de Paris.
pub fn forecast_url(base: &str, lat: f64, lon: f64, date: &str) -> String {
    format!(
        "{base}/v1/meteofrance?latitude={lat:.5}&longitude={lon:.5}&hourly={}&models={MODEL}\
         &start_date={date}&end_date={date}&timezone=Europe%2FParis&wind_speed_unit=ms",
        hourly_vars()
    )
}

/// Veille d'une date AAAA-MM-JJ (calendrier grégorien).
pub fn previous_day(date: &str) -> Option<String> {
    let mut it = date.split('-').map(|p| p.parse::<u32>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = |m: u32| match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    let (y, m, d) = if d > 1 {
        (y, m, d - 1)
    } else if m > 1 {
        (y, m - 1, days(m - 1))
    } else {
        (y - 1, 12, 31)
    };
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// URL de l'état du sol (humidité 0–1 cm, neige, pluie) pour `date` et la veille.
pub fn soil_url(base: &str, lat: f64, lon: f64, date: &str) -> String {
    let start = previous_day(date).unwrap_or_else(|| date.to_owned());
    format!(
        "{base}/v1/forecast?latitude={lat:.5}&longitude={lon:.5}\
         &hourly=soil_moisture_0_to_1cm,snow_depth,precipitation\
         &start_date={start}&end_date={date}&timezone=Europe%2FParis"
    )
}

/// État du sol horaire (veille + jour J).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoilState {
    pub times: Vec<String>,
    /// Humidité volumique 0–1 cm (m³/m³).
    pub soil_moisture: Vec<Option<f64>>,
    /// Hauteur de neige (m).
    pub snow_depth: Vec<Option<f64>>,
    /// Précipitations (mm/h).
    pub precipitation: Vec<Option<f64>>,
}

impl SoilState {
    /// (humidité, neige m, pluie des 24 h précédentes mm) à l'heure `time`
    /// (préfixe « AAAA-MM-JJTHH »).
    pub fn at(&self, time: &str) -> Option<(Option<f64>, Option<f64>, f64)> {
        let i = self.times.iter().position(|t| t.starts_with(time))?;
        let rain: f64 = self.precipitation
            [i.saturating_sub(23)..=i.min(self.precipitation.len().saturating_sub(1))]
            .iter()
            .flatten()
            .sum();
        Some((
            self.soil_moisture.get(i).copied().flatten(),
            self.snow_depth.get(i).copied().flatten(),
            rain,
        ))
    }
}

pub fn parse_soil(bytes: &[u8]) -> Result<SoilState> {
    let v: Value = serde_json::from_slice(bytes)?;
    let hourly = v
        .get("hourly")
        .ok_or_else(|| DataError::Parse("Open-Meteo : pas de `hourly`".into()))?;
    Ok(SoilState {
        times: hourly
            .get("time")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        soil_moisture: plain(hourly, "soil_moisture_0_to_1cm"),
        snow_depth: plain(hourly, "snow_depth"),
        precipitation: plain(hourly, "precipitation"),
    })
}

pub async fn fetch_soil(
    client: &GeoClient,
    lat: f64,
    lon: f64,
    date: &str,
    cancel: &CancellationToken,
) -> Result<SoilState> {
    let url = soil_url(&client.endpoints().open_meteo, lat, lon, date);
    parse_soil(&client.get(Service::OpenMeteo, &url, cancel).await?.bytes)
}

/// Série horaire d'une grandeur à une hauteur donnée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub height_m: u16,
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeteoForecast {
    pub model: String,
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: Option<f64>,
    pub times: Vec<String>,
    pub temperature: Vec<Series>,
    pub wind_speed: Vec<Series>,
    pub wind_direction: Vec<Series>,
    pub cloud_cover: Vec<Option<f64>>,
    pub relative_humidity: Vec<Option<f64>>,
}

/// Profil vertical à une heure donnée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerticalProfile {
    pub time: String,
    /// (hauteur m, T °C)
    pub temperature: Vec<(u16, f64)>,
    /// (hauteur m, vitesse m/s, direction °)
    pub wind: Vec<(u16, f64, Option<f64>)>,
    pub cloud_cover: Option<f64>,
    pub relative_humidity: Option<f64>,
}

fn series(hourly: &Value, var: &str, h: u16) -> Series {
    let key = format!("{var}_{h}m");
    let values = hourly
        .get(&key)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default();
    Series {
        height_m: h,
        values,
    }
}

fn plain(hourly: &Value, key: &str) -> Vec<Option<f64>> {
    hourly
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default()
}

pub fn parse(bytes: &[u8]) -> Result<MeteoForecast> {
    let v: Value = serde_json::from_slice(bytes)?;
    if v.get("error").and_then(Value::as_bool) == Some(true) {
        return Err(DataError::Parse(format!(
            "Open-Meteo : {}",
            v.get("reason").and_then(Value::as_str).unwrap_or("erreur")
        )));
    }
    let hourly = v
        .get("hourly")
        .ok_or_else(|| DataError::Parse("Open-Meteo : pas de `hourly`".into()))?;
    let times = hourly
        .get("time")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(MeteoForecast {
        model: MODEL.into(),
        latitude: v
            .get("latitude")
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN),
        longitude: v
            .get("longitude")
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN),
        elevation: v.get("elevation").and_then(Value::as_f64),
        times,
        temperature: T_HEIGHTS
            .iter()
            .map(|h| series(hourly, "temperature", *h))
            .collect(),
        wind_speed: WIND_HEIGHTS
            .iter()
            .map(|h| series(hourly, "wind_speed", *h))
            .collect(),
        wind_direction: WIND_HEIGHTS
            .iter()
            .map(|h| series(hourly, "wind_direction", *h))
            .collect(),
        cloud_cover: plain(hourly, "cloud_cover"),
        relative_humidity: plain(hourly, "relative_humidity_2m"),
    })
}

impl MeteoForecast {
    /// Profil à l'heure d'indice `i` (valeurs nulles omises).
    pub fn profile(&self, i: usize) -> Option<VerticalProfile> {
        let at = |s: &Series| s.values.get(i).copied().flatten();
        Some(VerticalProfile {
            time: self.times.get(i)?.clone(),
            temperature: self
                .temperature
                .iter()
                .filter_map(|s| at(s).map(|v| (s.height_m, v)))
                .collect(),
            wind: self
                .wind_speed
                .iter()
                .zip(&self.wind_direction)
                .filter_map(|(s, d)| at(s).map(|v| (s.height_m, v, at(d))))
                .collect(),
            cloud_cover: self.cloud_cover.get(i).copied().flatten(),
            relative_humidity: self.relative_humidity.get(i).copied().flatten(),
        })
    }

    /// Indice de l'heure `hh` (0–23) dans la journée.
    pub fn index_of_hour(&self, hh: u8) -> Option<usize> {
        let suffix = format!("T{hh:02}:00");
        self.times.iter().position(|t| t.ends_with(&suffix))
    }
}

pub async fn fetch_day(
    client: &GeoClient,
    lat: f64,
    lon: f64,
    date: &str,
    cancel: &CancellationToken,
) -> Result<(MeteoForecast, bytes::Bytes)> {
    let url = forecast_url(&client.endpoints().open_meteo, lat, lon, date);
    let body = client.get(Service::OpenMeteo, &url, cancel).await?;
    Ok((parse(&body.bytes)?, body.bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"latitude":44.1,"longitude":3.1,"elevation":357.0,"hourly":{"time":["2026-11-17T22:00","2026-11-17T23:00"],
      "temperature_2m":[3.1,2.5],"temperature_20m":[4.0,3.8],"temperature_50m":[5.2,5.0],"temperature_100m":[6.0,6.1],"temperature_150m":[6.2,6.3],
      "wind_speed_10m":[1.2,0.8],"wind_direction_10m":[200,210],"wind_speed_20m":[1.5,1.0],"wind_speed_50m":[2.4,2.0],"wind_speed_100m":[3.1,2.9],"wind_speed_150m":[3.5,null],
      "cloud_cover":[5,0],"relative_humidity_2m":[88,91]}}"#;

    #[test]
    fn parses_profile_with_inversion() {
        let f = parse(SAMPLE.as_bytes()).unwrap();
        let i = f.index_of_hour(23).unwrap();
        let p = f.profile(i).unwrap();
        assert_eq!(p.temperature.len(), 5);
        // inversion : T(150 m) > T(2 m)
        assert!(p.temperature[4].1 > p.temperature[0].1);
        // vent à 150 m nul en entrée : omis
        assert_eq!(p.wind.len(), 4);
        assert_eq!(p.wind[0], (10, 0.8, Some(210.0)));
    }

    #[test]
    fn error_payload_is_reported() {
        let e = parse(br#"{"error":true,"reason":"The service is overloaded"}"#).unwrap_err();
        assert!(e.to_string().contains("overloaded"));
    }

    #[test]
    fn url_contains_all_levels() {
        let u = forecast_url("https://api.open-meteo.com", 44.1, 3.09, "2026-11-17");
        for h in T_HEIGHTS {
            assert!(u.contains(&format!("temperature_{h}m")));
        }
        assert!(u.contains("wind_speed_150m") && u.contains("models=meteofrance_arome_france"));
    }

    #[tokio::test]
    #[ignore = "réseau : Open-Meteo"]
    async fn live_profile() {
        let c = GeoClient::public().unwrap();
        let today = crate::pack::today_utc();
        let (f, _) = fetch_day(&c, 44.10, 3.09, &today, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(f.times.len(), 24);
        let p = f.profile(22).unwrap();
        assert_eq!(p.temperature.len(), 5, "{p:?}");
    }
}

#[cfg(test)]
mod soil_tests {
    use super::*;

    #[test]
    fn previous_day_handles_boundaries() {
        assert_eq!(previous_day("2026-03-01").as_deref(), Some("2026-02-28"));
        assert_eq!(previous_day("2024-03-01").as_deref(), Some("2024-02-29"));
        assert_eq!(previous_day("2026-01-01").as_deref(), Some("2025-12-31"));
        assert_eq!(previous_day("2026-11-17").as_deref(), Some("2026-11-16"));
    }

    #[test]
    fn soil_state_at_hour_sums_24h_rain() {
        let mut times = Vec::new();
        for d in ["2026-11-16", "2026-11-17"] {
            for h in 0..24 {
                times.push(format!("{d}T{h:02}:00"));
            }
        }
        let n = times.len();
        let s = SoilState {
            times,
            soil_moisture: vec![Some(0.3); n],
            snow_depth: vec![Some(0.0); n],
            precipitation: vec![Some(1.0); n],
        };
        let (m, snow, rain) = s.at("2026-11-17T23").unwrap();
        assert_eq!((m, snow), (Some(0.3), Some(0.0)));
        assert!((rain - 24.0).abs() < 1e-9);
        assert!(s.at("2026-11-18T00").is_none());
    }
}
