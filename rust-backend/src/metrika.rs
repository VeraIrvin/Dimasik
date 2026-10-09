//! Server-only Yandex reporting. No browser-controlled credentials or upstream URLs.
use crate::{config::Config, error::ApiError, util::now_unix, AppState};
use axum::{
    body::Body,
    extract::{RawQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{format_description::well_known::Rfc3339, Date, OffsetDateTime, UtcOffset};
use tokio::sync::Mutex;

const ENDPOINT: &str = "https://api-metrika.yandex.net/stat/v1/data";
const TTL: Duration = Duration::from_secs(600);
const CAPACITY: usize = 256;
#[derive(Debug, Clone, Copy)]
pub struct TrafficError(&'static str);
impl TrafficError {
    fn invalid() -> Self {
        Self("invalid_response")
    }
}
impl IntoResponse for TrafficError {
    fn into_response(self) -> Response {
        let (status, message) = match self.0 {
            "not_configured" => (StatusCode::SERVICE_UNAVAILABLE, "Яндекс Метрика не настроена. Заполните YANDEX_METRIKA_OAUTH_TOKEN и YANDEX_METRIKA_COUNTER_ID в .env.metrika.local и перезапустите backend."),
            "invalid_period" => (StatusCode::BAD_REQUEST, "Укажите корректный период без будущих дат."),
            "invalid_report" => (StatusCode::BAD_REQUEST, "Неизвестный отчёт посещаемости."),
            "access_denied" => (StatusCode::BAD_GATEWAY, "Нет доступа к счётчику Яндекс Метрики. Проверьте OAuth-токен, разрешение metrika:read и права на счётчик."),
            "rate_limited" => (StatusCode::SERVICE_UNAVAILABLE, "Лимит запросов Яндекс Метрики. Попробуйте позже."),
            "invalid_response" => (StatusCode::BAD_GATEWAY, "Яндекс Метрика вернула некорректные данные."),
            _ => (StatusCode::BAD_GATEWAY, "Яндекс Метрика временно недоступна."),
        };
        (status, Json(json!({"error":message,"code":self.0}))).into_response()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    report: String,
    from: Date,
    to: Date,
}
struct Cached {
    fetched: Instant,
    bytes: Bytes,
}
type Slot = Arc<Mutex<Option<Cached>>>;
pub struct TrafficRuntime {
    client: reqwest::Client,
    credentials: Option<(String, String)>,
    endpoint: String,
    ttl: Duration,
    slots: Mutex<HashMap<Key, Slot>>,
}
impl TrafficRuntime {
    pub fn new(config: &Config) -> Result<Self, String> {
        Self::create(
            config.yandex_metrika_oauth_token.clone(),
            config.yandex_metrika_counter_id.clone(),
            ENDPOINT.to_owned(),
            TTL,
        )
    }
    fn create(
        token: Option<String>,
        counter: Option<String>,
        endpoint: String,
        ttl: Duration,
    ) -> Result<Self, String> {
        let credentials = token
            .zip(counter)
            .map(|(token, counter)| (token.trim().to_owned(), counter.trim().to_owned()))
            .filter(|(token, counter)| {
                !token.is_empty() && counter.parse::<u64>().is_ok_and(|id| id > 0)
            });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot initialize reporting HTTP client".to_owned())?;
        Ok(Self {
            client,
            credentials,
            endpoint,
            ttl,
            slots: Mutex::new(HashMap::new()),
        })
    }
    async fn get(&self, key: Key) -> Result<Bytes, TrafficError> {
        let (token, counter) = self
            .credentials
            .as_ref()
            .ok_or(TrafficError("not_configured"))?;
        let slot = {
            let mut map = self.slots.lock().await;
            if let Some(slot) = map.get(&key) {
                slot.clone()
            } else {
                // Never evict in-flight/waited-on slots: this preserves single-flight.
                if map.len() >= CAPACITY {
                    let victim = map
                        .iter()
                        .filter(|(_, slot)| Arc::strong_count(slot) == 1)
                        .filter_map(|(key, slot)| {
                            slot.try_lock()
                                .ok()
                                .map(|entry| (key, entry.as_ref().map(|c| c.fetched)))
                        })
                        .min_by_key(|(_, fetched)| *fetched)
                        .map(|(key, _)| key.clone());
                    if let Some(victim) = victim {
                        map.remove(&victim);
                    } else {
                        return Err(TrafficError("unavailable"));
                    }
                }
                let slot = Arc::new(Mutex::new(None));
                map.insert(key.clone(), slot.clone());
                slot
            }
        };
        let mut cached = slot.lock().await;
        if let Some(entry) = cached
            .as_ref()
            .filter(|entry| entry.fetched.elapsed() < self.ttl)
        {
            return Ok(entry.bytes.clone());
        }
        let (metrics, dimensions, sort, limit) = match key.report.as_str() {
            "overview" => ("ym:s:users,ym:s:visits,ym:s:pageviews", None, None, "20"),
            "daily" => (
                "ym:s:users,ym:s:visits,ym:s:pageviews",
                Some("ym:s:date"),
                Some("ym:s:date"),
                "100000",
            ),
            "pages" => (
                "ym:pv:pageviews",
                Some("ym:pv:URL"),
                Some("-ym:pv:pageviews"),
                "20",
            ),
            "sources" => (
                "ym:s:users,ym:s:visits",
                Some("ym:s:trafficSource"),
                Some("-ym:s:visits"),
                "20",
            ),
            "devices" => (
                "ym:s:users,ym:s:visits",
                Some("ym:s:deviceCategory"),
                Some("-ym:s:visits"),
                "20",
            ),
            _ => return Err(TrafficError("invalid_report")),
        };
        let from = key.from.to_string();
        let to = key.to.to_string();
        let mut query = vec![
            ("ids", counter.as_str()),
            ("date1", from.as_str()),
            ("date2", to.as_str()),
            ("timezone", "+03:00"),
            ("lang", "ru"),
            ("accuracy", "full"),
            ("include_undefined", "true"),
            ("metrics", metrics),
            ("limit", limit),
        ];
        if let Some(dimensions) = dimensions {
            query.push(("dimensions", dimensions));
        }
        if let Some(sort) = sort {
            query.push(("sort", sort));
        }
        let mut authorization = header::HeaderValue::from_str(&format!("OAuth {token}"))
            .map_err(|_| TrafficError("not_configured"))?;
        authorization.set_sensitive(true);
        let response = self
            .client
            .get(&self.endpoint)
            .header(header::AUTHORIZATION, authorization)
            .query(&query)
            .send()
            .await
            .map_err(|_| TrafficError("unavailable"))?;
        match response.status().as_u16() {
            401 | 403 => return Err(TrafficError("access_denied")),
            429 => return Err(TrafficError("rate_limited")),
            200 => {}
            _ => return Err(TrafficError("unavailable")),
        }
        let value: Value = response.json().await.map_err(|_| TrafficError::invalid())?;
        let bytes = parse_response(&key, value)?;
        *cached = Some(Cached {
            fetched: Instant::now(),
            bytes: bytes.clone(),
        });
        Ok(bytes)
    }
}
fn date(value: &str) -> Result<Date, TrafficError> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    if value.len() != 10 {
        return Err(TrafficError("invalid_period"));
    }
    Date::parse(value, format).map_err(|_| TrafficError("invalid_period"))
}
fn resolve(query: Option<&str>, today: Date) -> Result<Key, TrafficError> {
    let values: HashMap<_, _> = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect();
    let report = values.get("report").ok_or(TrafficError("invalid_report"))?;
    if !matches!(
        report.as_str(),
        "overview" | "daily" | "pages" | "sources" | "devices"
    ) {
        return Err(TrafficError("invalid_report"));
    }
    let (from, to) = match values.get("period").map(String::as_str) {
        Some("today") => (today, today),
        Some("7d") => (today - time::Duration::days(6), today),
        Some("30d") => (today - time::Duration::days(29), today),
        Some("custom") => (
            date(values.get("from").ok_or(TrafficError("invalid_period"))?)?,
            date(values.get("to").ok_or(TrafficError("invalid_period"))?)?,
        ),
        _ => return Err(TrafficError("invalid_period")),
    };
    if from > to || to > today {
        return Err(TrafficError("invalid_period"));
    }
    Ok(Key {
        report: report.clone(),
        from,
        to,
    })
}
fn numbers<const N: usize>(value: &Value) -> Result<[f64; N], TrafficError> {
    let values = value
        .as_array()
        .filter(|v| v.len() == N)
        .ok_or_else(TrafficError::invalid)?;
    let mut result = [0.0; N];
    for (number, value) in result.iter_mut().zip(values) {
        *number = value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(TrafficError::invalid)?;
    }
    Ok(result)
}
fn dimension(row: &Value) -> Result<&Value, TrafficError> {
    row.get("dimensions")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 1)
        .and_then(|v| v.first())
        .filter(|v| v.is_object())
        .ok_or_else(TrafficError::invalid)
}
fn parse_response(key: &Key, value: Value) -> Result<Bytes, TrafficError> {
    let sampled = value
        .get("sampled")
        .and_then(Value::as_bool)
        .ok_or_else(TrafficError::invalid)?;
    let share = value
        .get("sample_share")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1.0)
        .ok_or_else(TrafficError::invalid)?;
    let lag = match value.get("data_lag") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(TrafficError::invalid)?,
        ),
    };
    let data = if key.report == "overview" {
        let n = numbers::<3>(value.get("totals").ok_or_else(TrafficError::invalid)?)?;
        json!({"visitors":n[0],"visits":n[1],"pageviews":n[2]})
    } else {
        let upstream = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(TrafficError::invalid)?;
        if key.report == "daily" {
            let total = value
                .get("total_rows")
                .and_then(Value::as_u64)
                .ok_or_else(TrafficError::invalid)?;
            if total != upstream.len() as u64 {
                return Err(TrafficError::invalid());
            }
            let mut days = HashMap::with_capacity(upstream.len());
            for row in upstream {
                let name = dimension(row)?
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(TrafficError::invalid)?;
                let day = date(name).map_err(|_| TrafficError::invalid())?;
                if day < key.from || day > key.to || days.contains_key(&day) {
                    return Err(TrafficError::invalid());
                }
                days.insert(
                    day,
                    numbers::<3>(row.get("metrics").ok_or_else(TrafficError::invalid)?)?,
                );
            }
            let day_count = (key.to - key.from).whole_days() as usize + 1;
            if day_count > 100_000 {
                return Err(TrafficError("invalid_period"));
            }
            let mut rows = Vec::with_capacity(day_count);
            let mut day = key.from;
            loop {
                let n = days.remove(&day).unwrap_or([0.0; 3]);
                rows.push(
                    json!({"date":day.to_string(),"visitors":n[0],"visits":n[1],"pageviews":n[2]}),
                );
                if day == key.to {
                    break;
                }
                day = day.next_day().ok_or_else(TrafficError::invalid)?;
            }
            json!({"rows":rows})
        } else {
            if upstream.len() > 20 {
                return Err(TrafficError::invalid());
            }
            let mut rows = Vec::with_capacity(upstream.len());
            for row in upstream {
                let dim = dimension(row)?;
                let name = match dim.get("name") {
                    Some(Value::String(name)) => name.clone(),
                    Some(Value::Null) => dim
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .unwrap_or("Не определено")
                        .to_owned(),
                    _ => return Err(TrafficError::invalid()),
                };
                let metrics = row.get("metrics").ok_or_else(TrafficError::invalid)?;
                rows.push(if key.report == "pages" {
                    let n = numbers::<1>(metrics)?;
                    json!({"url":name,"pageviews":n[0]})
                } else {
                    let n = numbers::<2>(metrics)?;
                    json!({"name":name,"visitors":n[0],"visits":n[1]})
                });
            }
            json!({"rows":rows})
        }
    };
    let updated = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| TrafficError::invalid())?;
    serde_json::to_vec(&json!({"report":key.report,"period":{"from":key.from.to_string(),"to":key.to.to_string(),"timezone":"+03:00"},"updatedAt":updated,"cacheTtlSeconds":600,"sampled":sampled,"sampleShare":share,"dataLagSeconds":lag,"data":data})).map(Bytes::from).map_err(|_| TrafficError::invalid())
}
pub async fn traffic(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    // GET authorization deliberately checks session only, before configuration/cache.
    if !state.auth.has_admin_session(&headers, now_unix()) {
        return ApiError::unauthorized().into_response();
    }
    let today = OffsetDateTime::now_utc()
        .to_offset(UtcOffset::from_hms(3, 0, 0).expect("fixed offset"))
        .date();
    let key = match resolve(query.as_deref(), today) {
        Ok(key) => key,
        Err(error) => return error.into_response(),
    };
    match state.metrika.get(key).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, "application/json")],
            Body::from(bytes),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, routing::get, Router};
    use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

    fn key(report: &str, from: &str, to: &str) -> Key {
        Key {
            report: report.into(),
            from: date(from).unwrap(),
            to: date(to).unwrap(),
        }
    }
    fn fixture(totals: Value, data: Value) -> Value {
        json!({"sampled":false,"sample_share":1.0,"data_lag":90,"total_rows":data.as_array().unwrap().len(),"totals":totals,"data":data})
    }
    fn decode(bytes: Bytes) -> Value {
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn period_users_are_not_daily_users_and_missing_days_are_zero() {
        let overview = decode(
            parse_response(
                &key("overview", "2020-01-01", "2020-01-03"),
                fixture(json!([4, 10, 21]), json!([])),
            )
            .unwrap(),
        );
        let daily = decode(
            parse_response(
                &key("daily", "2020-01-01", "2020-01-03"),
                fixture(
                    json!([4, 10, 21]),
                    json!([
                        {"dimensions":[{"name":"2020-01-03"}],"metrics":[3,6,12]},
                        {"dimensions":[{"name":"2020-01-01"}],"metrics":[3,4,9]}
                    ]),
                ),
            )
            .unwrap(),
        );
        assert_eq!(overview["data"]["visitors"], 4.0);
        assert_eq!(
            daily["data"]["rows"],
            json!([
                {"date":"2020-01-01","visitors":3.0,"visits":4.0,"pageviews":9.0},
                {"date":"2020-01-02","visitors":0.0,"visits":0.0,"pageviews":0.0},
                {"date":"2020-01-03","visitors":3.0,"visits":6.0,"pageviews":12.0}
            ])
        );
    }

    #[test]
    fn malformed_and_truncated_reports_do_not_turn_into_zeroes() {
        for totals in [
            json!(null),
            json!([1, 2]),
            json!([-1, 2, 3]),
            json!(["4", 2, 3]),
        ] {
            assert_eq!(
                parse_response(
                    &key("overview", "2020-01-01", "2020-01-01"),
                    fixture(totals, json!([]))
                )
                .unwrap_err()
                .0,
                "invalid_response"
            );
        }
        let mut truncated = fixture(json!([0, 0, 0]), json!([]));
        truncated["total_rows"] = json!(1);
        assert_eq!(
            parse_response(&key("daily", "2020-01-01", "2020-01-01"), truncated)
                .unwrap_err()
                .0,
            "invalid_response"
        );
        let duplicate = fixture(
            json!([2, 2, 2]),
            json!([
                {"dimensions":[{"name":"2020-01-01"}],"metrics":[1,1,1]},
                {"dimensions":[{"name":"2020-01-01"}],"metrics":[1,1,1]}
            ]),
        );
        assert_eq!(
            parse_response(&key("daily", "2020-01-01", "2020-01-01"), duplicate)
                .unwrap_err()
                .0,
            "invalid_response"
        );
    }

    #[test]
    fn inclusive_periods_and_calendar_validation() {
        let today = date("2024-03-01").unwrap();
        let week = resolve(Some("report=overview&period=7d"), today).unwrap();
        assert_eq!(week.from, date("2024-02-24").unwrap());
        assert_eq!(week.to, today);
        assert!(resolve(
            Some("report=daily&period=custom&from=2024-02-29&to=2024-03-01"),
            today
        )
        .is_ok());
        for query in [
            "report=overview&period=custom&from=2023-02-29&to=2024-03-01",
            "report=overview&period=custom&from=2024-03-02&to=2024-03-02",
            "report=overview&period=custom&from=2024-03-01&to=2024-02-28",
            "report=overview&period=custom&from=2024-02-29",
        ] {
            assert_eq!(resolve(Some(query), today).unwrap_err().0, "invalid_period");
        }
        assert_eq!(
            resolve(Some("report=credentials&period=today"), today)
                .unwrap_err()
                .0,
            "invalid_report"
        );
    }

    struct Upstream {
        calls: AtomicUsize,
        status: AtomicU16,
    }
    async fn serve_fixture() -> (String, Arc<Upstream>, tokio::task::JoinHandle<()>) {
        let state = Arc::new(Upstream {
            calls: AtomicUsize::new(0),
            status: AtomicU16::new(200),
        });
        let shared = state.clone();
        let router = Router::new().route("/data", get(move |Query(query): Query<HashMap<String,String>>| {
            let shared = shared.clone();
            async move {
                let count = shared.calls.fetch_add(1,Ordering::SeqCst) + 1;
                let status = StatusCode::from_u16(shared.status.load(Ordering::SeqCst)).unwrap();
                if status != StatusCode::OK {
                    return (status,Json(json!({"error":"private-upstream-body-unit-token"})));
                }
                let data = match query.get("dimensions").map(String::as_str) {
                    Some("ym:s:date") => json!([{"dimensions":[{"name":query["date1"]}],"metrics":[3,4,9]}]),
                    Some("ym:pv:URL") => json!([{"dimensions":[{"name":"https://example.org/history"}],"metrics":[9]}]),
                    Some(_) => json!([{"dimensions":[{"name":"Смартфоны"}],"metrics":[3,4]}]),
                    None => json!([]),
                };
                (status,Json(fixture(json!([count * 4,10,21]),data)))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/data", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (endpoint, state, task)
    }
    fn runtime(endpoint: String) -> TrafficRuntime {
        TrafficRuntime::create(
            Some("unit-token".into()),
            Some("113584085".into()),
            endpoint,
            TTL,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn cache_coalesces_requests_and_separates_periods_and_reports() {
        let (endpoint, upstream, task) = serve_fixture().await;
        let runtime = runtime(endpoint);
        let first = key("overview", "2020-01-01", "2020-01-03");
        let (a, b) = tokio::join!(runtime.get(first.clone()), runtime.get(first.clone()));
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a, b);
        assert_eq!(decode(a.clone())["data"]["visitors"], 4.0);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        let daily = decode(
            runtime
                .get(key("daily", "2020-01-01", "2020-01-03"))
                .await
                .unwrap(),
        );
        assert_eq!(daily["data"]["rows"][0]["visitors"], 3.0);
        let other = decode(
            runtime
                .get(key("overview", "2020-01-02", "2020-01-03"))
                .await
                .unwrap(),
        );
        assert_eq!(other["data"]["visitors"], 12.0);
        assert_eq!(runtime.get(first.clone()).await.unwrap(), a);
        let slot = runtime.slots.lock().await.get(&first).unwrap().clone();
        slot.lock().await.as_mut().unwrap().fetched = Instant::now() - TTL;
        let refreshed = decode(runtime.get(first).await.unwrap());
        assert_eq!(refreshed["data"]["visitors"], 16.0);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 4);
        task.abort();
    }

    #[tokio::test]
    async fn upstream_access_failure_is_private_and_not_cached_as_zero() {
        let (endpoint, upstream, task) = serve_fixture().await;
        let runtime = runtime(endpoint);
        upstream.status.store(403, Ordering::SeqCst);
        let error = runtime
            .get(key("overview", "2020-01-01", "2020-01-03"))
            .await
            .unwrap_err();
        assert_eq!(error.0, "access_denied");
        let response = error.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("unit-token"));
        upstream.status.store(200, Ordering::SeqCst);
        assert_eq!(
            decode(
                runtime
                    .get(key("overview", "2020-01-01", "2020-01-03"))
                    .await
                    .unwrap()
            )["data"]["visitors"],
            8.0
        );
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn administrator_authorization_precedes_cache_reads() {
        use axum::http::{Method, Request};
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let args = crate::config::parse_args(&["--fresh".into()]).unwrap();
        let mut config = Config::resolve(&HashMap::new(), &args);
        config.database_path = dir.path().join("test.sqlite");
        config.import_dir = dir.path().join("empty");
        config.public_data_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../public/data");
        config.admin_username = Some("traffic-admin".into());
        config.admin_password = Some("traffic-password".into());
        config.admin_session_secret = Some("traffic-test-session-secret-with-32-characters".into());
        config.frontend_origin = Some("http://localhost:3000".into());
        let mut state = crate::prepare(&config).unwrap();
        let (endpoint, upstream, task) = serve_fixture().await;
        state.metrika = Arc::new(runtime(endpoint));
        state
            .metrika
            .get(key("overview", "2020-01-01", "2020-01-03"))
            .await
            .unwrap();
        let app = crate::build_router(state);
        let uri =
            "/api/metrika/traffic?report=overview&period=custom&from=2020-01-01&to=2020-01-03";
        for cookie in [None, Some("dimasik_admin_session=forged")] {
            let mut req = Request::builder().uri(uri);
            if let Some(cookie) = cookie {
                req = req.header(header::COOKIE, cookie);
            }
            let response = app
                .clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/session")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "http://localhost:3000")
                    .body(Body::from(
                        r#"{"login":"traffic-admin","password":"traffic-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["data"]["visitors"],
            4.0
        );
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
