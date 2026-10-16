use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bar {
    pub date: NaiveDate,
    pub open: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instrument {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    pub source: String,
    #[serde(default = "one")]
    pub lot_size: u32,
    #[serde(default)]
    pub multiplier: Option<f64>,
    #[serde(default)]
    pub expiry: Option<NaiveDate>,
    pub bars: Vec<Bar>,
}
fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dividend {
    pub ticker: String,
    pub record_date: NaiveDate,
    pub amount: f64,
    // A public source must establish BOTH amount and record date by this day.
    pub declared_on: Option<NaiveDate>,
    pub last_buy_date: Option<NaiveDate>,
    pub source: String,
    #[serde(default)]
    pub announcement_source: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Dataset {
    pub instruments: BTreeMap<String, Instrument>,
    pub dividends: Vec<Dividend>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Dataset {
    pub fn validate(&mut self, today: NaiveDate) -> anyhow::Result<()> {
        for (key, s) in &mut self.instruments {
            anyhow::ensure!(key == &s.ticker && !key.is_empty(), "Некорректный тикер");
            anyhow::ensure!(
                s.source.starts_with("https://"),
                "Нужна ссылка на публичный источник {key}"
            );
            anyhow::ensure!(s.lot_size > 0, "Размер лота должен быть положительным");
            if s.kind == "future" {
                anyhow::ensure!(
                    s.multiplier.is_some_and(|m| m.is_finite() && m > 0.0) && s.expiry.is_some(),
                    "Нет спецификации фьючерса {key}"
                );
            }
            s.bars.retain(|b| b.date < today);
            s.bars.sort_by_key(|b| b.date);
            anyhow::ensure!(
                s.bars.windows(2).all(|w| w[0].date != w[1].date),
                "Повтор даты {key}"
            );
            anyhow::ensure!(
                s.bars.iter().all(|b| b.open.is_finite()
                    && b.open > 0.0
                    && b.close.is_finite()
                    && b.close > 0.0
                    && b.volume.is_finite()
                    && b.volume >= 0.0),
                "Некорректные цены {key}"
            );
        }
        for e in &self.dividends {
            anyhow::ensure!(
                e.amount.is_finite() && e.amount > 0.0 && e.source.starts_with("https://"),
                "Некорректный дивиденд"
            );
            anyhow::ensure!(
                e.declared_on.is_none_or(|d| d < e.record_date),
                "Объявление после реестра"
            );
            anyhow::ensure!(
                e.last_buy_date.is_none_or(|d| d <= e.record_date),
                "Покупка после реестра"
            );
            if e.declared_on.is_some() {
                anyhow::ensure!(
                    e.announcement_source
                        .as_ref()
                        .is_some_and(|s| s.starts_with("https://")),
                    "Нет источника даты объявления"
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub capital: f64,
    pub refresh: bool,
    pub periods: Vec<usize>,
    pub entry_windows: Vec<usize>,
    pub exit_windows: Vec<usize>,
    pub slots: Vec<usize>,
    pub training_start: NaiveDate,
    pub start: NaiveDate,
    pub min_events: usize,
    pub fund: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            capital: 1_000_000.0,
            refresh: false,
            periods: vec![
                5, 8, 10, 13, 15, 20, 25, 30, 34, 40, 50, 60, 75, 89, 100, 120, 150, 180, 200, 233,
                250, 275, 300,
            ],
            entry_windows: vec![5, 10, 15, 20, 30, 40, 60],
            exit_windows: vec![1, 2, 3, 5, 10],
            slots: (1..=14).collect(),
            training_start: NaiveDate::from_ymd_opt(2015, 1, 1).unwrap(),
            start: NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            min_events: 2,
            fund: "TMOS".into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.capital.is_finite() && (10_000.0..=1e12).contains(&self.capital),
            "Капитал: от 10 тысяч до 1 трлн ₽"
        );
        anyhow::ensure!(
            !self.periods.is_empty()
                && self.periods.len() <= 499
                && self.periods.iter().all(|p| (2..=500).contains(p)),
            "Нужно 1–499 периодов от 2 до 500"
        );
        anyhow::ensure!(
            !self.entry_windows.is_empty()
                && self.entry_windows.len() <= 119
                && self.entry_windows.iter().all(|p| (2..=120).contains(p)),
            "Окна входа: 2–120 дней, до 119 вариантов"
        );
        anyhow::ensure!(
            !self.exit_windows.is_empty()
                && self.exit_windows.len() <= 60
                && self.exit_windows.iter().all(|p| (1..=60).contains(p)),
            "Окна выхода: 1–60 дней, до 60 вариантов"
        );
        anyhow::ensure!(
            !self.slots.is_empty()
                && self.slots.len() <= 20
                && self.slots.iter().all(|p| (1..=20).contains(p)),
            "Слоты: 1–20"
        );
        anyhow::ensure!(
            self.min_events > 0 && self.training_start < self.start,
            "Нужна история до начала расчёта и минимум одно событие"
        );
        anyhow::ensure!(
            self.fund
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && self.fund.len() <= 12
                && !self.fund.is_empty(),
            "Неверный тикер фонда"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub date: NaiveDate,
    pub equity: f64,
    pub benchmark: Option<f64>,
    pub lqdt: Option<f64>,
    #[serde(default)]
    pub mcftr: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trade {
    pub ticker: String,
    pub side: String,
    pub entry_date: NaiveDate,
    pub exit_date: Option<NaiveDate>,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub quantity: f64,
    pub pnl: f64,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Holding {
    pub ticker: String,
    pub side: String,
    pub weight: f64,
    pub reason: String,
    pub until: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub effective: NaiveDate,
    pub trained_until: NaiveDate,
    pub parameters: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub label: String,
    pub mode: String,
    pub status: String,
    pub message: Option<String>,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub initial: f64,
    pub final_equity: f64,
    pub total_return: f64,
    pub cagr: f64,
    pub max_drawdown: f64,
    pub parameters: String,
    pub tested: usize,
    pub points: Vec<Point>,
    pub trades: Vec<Trade>,
    pub holdings: Vec<Holding>,
    pub selections: Vec<Selection>,
    pub warnings: Vec<String>,
    pub leaderboard: Vec<Ranked>,
    #[serde(default)]
    pub next_action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ranked {
    pub parameters: String,
    pub total_return: f64,
    pub events: usize,
}
impl Run {
    pub fn unavailable(label: &str, mode: &str, message: impl Into<String>, capital: f64) -> Self {
        Self {
            label: label.into(),
            mode: mode.into(),
            status: "unavailable".into(),
            message: Some(message.into()),
            start: None,
            end: None,
            initial: capital,
            final_equity: capital,
            total_return: 0.0,
            cagr: 0.0,
            max_drawdown: 0.0,
            parameters: String::new(),
            tested: 0,
            points: vec![],
            trades: vec![],
            holdings: vec![],
            selections: vec![],
            warnings: vec![],
            leaderboard: vec![],
            next_action: String::new(),
        }
    }
    pub fn finish(&mut self) {
        if let (Some(a), Some(b)) = (self.points.first(), self.points.last()) {
            self.start = Some(a.date);
            self.end = Some(b.date);
            self.final_equity = b.equity;
            self.total_return = (self.final_equity / self.initial - 1.0) * 100.0;
            let years = (b.date - a.date).num_days() as f64 / 365.25;
            self.cagr = if years > 0.0 {
                ((self.final_equity.max(0.0) / self.initial).powf(1.0 / years) - 1.0) * 100.0
            } else {
                0.0
            };
            let mut high = self.initial;
            for p in &self.points {
                high = high.max(p.equity);
                self.max_drawdown = self.max_drawdown.max((1.0 - p.equity / high) * 100.0);
            }
            self.status = if self.final_equity <= 0.0 {
                "bankrupt"
            } else {
                "ready"
            }
            .into();
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub id: u8,
    pub calculated_at: String,
    pub settings: Settings,
    pub runs: Vec<Run>,
    pub warnings: Vec<String>,
}
