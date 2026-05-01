//! EvaporChain Oracle — Real-world sensor data ingestion from major data providers.
//!
//! Pulls live data from NASA, NOAA, USGS, OpenSky, Bitcoin mempool, and CoinGecko,
//! then submits each reading as a decaying on-chain object with appropriate energy
//! and half-life parameters.
//!
//! Usage:
//!   evaporchain-oracle --node http://100.119.53.101:8080
//!   evaporchain-oracle --node http://localhost:8080 --sources all
//!   evaporchain-oracle --node http://localhost:8080 --sources nasa,usgs,bitcoin

use chrono::Utc;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{error, info, warn};

// ─────────────────────── Configuration ─────────────────────────────────

/// Generate a deterministic object ID from source + key.
fn _object_id(source: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update(b":");
    hasher.update(key.as_bytes());
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(&hash[..20]))
}

/// Generate a unique object ID with timestamp to avoid collisions.
fn unique_object_id(source: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update(b":");
    hasher.update(key.as_bytes());
    hasher.update(b":");
    hasher.update(Utc::now().timestamp_millis().to_le_bytes());
    let mut rng = rand::thread_rng();
    hasher.update(rng.gen::<u64>().to_le_bytes());
    let hash = hasher.finalize();
    format!("0x{}", hex::encode(&hash[..20]))
}

// ─────────────────────── API Types ──────────────────────────────────────

#[derive(Serialize)]
struct OracleIngestPayload {
    source: String,
    object_id: String,
    energy: u64,
    half_life: u64,
    data: String,
}

#[derive(Deserialize, Debug)]
struct TxResult {
    success: bool,
    message: String,
}

// ─────────────────────── Data Source Types ───────────────────────────────

// NASA ISS
#[derive(Deserialize, Debug)]
struct IssResponse {
    iss_position: IssPosition,
    timestamp: u64,
}

#[derive(Deserialize, Debug)]
struct IssPosition {
    latitude: String,
    longitude: String,
}

// USGS Earthquakes
#[derive(Deserialize, Debug)]
struct UsgsResponse {
    features: Vec<UsgsFeature>,
}

#[derive(Deserialize, Debug)]
struct UsgsFeature {
    properties: UsgsProperties,
    geometry: UsgsGeometry,
}

#[derive(Deserialize, Debug)]
struct UsgsProperties {
    mag: Option<f64>,
    place: Option<String>,
    #[allow(dead_code)]
    time: Option<u64>,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    event_type: Option<String>,
}

#[derive(Deserialize, Debug)]
struct UsgsGeometry {
    coordinates: Vec<f64>,
}

// NOAA Solar Wind
// The solar wind endpoint returns a JSON array of arrays:
// [["time_tag","density","speed","temperature"], ["2025-01-01 00:00:00.000","5.2","400.1","100000"], ...]

// OpenSky Network
#[derive(Deserialize, Debug)]
struct OpenSkyResponse {
    time: u64,
    states: Option<Vec<Vec<serde_json::Value>>>,
}

// Bitcoin Mempool
#[derive(Deserialize, Debug)]
struct MempoolStats {
    count: u64,
    vsize: u64,
    total_fee: f64,
}

#[derive(Deserialize, Debug)]
struct MempoolFees {
    #[serde(rename = "fastestFee")]
    fastest_fee: u64,
    #[serde(rename = "halfHourFee")]
    half_hour_fee: u64,
    #[serde(rename = "hourFee")]
    hour_fee: u64,
    #[serde(rename = "economyFee")]
    #[allow(dead_code)]
    economy_fee: u64,
    #[serde(rename = "minimumFee")]
    #[allow(dead_code)]
    minimum_fee: u64,
}

// CoinGecko
// Returns: {"bitcoin":{"usd":60000,"usd_24h_change":-1.5},...}

// NOAA Weather
#[derive(Deserialize, Debug)]
struct NoaaObservation {
    properties: NoaaObsProperties,
}

#[derive(Deserialize, Debug)]
struct NoaaObsProperties {
    temperature: Option<NoaaMeasurement>,
    #[serde(rename = "windSpeed")]
    wind_speed: Option<NoaaMeasurement>,
    #[serde(rename = "barometricPressure")]
    barometric_pressure: Option<NoaaMeasurement>,
    #[serde(rename = "relativeHumidity")]
    #[allow(dead_code)]
    relative_humidity: Option<NoaaMeasurement>,
    #[serde(rename = "textDescription")]
    text_description: Option<String>,
}

#[derive(Deserialize, Debug)]
struct NoaaMeasurement {
    value: Option<f64>,
    #[serde(rename = "unitCode")]
    #[allow(dead_code)]
    unit_code: Option<String>,
}

// ─────────────────────── Oracle Core ────────────────────────────────────

struct Oracle {
    client: Client,
    node_url: String,
    stats: OracleStats,
}

struct OracleStats {
    total_submitted: u64,
    total_accepted: u64,
    total_failed: u64,
}

impl Oracle {
    fn new(node_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("build HTTP client"),
            node_url: node_url.trim_end_matches('/').to_string(),
            stats: OracleStats {
                total_submitted: 0,
                total_accepted: 0,
                total_failed: 0,
            },
        }
    }

    async fn submit_object(
        &mut self,
        source: &str,
        key: &str,
        energy: u64,
        half_life: u64,
        data: &str,
    ) -> bool {
        let payload = OracleIngestPayload {
            source: source.to_string(),
            object_id: unique_object_id(source, key),
            energy,
            half_life,
            data: data.to_string(),
        };

        self.stats.total_submitted += 1;

        let url = format!("{}/api/oracle/ingest", self.node_url);
        match self.client.post(&url).json(&payload).send().await {
            Ok(resp) => match resp.json::<TxResult>().await {
                Ok(result) => {
                    if result.success {
                        self.stats.total_accepted += 1;
                        true
                    } else {
                        warn!("[{}] Rejected: {}", source, result.message);
                        self.stats.total_failed += 1;
                        false
                    }
                }
                Err(e) => {
                    warn!("[{}] Parse error: {}", source, e);
                    self.stats.total_failed += 1;
                    false
                }
            },
            Err(e) => {
                error!("[{}] Network error: {}", source, e);
                self.stats.total_failed += 1;
                false
            }
        }
    }

    // ── NASA: ISS Real-Time Position ──

    async fn poll_nasa_iss(&mut self) {
        let url = "http://api.open-notify.org/iss-now.json";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<IssResponse>().await {
                Ok(iss) => {
                    let data = format!(
                        "{{\"source\":\"NASA\",\"type\":\"ISS_POSITION\",\"lat\":{},\"lon\":{},\"timestamp\":{}}}",
                        iss.iss_position.latitude, iss.iss_position.longitude, iss.timestamp
                    );
                    // ISS position: high energy (important), short half-life (stale fast)
                    if self
                        .submit_object("nasa:iss", "position", 5000, 30, &data)
                        .await
                    {
                        info!(
                            "\x1b[34m[NASA]\x1b[0m ISS at ({}, {})",
                            iss.iss_position.latitude, iss.iss_position.longitude
                        );
                    }
                }
                Err(e) => warn!("[NASA] ISS parse error: {}", e),
            },
            Err(e) => warn!("[NASA] ISS fetch error: {}", e),
        }
    }

    // ── USGS: Global Earthquakes ──

    async fn poll_usgs_earthquakes(&mut self) {
        let url = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_hour.geojson";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<UsgsResponse>().await {
                Ok(quakes) => {
                    let count = quakes.features.len();
                    for quake in quakes.features.iter().take(5) {
                        let mag = quake.properties.mag.unwrap_or(0.0);
                        let place = quake.properties.place.as_deref().unwrap_or("Unknown");
                        let coords = &quake.geometry.coordinates;
                        let (lon, lat, depth) = (
                            coords.first().copied().unwrap_or(0.0),
                            coords.get(1).copied().unwrap_or(0.0),
                            coords.get(2).copied().unwrap_or(0.0),
                        );

                        // Energy scales with magnitude (exponential)
                        let energy = (1000.0 * (10.0_f64).powf(mag / 2.0)) as u64;
                        // Larger quakes stay relevant longer
                        let half_life = if mag >= 6.0 {
                            3600
                        } else if mag >= 4.0 {
                            600
                        } else {
                            120
                        };

                        let data = format!(
                            "{{\"source\":\"USGS\",\"type\":\"EARTHQUAKE\",\"magnitude\":{:.1},\"place\":\"{}\",\"lat\":{:.4},\"lon\":{:.4},\"depth_km\":{:.1}}}",
                            mag, place.replace('"', "'"), lat, lon, depth
                        );
                        if self
                            .submit_object(
                                "usgs:quake",
                                &format!("m{:.0}", mag * 10.0),
                                energy,
                                half_life,
                                &data,
                            )
                            .await
                        {
                            info!(
                                "\x1b[31m[USGS]\x1b[0m M{:.1} earthquake — {} (depth {:.0}km)",
                                mag, place, depth
                            );
                        }
                    }
                    if count > 0 {
                        info!(
                            "\x1b[31m[USGS]\x1b[0m {} earthquakes in the last hour",
                            count
                        );
                    }
                }
                Err(e) => warn!("[USGS] Parse error: {}", e),
            },
            Err(e) => warn!("[USGS] Fetch error: {}", e),
        }
    }

    // ── NOAA: Solar Wind Plasma ──

    async fn poll_noaa_solar_wind(&mut self) {
        let url = "https://services.swpc.noaa.gov/products/solar-wind/plasma-2-hour.json";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<Vec<Vec<String>>>().await {
                Ok(rows) => {
                    // Skip header row, get the latest reading
                    if let Some(latest) = rows.last() {
                        if latest.len() >= 4 {
                            let time_tag = &latest[0];
                            let density = &latest[1]; // protons/cm³
                            let speed = &latest[2]; // km/s
                            let temperature = &latest[3]; // Kelvin

                            let data = format!(
                                "{{\"source\":\"NOAA_SWPC\",\"type\":\"SOLAR_WIND\",\"time\":\"{}\",\"density_pcm3\":{},\"speed_kms\":{},\"temperature_K\":{}}}",
                                time_tag, density, speed, temperature
                            );
                            // Solar wind: moderate energy, decays in minutes
                            if self
                                .submit_object("noaa:solar", "wind", 3000, 120, &data)
                                .await
                            {
                                info!(
                                    "\x1b[33m[NOAA]\x1b[0m Solar wind: density={}p/cm³ speed={}km/s temp={}K",
                                    density, speed, temperature
                                );
                            }
                        }
                    }
                }
                Err(e) => warn!("[NOAA] Solar wind parse error: {}", e),
            },
            Err(e) => warn!("[NOAA] Solar wind fetch error: {}", e),
        }
    }

    // ── NOAA: Geomagnetic Kp Index ──

    async fn poll_noaa_kp_index(&mut self) {
        let url = "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<Vec<Vec<String>>>().await {
                Ok(rows) => {
                    if let Some(latest) = rows.last() {
                        if latest.len() >= 2 {
                            let time_tag = &latest[0];
                            let kp = &latest[1];

                            let kp_val: f64 = kp.parse().unwrap_or(0.0);
                            // Kp >= 5 is geomagnetic storm
                            let energy = if kp_val >= 7.0 {
                                50000 // Severe storm
                            } else if kp_val >= 5.0 {
                                20000 // Storm
                            } else {
                                2000 // Quiet
                            };

                            let data = format!(
                                "{{\"source\":\"NOAA_SWPC\",\"type\":\"KP_INDEX\",\"time\":\"{}\",\"kp\":{},\"storm\":{}}}",
                                time_tag, kp, kp_val >= 5.0
                            );
                            if self
                                .submit_object("noaa:kp", "index", energy, 300, &data)
                                .await
                            {
                                info!(
                                    "\x1b[33m[NOAA]\x1b[0m Kp index: {} {}",
                                    kp,
                                    if kp_val >= 5.0 {
                                        "⚡ GEOMAGNETIC STORM"
                                    } else {
                                        "(quiet)"
                                    }
                                );
                            }
                        }
                    }
                }
                Err(e) => warn!("[NOAA] Kp parse error: {}", e),
            },
            Err(e) => warn!("[NOAA] Kp fetch error: {}", e),
        }
    }

    // ── NOAA: US Weather Station ──

    async fn poll_noaa_weather(&mut self, station: &str) {
        let url = format!(
            "https://api.weather.gov/stations/{}/observations/latest",
            station
        );
        match self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "EvaporChain-Oracle/1.0 (satyawansinghinuk@gmail.com)",
            )
            .send()
            .await
        {
            Ok(resp) => match resp.json::<NoaaObservation>().await {
                Ok(obs) => {
                    let temp = obs
                        .properties
                        .temperature
                        .and_then(|t| t.value)
                        .map(|c| format!("{:.1}", c))
                        .unwrap_or_else(|| "null".to_string());
                    let wind = obs
                        .properties
                        .wind_speed
                        .and_then(|w| w.value)
                        .map(|s| format!("{:.1}", s))
                        .unwrap_or_else(|| "null".to_string());
                    let pressure = obs
                        .properties
                        .barometric_pressure
                        .and_then(|p| p.value)
                        .map(|p| format!("{:.0}", p))
                        .unwrap_or_else(|| "null".to_string());
                    let desc = obs.properties.text_description.unwrap_or_default();

                    let data = format!(
                        "{{\"source\":\"NOAA_NWS\",\"type\":\"WEATHER\",\"station\":\"{}\",\"temp_c\":{},\"wind_kph\":{},\"pressure_pa\":{},\"description\":\"{}\"}}",
                        station, temp, wind, pressure, desc.replace('"', "'")
                    );
                    if self
                        .submit_object("noaa:weather", station, 2000, 300, &data)
                        .await
                    {
                        info!(
                            "\x1b[33m[NOAA]\x1b[0m {} weather: {}°C, wind {}kph — {}",
                            station, temp, wind, desc
                        );
                    }
                }
                Err(e) => warn!("[NOAA] Weather parse error: {}", e),
            },
            Err(e) => warn!("[NOAA] Weather fetch error: {}", e),
        }
    }

    // ── OpenSky: Live Aircraft ──

    async fn poll_opensky(&mut self) {
        // UK airspace bounding box
        let url = "https://opensky-network.org/api/states/all?lamin=49&lomin=-8&lamax=61&lomax=2";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<OpenSkyResponse>().await {
                Ok(sky) => {
                    let states = sky.states.unwrap_or_default();
                    let total = states.len();

                    // Submit top 5 aircraft by altitude
                    let mut aircraft: Vec<_> = states
                        .iter()
                        .filter_map(|s| {
                            let callsign = s.get(1)?.as_str().unwrap_or("").trim().to_string();
                            let country = s.get(2)?.as_str().unwrap_or("?").to_string();
                            let lon = s.get(5)?.as_f64()?;
                            let lat = s.get(6)?.as_f64()?;
                            let alt = s.get(7)?.as_f64().unwrap_or(0.0);
                            let velocity = s.get(9)?.as_f64().unwrap_or(0.0);
                            Some((callsign, country, lat, lon, alt, velocity))
                        })
                        .collect();
                    aircraft
                        .sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

                    for (callsign, country, lat, lon, alt, velocity) in aircraft.iter().take(5) {
                        let data = format!(
                            "{{\"source\":\"OpenSky\",\"type\":\"AIRCRAFT\",\"callsign\":\"{}\",\"country\":\"{}\",\"lat\":{:.4},\"lon\":{:.4},\"altitude_m\":{:.0},\"velocity_ms\":{:.1}}}",
                            callsign, country, lat, lon, alt, velocity
                        );
                        // Aircraft position: stale very fast
                        if self
                            .submit_object("opensky:aircraft", callsign, 2000, 30, &data)
                            .await
                        {
                            info!(
                                "\x1b[36m[OpenSky]\x1b[0m {} ({}) at {:.0}m, {:.0}m/s",
                                callsign, country, alt, velocity
                            );
                        }
                    }

                    // Submit airspace summary
                    let summary = format!(
                        "{{\"source\":\"OpenSky\",\"type\":\"AIRSPACE_SUMMARY\",\"region\":\"UK\",\"total_aircraft\":{},\"timestamp\":{}}}",
                        total, sky.time
                    );
                    self.submit_object("opensky:summary", "uk", 3000, 60, &summary)
                        .await;
                    info!("\x1b[36m[OpenSky]\x1b[0m {} aircraft in UK airspace", total);
                }
                Err(e) => warn!("[OpenSky] Parse error: {}", e),
            },
            Err(e) => warn!("[OpenSky] Fetch error: {}", e),
        }
    }

    // ── Bitcoin Mempool ──

    async fn poll_bitcoin_mempool(&mut self) {
        // Mempool stats
        let stats_url = "https://mempool.space/api/mempool";
        let fees_url = "https://mempool.space/api/v1/fees/recommended";

        let stats: Option<MempoolStats> = match self.client.get(stats_url).send().await {
            Ok(r) => r.json().await.ok(),
            Err(_) => None,
        };
        let fees: Option<MempoolFees> = match self.client.get(fees_url).send().await {
            Ok(r) => r.json().await.ok(),
            Err(_) => None,
        };

        if let Some(stats) = stats {
            let fee_str = fees
                .as_ref()
                .map(|f| {
                    format!(
                        ",\"fastest_sat_vb\":{},\"half_hour_sat_vb\":{},\"hour_sat_vb\":{}",
                        f.fastest_fee, f.half_hour_fee, f.hour_fee
                    )
                })
                .unwrap_or_default();

            let data = format!(
                "{{\"source\":\"Bitcoin\",\"type\":\"MEMPOOL\",\"unconfirmed_txs\":{},\"vsize_bytes\":{},\"total_fee_btc\":{:.8}{}}}",
                stats.count, stats.vsize, stats.total_fee / 100_000_000.0, fee_str
            );
            // Mempool state: changes every second
            if self
                .submit_object("bitcoin:mempool", "stats", 4000, 30, &data)
                .await
            {
                info!(
                    "\x1b[35m[Bitcoin]\x1b[0m Mempool: {} unconfirmed txs, {:.2} MB, fastest fee: {} sat/vB",
                    stats.count,
                    stats.vsize as f64 / 1_000_000.0,
                    fees.as_ref().map_or(0, |f| f.fastest_fee)
                );
            }
        }
    }

    // ── CoinGecko: Crypto Prices ──

    async fn poll_coingecko(&mut self) {
        let url = "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,ethereum,solana&vs_currencies=usd,gbp&include_24hr_change=true&include_market_cap=true";
        match self.client.get(url).send().await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(prices) => {
                    for coin in &["bitcoin", "ethereum", "solana"] {
                        if let Some(data_obj) = prices.get(coin) {
                            let usd = data_obj.get("usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let gbp = data_obj.get("gbp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let change = data_obj
                                .get("usd_24h_change")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            let mcap = data_obj
                                .get("usd_market_cap")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);

                            let data = format!(
                                "{{\"source\":\"CoinGecko\",\"type\":\"CRYPTO_PRICE\",\"coin\":\"{}\",\"usd\":{:.2},\"gbp\":{:.2},\"change_24h\":{:.2},\"market_cap\":{:.0}}}",
                                coin, usd, gbp, change, mcap
                            );
                            // Price data: moderate decay
                            if self
                                .submit_object("coingecko:price", coin, 3000, 60, &data)
                                .await
                            {
                                let arrow = if change >= 0.0 {
                                    "\x1b[32m+"
                                } else {
                                    "\x1b[31m"
                                };
                                info!(
                                    "\x1b[35m[CoinGecko]\x1b[0m {}: ${:.2} ({}${:.2}%\x1b[0m)",
                                    coin, usd, arrow, change
                                );
                            }
                        }
                    }
                }
                Err(e) => warn!("[CoinGecko] Parse error: {}", e),
            },
            Err(e) => warn!("[CoinGecko] Fetch error: {}", e),
        }
    }
}

// ─────────────────────── Main Loop ──────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let node_url = args
        .iter()
        .position(|a| a == "--node")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("http://localhost:8080");

    println!("\x1b[1;36m╔══════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1;36m║      EvaporChain Oracle — Real-World Data Ingestion     ║\x1b[0m");
    println!("\x1b[1;36m╚══════════════════════════════════════════════════════════╝\x1b[0m");
    println!();
    println!("  Node:    {}", node_url);
    println!("  Sources: NASA ISS | USGS Earthquakes | NOAA Solar/Weather");
    println!("           OpenSky Aircraft | Bitcoin Mempool | CoinGecko");
    println!();

    let mut oracle = Oracle::new(node_url);

    // Verify node is reachable
    match oracle
        .client
        .get(format!("{}/api/status", node_url))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(status) = resp.json::<serde_json::Value>().await {
                let height = status
                    .get("block_height")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!("  Chain:   \x1b[32mConnected\x1b[0m (height={})", height);
            }
        }
        Err(e) => {
            eprintln!("  \x1b[31mCannot reach node at {}: {}\x1b[0m", node_url, e);
            eprintln!("  Start the node first, then re-run the oracle.");
            std::process::exit(1);
        }
    }
    println!();
    println!("\x1b[90m──────────────────────────────────────────────────────────\x1b[0m");
    println!();

    let mut tick: u64 = 0;

    loop {
        let cycle_start = tokio::time::Instant::now();

        // ── Every 10s: ISS, Bitcoin mempool ──
        oracle.poll_nasa_iss().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        oracle.poll_bitcoin_mempool().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ── Every 30s: CoinGecko, NOAA solar ──
        if tick.is_multiple_of(3) {
            oracle.poll_coingecko().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            oracle.poll_noaa_solar_wind().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            oracle.poll_noaa_kp_index().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // ── Every 60s: Earthquakes, OpenSky, Weather ──
        if tick.is_multiple_of(6) {
            oracle.poll_usgs_earthquakes().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            oracle.poll_opensky().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            // Major US weather stations
            oracle.poll_noaa_weather("KJFK").await; // JFK Airport, New York
            tokio::time::sleep(Duration::from_millis(500)).await;
            oracle.poll_noaa_weather("KORD").await; // O'Hare, Chicago
            tokio::time::sleep(Duration::from_millis(500)).await;
            oracle.poll_noaa_weather("KLAX").await; // LAX, Los Angeles
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Stats every 5 cycles
        if tick.is_multiple_of(5) && tick > 0 {
            println!(
                "\n\x1b[90m[Oracle Stats] submitted={} accepted={} failed={} uptime={}s\x1b[0m\n",
                oracle.stats.total_submitted,
                oracle.stats.total_accepted,
                oracle.stats.total_failed,
                tick * 10
            );
        }

        tick += 1;

        // Sleep remainder of 10-second cycle
        let elapsed = cycle_start.elapsed();
        if elapsed < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_secs(10) - elapsed).await;
        }
    }
}
