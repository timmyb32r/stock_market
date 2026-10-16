mod benchmarks;
mod data;
mod dividend;
mod engine;
mod ma;
mod model;
mod mozgovik;

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{header, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, Timelike};
use model::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize)]
struct Job {
    status: String,
    progress: String,
    strategy: Option<u8>,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
}
struct Inner {
    dataset: Dataset,
    reports: BTreeMap<u8, Report>,
    job: Option<Job>,
}
#[derive(Clone)]
struct App {
    inner: Arc<Mutex<Inner>>,
    gate: Arc<tokio::sync::Semaphore>,
    dir: PathBuf,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;
fn error(code: StatusCode, e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({"error":e.to_string()})))
}
fn progress(app: &App, text: String) {
    if let Some(j) = app.inner.lock().unwrap().job.as_mut() {
        j.progress = text;
    }
}

#[derive(Serialize)]
struct Summary {
    updated_at: Option<String>,
    instruments: usize,
    bars: usize,
    dividends: usize,
    verified_dividends: usize,
    warnings: Vec<String>,
}
#[derive(Serialize)]
struct Snapshot {
    data: Summary,
    reports: BTreeMap<u8, Report>,
    benchmarks: BTreeMap<String, Run>,
    job: Option<Job>,
    defaults: Settings,
    today: chrono::NaiveDate,
}
async fn state(State(app): State<App>) -> Json<Snapshot> {
    let x = app.inner.lock().unwrap();
    Json(Snapshot {
        data: Summary {
            updated_at: x.dataset.updated_at.clone(),
            instruments: x.dataset.instruments.len(),
            bars: x.dataset.instruments.values().map(|s| s.bars.len()).sum(),
            dividends: x.dataset.dividends.len(),
            verified_dividends: x
                .dataset
                .dividends
                .iter()
                .filter(|e| {
                    e.declared_on.is_some()
                        && e.last_buy_date.is_some()
                        && e.announcement_source.is_some()
                })
                .count(),
            warnings: x.dataset.warnings.clone(),
        },
        reports: x.reports.clone(),
        benchmarks: benchmarks::summary(&x.dataset, &Settings::default()),
        job: x.job.clone(),
        defaults: Settings::default(),
        today: data::today(),
    })
}
async fn sources(State(app): State<App>) -> Json<serde_json::Value> {
    let x = app.inner.lock().unwrap();
    let instruments:Vec<_>=x.dataset.instruments.values().map(|s|serde_json::json!({"ticker":s.ticker,"name":s.name,"kind":s.kind,"source":s.source,"count":s.bars.len(),"from":s.bars.first().map(|b|b.date),"to":s.bars.last().map(|b|b.date),"multiplier":s.multiplier,"expiry":s.expiry})).collect();
    Json(
        serde_json::json!({"instruments":instruments,"dividends":x.dataset.dividends,"warnings":x.dataset.warnings}),
    )
}
async fn export(State(app): State<App>) -> Json<Dataset> {
    Json(app.inner.lock().unwrap().dataset.clone())
}
async fn static_file(uri: Uri) -> axum::response::Response {
    let root = PathBuf::from(std::env::var("MARKET_WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    let path = uri.path().trim_start_matches('/');
    if path.split('/').any(|p| p == ".." || p.starts_with('.')) || path.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = if path.starts_with("assets/") {
        root.join(path)
    } else {
        root.join("index.html")
    };
    let mime = match file.extension().and_then(|s| s.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "text/html; charset=utf-8",
    };
    match tokio::fs::read(file).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "Фронтенд не собран. Выполните cd web && npm install && npm run build",
        )
            .into_response(),
    }
}

fn start_job(
    app: &App,
    strategy: Option<u8>,
) -> Result<tokio::sync::OwnedSemaphorePermit, (StatusCode, Json<serde_json::Value>)> {
    let permit = app.gate.clone().try_acquire_owned().map_err(|_| {
        error(
            StatusCode::CONFLICT,
            "Расчёт или обновление уже выполняется",
        )
    })?;
    app.inner.lock().unwrap().job = Some(Job {
        status: "running".into(),
        progress: "Подготовка".into(),
        strategy,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        error: None,
    });
    Ok(permit)
}
async fn perform(
    app: App,
    only: Option<u8>,
    settings: Option<Settings>,
    refresh: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let outcome: anyhow::Result<()> = async {
        if refresh {
            let old = app.inner.lock().unwrap().dataset.clone();
            let client = data::Moex::new()?;
            let data = client
                .refresh(&old, &app.dir.join("market.json"), |s| progress(&app, s))
                .await?;
            data::save(&app.dir.join("market.json"), &data)?;
            app.inner.lock().unwrap().dataset = data;
        }
        for id in only.map(|id| vec![id]).unwrap_or(vec![1, 2, 3, 4]) {
            let s = settings.clone().unwrap_or_else(|| {
                app.inner
                    .lock()
                    .unwrap()
                    .reports
                    .get(&id)
                    .map(|r| r.settings.clone())
                    .unwrap_or_default()
            });
            let data = app.inner.lock().unwrap().dataset.clone();
            let worker = app.clone();
            progress(&app, format!("Стратегия {id}: начало расчёта"));
            let report = tokio::task::spawn_blocking(move || {
                let mut report = engine::calculate(&data, id, &s, |p| progress(&worker, p));
                benchmarks::enrich(&mut report, &data);
                report
            })
            .await?;
            data::save(&app.dir.join(format!("strategy-{id}.json")), &report)?;
            app.inner.lock().unwrap().reports.insert(id, report);
        }
        Ok(())
    }
    .await;
    let mut x = app.inner.lock().unwrap();
    if let Some(j) = &mut x.job {
        j.finished_at = Some(chrono::Utc::now().to_rfc3339());
        match outcome {
            Ok(()) => {
                j.status = "done".into();
                j.progress = "Расчёты сохранены".into();
            }
            Err(e) => {
                eprintln!("Job failed: {e:#}");
                j.status = "failed".into();
                j.error = Some(format!("{e:#}"));
                j.progress = "Операция не завершена; предыдущие данные сохранены".into();
            }
        }
    }
}
async fn recalculate(
    State(app): State<App>,
    Path(id): Path<u8>,
    Json(settings): Json<Settings>,
) -> ApiResult<serde_json::Value> {
    if !(1..=4).contains(&id) {
        return Err(error(StatusCode::NOT_FOUND, "Нет такой стратегии"));
    }
    settings
        .validate()
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let permit = start_job(&app, Some(id))?;
    let refresh = settings.refresh;
    tokio::spawn(perform(app, Some(id), Some(settings), refresh, permit));
    Ok(Json(serde_json::json!({"status":"running"})))
}
async fn refresh(State(app): State<App>) -> ApiResult<serde_json::Value> {
    let permit = start_job(&app, None)?;
    tokio::spawn(perform(app, None, None, true, permit));
    Ok(Json(serde_json::json!({"status":"running"})))
}
async fn import(
    State(app): State<App>,
    Json(mut incoming): Json<Dataset>,
) -> ApiResult<serde_json::Value> {
    incoming
        .validate(data::today())
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let permit = start_job(&app, None)?;
    // Import replaces matching instruments/events, preserving all other cached data.
    let mut merged = app.inner.lock().unwrap().dataset.clone();
    merged.warnings.retain(|w| {
        !incoming
            .instruments
            .keys()
            .any(|t| w.starts_with(&format!("{t}: ")))
    });
    merged.instruments.extend(incoming.instruments);
    for e in incoming.dividends {
        if let Some(prev) = merged.dividends.iter_mut().find(|p| {
            p.ticker == e.ticker
                && p.record_date == e.record_date
                && (p.amount - e.amount).abs() < 0.001
        }) {
            *prev = e;
        } else {
            merged.dividends.push(e);
        }
    }
    merged.warnings.extend(incoming.warnings);
    merged.updated_at = Some(chrono::Utc::now().to_rfc3339());
    data::save(&app.dir.join("market.json"), &merged)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    app.inner.lock().unwrap().dataset = merged;
    tokio::spawn(perform(app, None, None, false, permit));
    Ok(Json(serde_json::json!({"status":"running"})))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(std::env::var("MARKET_DATA_DIR").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(&dir)?;
    let mut dataset: Dataset = data::read(&dir.join("market.json")).unwrap_or_default();
    dataset.validate(data::today())?;
    let reports = (1..=4)
        .filter_map(|i| {
            data::read(&dir.join(format!("strategy-{i}.json"))).map(|mut r: Report| {
                r.runs.retain(|run| run.label == "Исторический оптимум");
                benchmarks::enrich(&mut r, &dataset);
                (i, r)
            })
        })
        .collect();
    let app = App {
        inner: Arc::new(Mutex::new(Inner {
            dataset,
            reports,
            job: None,
        })),
        gate: Arc::new(tokio::sync::Semaphore::new(1)),
        dir,
    };
    let scheduler = app.clone();
    if std::env::var("MARKET_AUTO_REFRESH").as_deref() != Ok("0") {
        tokio::spawn(async move {
            let mut attempted = None;
            loop {
                let now = chrono::Utc::now() + Duration::hours(3);
                let day = now.date_naive();
                let target = if now.hour() > 0 || now.minute() >= 15 {
                    day
                } else {
                    day - Duration::days(1)
                };
                let fresh = scheduler
                    .inner
                    .lock()
                    .unwrap()
                    .dataset
                    .updated_at
                    .as_ref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| (t + Duration::hours(3)).date_naive());
                let stale_results = {
                    let x = scheduler.inner.lock().unwrap();
                    x.reports.len() < 3
                        || x.reports.values().any(|r| {
                            x.dataset
                                .updated_at
                                .as_ref()
                                .is_some_and(|d| r.calculated_at < *d)
                        })
                };
                let refresh = fresh.is_none_or(|d| d < target);
                if attempted != Some(target) && (refresh || stale_results) {
                    if let Ok(permit) = start_job(&scheduler, None) {
                        attempted = Some(target);
                        perform(scheduler.clone(), None, None, refresh, permit).await;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }
    let api = Router::new()
        .route("/state", get(state))
        .route("/sources", get(sources))
        .route(
            "/strategies/4/source",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                    mozgovik::SOURCE_JSON,
                )
            }),
        )
        .route("/export", get(export))
        .route("/refresh", post(refresh))
        .route("/strategies/{id}/recalculate", post(recalculate))
        .route("/import", post(import));
    let router = Router::new()
        .nest("/api", api)
        .route("/health", get(|| async { "ok" }))
        .fallback(get(static_file))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(app);
    let addr = std::env::var("MARKET_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Market Lab: http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
