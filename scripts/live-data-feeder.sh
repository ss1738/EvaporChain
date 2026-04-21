#!/bin/bash
# EvaporChain Live Data Feeder — pumps real external data as objects
# Objects get different half-lives based on data type:
#   - Sensor readings: short half-life (2000 blocks) — stale fast
#   - Weather data: medium half-life (5000 blocks)
#   - Air quality: medium half-life (8000 blocks)
#   - Market data: short half-life (1500 blocks) — yesterday's price is irrelevant
#
# Usage: ./live-data-feeder.sh [API_URL]
# Default API: http://127.0.0.1:8080

API="${1:-http://127.0.0.1:8080}"
CREATOR="0xfeed000000000000000000000000000000000000000000000000000000000001"
SEQ=$(date +%s)

log() { echo "[$(date '+%H:%M:%S')] $*"; }

create_object() {
    local name="$1" energy="$2" half_life="$3" data="$4"
    SEQ=$((SEQ + 1))
    local oid
    oid=$(printf '0xfeed%056x' "$SEQ")

    local payload
    payload=$(cat <<ENDJSON
{
  "creator": "$CREATOR",
  "object_id": "$oid",
  "energy": $energy,
  "half_life": $half_life,
  "data": "$data",
  "signature": "0x00"
}
ENDJSON
)
    local resp
    resp=$(curl -s -X POST "$API/api/tx/create-object" \
        -H "Content-Type: application/json" \
        -d "$payload" 2>&1)

    local ok
    ok=$(echo "$resp" | grep -o '"success":true')
    if [ -n "$ok" ]; then
        log "CREATED $name | energy=$energy half_life=$half_life | $oid"
    else
        log "FAILED  $name | $resp"
    fi
}

# ── Weather Data (OpenMeteo — free, no API key) ──
feed_weather() {
    log "Fetching weather data (London, Delhi, New York)..."
    local cities=("51.5074,-0.1278:London" "28.6139,77.2090:Delhi" "40.7128,-74.0060:NewYork")
    for entry in "${cities[@]}"; do
        local coords="${entry%%:*}"
        local city="${entry##*:}"
        local lat="${coords%%,*}"
        local lon="${coords##*,}"

        local weather
        weather=$(curl -s --max-time 5 "https://api.open-meteo.com/v1/forecast?latitude=$lat&longitude=$lon&current=temperature_2m,relative_humidity_2m,wind_speed_10m,pressure_msl" 2>&1)

        local temp humidity wind pressure
        temp=$(echo "$weather" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['temperature_2m'])" 2>/dev/null)
        humidity=$(echo "$weather" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['relative_humidity_2m'])" 2>/dev/null)
        wind=$(echo "$weather" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['wind_speed_10m'])" 2>/dev/null)
        pressure=$(echo "$weather" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['pressure_msl'])" 2>/dev/null)

        if [ -n "$temp" ]; then
            local data_str="weather:${city}:temp=${temp}C,humidity=${humidity}%,wind=${wind}kmh,pressure=${pressure}hPa"
            create_object "weather:$city" 3000 5000 "$data_str"
        else
            log "SKIP weather:$city (no data)"
        fi
    done
}

# ── Air Quality (OpenMeteo Air Quality — free) ──
feed_air_quality() {
    log "Fetching air quality data..."
    local cities=("28.6139,77.2090:Delhi" "51.5074,-0.1278:London" "39.9042,116.4074:Beijing")
    for entry in "${cities[@]}"; do
        local coords="${entry%%:*}"
        local city="${entry##*:}"
        local lat="${coords%%,*}"
        local lon="${coords##*,}"

        local aq
        aq=$(curl -s --max-time 5 "https://air-quality-api.open-meteo.com/v1/air-quality?latitude=$lat&longitude=$lon&current=pm2_5,pm10,nitrogen_dioxide,ozone,carbon_monoxide" 2>&1)

        local pm25 pm10 no2 o3 co
        pm25=$(echo "$aq" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['pm2_5'])" 2>/dev/null)
        pm10=$(echo "$aq" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['pm10'])" 2>/dev/null)
        no2=$(echo "$aq" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['nitrogen_dioxide'])" 2>/dev/null)
        o3=$(echo "$aq" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['ozone'])" 2>/dev/null)
        co=$(echo "$aq" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['current']['carbon_monoxide'])" 2>/dev/null)

        if [ -n "$pm25" ]; then
            local data_str="airquality:${city}:pm2.5=${pm25},pm10=${pm10},no2=${no2},o3=${o3},co=${co}"
            create_object "aq:$city" 5000 8000 "$data_str"
        else
            log "SKIP aq:$city (no data)"
        fi
    done
}

# ── Crypto Market Data (CoinGecko — free, no key) ──
feed_market() {
    log "Fetching market data..."
    local market
    market=$(curl -s "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,ethereum,solana&vs_currencies=usd&include_24hr_change=true" 2>&1)

    for coin in bitcoin ethereum solana; do
        local price change
        price=$(echo "$market" | grep -o "\"$coin\":{[^}]*}" | grep -o '"usd":[0-9.]*' | cut -d: -f2)
        change=$(echo "$market" | grep -o "\"$coin\":{[^}]*}" | grep -o '"usd_24h_change":[0-9.-]*' | cut -d: -f2)

        if [ -n "$price" ]; then
            local data_str="market:${coin}:price=\$${price},24h_change=${change}%"
            create_object "market:$coin" 2000 1500 "$data_str"
        else
            log "SKIP market:$coin (no data)"
        fi
    done
}

# ── IoT Sensor Simulation (using real earthquake data — USGS) ──
feed_seismic() {
    log "Fetching seismic data (USGS)..."
    local quakes
    quakes=$(curl -s "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_hour.geojson" 2>&1)

    local count
    count=$(echo "$quakes" | grep -o '"count":[0-9]*' | head -1 | cut -d: -f2)

    if [ -z "$count" ] || [ "$count" = "0" ]; then
        log "No recent earthquakes (M2.5+)"
        return
    fi

    # Extract up to 5 recent quakes
    local i=0
    echo "$quakes" | grep -o '"title":"[^"]*"' | head -5 | while read -r line; do
        local title="${line#*:\"}"
        title="${title%\"}"
        local mag="${title%% -*}"
        create_object "seismic:event-$i" 1500 2000 "seismic:${title}"
        i=$((i + 1))
    done
}

# ── ISS Position (real-time satellite tracking) ──
feed_iss() {
    log "Fetching ISS position..."
    local iss
    iss=$(curl -s "http://api.open-notify.org/iss-now.json" 2>&1)

    local lat lon
    lat=$(echo "$iss" | grep -o '"latitude":"[^"]*"' | cut -d'"' -f4)
    lon=$(echo "$iss" | grep -o '"longitude":"[^"]*"' | cut -d'"' -f4)

    if [ -n "$lat" ]; then
        create_object "iss:position" 1000 1000 "iss:lat=${lat},lon=${lon},time=$(date -u +%H:%M:%S)"
    fi
}

# ── Main Loop ──
log "EvaporChain Live Data Feeder starting"
log "API: $API"
log "---"

# Check API is up
status=$(curl -s "$API/api/status" 2>&1)
height=$(echo "$status" | grep -o '"block_height":[0-9]*' | cut -d: -f2)
if [ -z "$height" ]; then
    log "ERROR: Cannot reach API at $API"
    exit 1
fi
log "Chain at block #$height — connected"
log "=== Starting feed cycle ==="

while true; do
    feed_weather
    feed_air_quality
    feed_market
    feed_seismic
    feed_iss

    # Show current chain state
    status=$(curl -s "$API/api/status" 2>&1)
    height=$(echo "$status" | grep -o '"block_height":[0-9]*' | cut -d: -f2)
    objects=$(echo "$status" | grep -o '"active_objects":[0-9]*' | cut -d: -f2)
    ghosts=$(echo "$status" | grep -o '"ghost_count":[0-9]*' | cut -d: -f2)
    evaporated=$(echo "$status" | grep -o '"total_evaporated":[0-9]*' | cut -d: -f2)

    log "--- Block #$height | Active: $objects | Ghosts: $ghosts | Evaporated: $evaporated ---"
    log "Next feed in 60 seconds..."
    sleep 60
done
