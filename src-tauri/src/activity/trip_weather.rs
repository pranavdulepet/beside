// Resolve a public airport locally. Only its rounded coordinates go to the
// forecast provider. Select the date and compare it with packing evidence here.
use chrono::{Duration as Days, NaiveDate, Utc};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::Read,
    sync::Mutex,
    time::{Duration, Instant},
};
use url::Url;

const AIRPORTS: &str = include_str!("../../data/airports.tsv");
const DAILY: &str = "temperature_2m_min,temperature_2m_max,precipitation_probability_max";
const CACHE_LIFE: Duration = Duration::from_secs(3600);
static CACHE: Mutex<VecDeque<(String, Instant, Value)>> = Mutex::new(VecDeque::new());

fn endpoint(airport: &str) -> Option<Url> {
    if airport.len() != 3 || !airport.bytes().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let mut row = AIRPORTS
        .lines()
        .find(|line| line.split('\t').next() == Some(airport))?
        .split('\t');
    row.next()?;
    let (latitude, longitude) = (row.next()?, row.next()?);
    let mut url = Url::parse("https://api.open-meteo.com/v1/forecast").ok()?;
    url.query_pairs_mut().extend_pairs([
        ("latitude", latitude),
        ("longitude", longitude),
        ("daily", DAILY),
        ("timezone", "auto"),
        ("forecast_days", "16"),
    ]);
    Some(url)
}

fn load(airport: &str, url: &Url) -> Option<Value> {
    if let Some(data) = CACHE
        .lock()
        .ok()?
        .iter()
        .find(|(k, t, _)| k == airport && t.elapsed() < CACHE_LIFE)
    {
        return Some(data.2.clone());
    }
    let client = ureq::AgentBuilder::new().redirects(0).build();
    let mut source = url.clone();
    let response = match client
        .get(source.as_str())
        .timeout(Duration::from_millis(1500))
        .call()
    {
        Ok(response) => response,
        // The official Previous Runs API also serves the current forecast
        // when no previous-day variable is requested. Retry only a failed
        // connection; respect all HTTP rejections, including rate limits.
        Err(ureq::Error::Transport(_)) => {
            source
                .set_host(Some("previous-runs-api.open-meteo.com"))
                .ok()?;
            client
                .get(source.as_str())
                .timeout(Duration::from_secs(3))
                .call()
                .ok()?
        }
        Err(ureq::Error::Status(_, _)) => return None,
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(64_001)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 64_000 {
        return None;
    }
    let mut data: Value = serde_json::from_slice(&bytes).ok()?;
    if data["daily"]["time"].as_array()?.is_empty() {
        return None;
    }
    data["fetchedAt"] = json!(Utc::now().to_rfc3339());
    data["sourceUrl"] = json!(source.as_str());
    let mut cache = CACHE.lock().ok()?;
    cache.retain(|(code, at, _)| code != airport && at.elapsed() < CACHE_LIFE);
    if cache.len() >= 12 {
        cache.pop_front();
    }
    cache.push_back((airport.to_owned(), Instant::now(), data.clone()));
    Some(data)
}

fn day_values(data: &Value, day: NaiveDate) -> Option<(f64, f64, f64)> {
    let units = &data["daily_units"];
    if units["temperature_2m_min"] != "°C"
        || units["temperature_2m_max"] != "°C"
        || units["precipitation_probability_max"] != "%"
    {
        return None;
    }
    let daily = &data["daily"];
    let date = day.to_string();
    let mut indices = daily["time"]
        .as_array()?
        .iter()
        .enumerate()
        .filter(|(_, d)| d.as_str() == Some(&date));
    let (i, _) = indices.next()?;
    if indices.next().is_some() {
        return None;
    }
    let low = daily["temperature_2m_min"].get(i)?.as_f64()?;
    let high = daily["temperature_2m_max"].get(i)?.as_f64()?;
    let rain = daily["precipitation_probability_max"].get(i)?.as_f64()?;
    ((-100.0..=65.0).contains(&low)
        && (low..=65.0).contains(&high)
        && (0.0..=100.0).contains(&rain))
    .then_some((low, high, rain))
}

pub(super) fn attach(reply: &mut Value, trip: &Value) {
    let Some(code) = trip["airport"].as_str() else {
        return;
    };
    let Some(day) = trip["date"]
        .as_str()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    else {
        return;
    };
    let Some(url) = endpoint(code) else { return };
    let today = Utc::now().date_naive();
    let in_range = day >= today - Days::days(1) && day < today + Days::days(16);
    let forecast = if in_range {
        load(code, &url).and_then(|v| {
            Some((
                day_values(&v, day)?,
                v["fetchedAt"].as_str()?.to_owned(),
                v["sourceUrl"].as_str()?.to_owned(),
            ))
        })
    } else {
        None
    };
    let label = day.format("%b %-d");
    let explanation = match forecast {
        Some(((low, high, rain), fetched, source_url)) => {
            let packing = if rain >= 50.0 && trip["rainLayer"] == true {
                " Your list already includes a rain layer."
            } else if low < 10.0 && trip["warmLayer"] == true {
                " Your list already includes a warm layer."
            } else if rain >= 50.0 || low < 10.0 {
                " Weigh the bag again if you add a layer."
            } else { "" };
            let summary = format!("Open-Meteo near {code} on {label} forecasts {low:.0}–{high:.0} °C with a {rain:.0}% peak chance of precipitation on your outbound date.");
            reply["webSources"] = json!([{
                "title": format!("Open-Meteo · {code} · {label}"),
                "url": source_url,
                "quote": summary,
                "retrievedAt": fetched,
                "basis": "Beside used the arrival airport and outbound date from your booking."
            }]);
            format!("{summary}{packing}")
        }
        None if in_range => format!("I couldn’t verify a forecast near {code} for {label}. No forecast was used."),
        None => format!("The {label} flight is outside the live forecast window. Weather wasn’t used for the bag estimate."),
    };
    if let Some(answer) = reply["answer"].as_str() {
        reply["answer"] = json!(format!("{answer}\n\n{explanation}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weather_uses_only_public_coordinates_and_the_exact_dated_row() {
        let url = endpoint("ATH").unwrap();
        assert!(url.as_str().contains("latitude=37.94"));
        assert!(!url.as_str().contains("ATH"));
        assert!(endpoint("my packing list").is_none());
        let day = NaiveDate::from_ymd_opt(2026, 12, 2).unwrap();
        let mut source = json!({"daily_units":{"temperature_2m_min":"°C","temperature_2m_max":"°C","precipitation_probability_max":"%"},
            "daily":{"time":["2026-12-01","2026-12-02"],"temperature_2m_min":[1,3],"temperature_2m_max":[2,9],"precipitation_probability_max":[80,7]}});
        assert_eq!(day_values(&source, day), Some((3., 9., 7.)));
        assert_eq!(day_values(&source, day + Days::days(1)), None);
        source["daily"]["temperature_2m_max"][1] = Value::Null;
        assert_eq!(day_values(&source, day), None);
    }
}
