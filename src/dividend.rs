use crate::model::*;
use chrono::{Duration, NaiveDate};
use rayon::prelude::*;

#[derive(Clone, Debug)]
struct Window {
    ticker: String,
    entry: usize,
    exit: usize,
    score: f64,
    events: usize,
}
impl Window {
    fn name(&self) -> String {
        format!(
            "{}: −{} / −{} т.д. ({} событий)",
            self.ticker, self.entry, self.exit, self.events
        )
    }
}
#[derive(Clone)]
struct Event {
    ticker: String,
    buy: NaiveDate,
    sell: NaiveDate,
    amount: f64,
}
struct Open {
    ticker: String,
    quantity: f64,
    entry: f64,
    date: NaiveDate,
    sell: NaiveDate,
}

fn events(data: &Dataset, w: &Window, asof: NaiveDate) -> Vec<Event> {
    let Some(ins) = data.instruments.get(&w.ticker) else {
        return vec![];
    };
    let calendar: Vec<_> = data
        .instruments
        .get("IMOEX")
        .unwrap_or(ins)
        .bars
        .iter()
        .map(|b| b.date)
        .collect();
    data.dividends
        .iter()
        .filter(|e| e.ticker == w.ticker)
        .filter_map(|e| {
            let declared = e.declared_on?;
            let last = e.last_buy_date?;
            if declared > asof || e.announcement_source.is_none() {
                return None;
            }
            // Do not treat a truncated price series as a known future ex-dividend session.
            if last > ins.bars.last()?.date {
                return None;
            }
            let ex = calendar.partition_point(|d| *d <= last);
            // The first observed session after last-buy is ex-dividend; do not require a future bar.
            let sell_index = ex.checked_sub(w.exit)?;
            let intended = ex.checked_sub(w.entry)?;
            // An announcement without an intraday timestamp becomes usable next session.
            let announced = calendar.partition_point(|d| *d <= declared);
            let buy_index = intended.max(announced);
            let sell = *calendar.get(sell_index)?;
            let buy = *calendar.get(buy_index)?;
            if buy >= sell || sell > last || sell > e.record_date || declared >= buy {
                return None;
            }
            Some(Event {
                ticker: w.ticker.clone(),
                buy,
                sell,
                amount: e.amount,
            })
        })
        .collect()
}

fn mark(data: &Dataset, ticker: &str, date: NaiveDate, open: bool) -> Option<f64> {
    let s = data.instruments.get(ticker)?;
    if open {
        let i = s.bars.binary_search_by_key(&date, |b| b.date).ok()?;
        Some(s.bars[i].open)
    } else {
        let i = s.bars.partition_point(|b| b.date <= date).checked_sub(1)?;
        Some(s.bars[i].close)
    }
}
fn cash_return(data: &Dataset, prev: NaiveDate, now: NaiveDate) -> Option<(f64, bool)> {
    if let (Some(a), Some(b)) = (
        mark(data, "LQDT", prev, false),
        mark(data, "LQDT", now, true),
    ) {
        let s = data.instruments.get("LQDT")?;
        if s.bars.first()?.date <= prev {
            return Some((b / a, false));
        }
    }
    let rates = data.instruments.get("CBR_KEY_RATE")?;
    let mut factor = 1.0;
    let mut day = prev;
    while day < now {
        let i = rates
            .bars
            .partition_point(|b| b.date <= day)
            .checked_sub(1)?;
        factor *= (1.0 + rates.bars[i].close / 100.0).powf(1.0 / 365.0);
        day += Duration::days(1);
    }
    Some((factor, true))
}
fn cash_intraday(data: &Dataset, date: NaiveDate) -> (f64, bool) {
    match (
        mark(data, "LQDT", date, true),
        mark(data, "LQDT", date, false),
    ) {
        (Some(a), Some(b)) => (b / a, false),
        _ => (1.0, true),
    }
}

fn optimize_windows(data: &Dataset, s: &Settings, from: NaiveDate, to: NaiveDate) -> Vec<Window> {
    let tickers: Vec<_> = data
        .instruments
        .values()
        .filter(|i| i.kind == "share")
        .map(|i| i.ticker.clone())
        .collect();
    let mut ranked: Vec<_> = tickers
        .par_iter()
        .filter_map(|ticker| {
            let ins = &data.instruments[ticker];
            // Unadjusted share splits make a price-only backtest invalid. Exclude until corrected data is imported.
            if ins.bars.windows(2).any(|b| {
                b[1].date <= to
                    && b[1].date >= from
                    && (b[1].open / b[0].close > 3.0 || b[1].open / b[0].close < 0.33)
            }) {
                return None;
            }
            let mut best: Option<Window> = None;
            for &entry in &s.entry_windows {
                for &exit in &s.exit_windows {
                    if exit >= entry {
                        continue;
                    }
                    let mut w = Window {
                        ticker: ticker.clone(),
                        entry,
                        exit,
                        score: 0.0,
                        events: 0,
                    };
                    let mut ev = events(data, &w, to);
                    ev.sort_by_key(|e| e.buy);
                    let mut value = 1.0;
                    let mut last = None;
                    for e in ev {
                        if e.buy < from || e.sell >= to || last.is_some_and(|d| e.buy <= d) {
                            continue;
                        }
                        if let (Some(a), Some(b)) = (
                            mark(data, ticker, e.buy, true),
                            mark(data, ticker, e.sell, true),
                        ) {
                            value *= b / a;
                            w.events += 1;
                            last = Some(e.sell);
                        }
                    }
                    w.score = (value - 1.0) * 100.0;
                    if w.events >= s.min_events && best.as_ref().is_none_or(|b| w.score > b.score) {
                        best = Some(w);
                    }
                }
            }
            best
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.ticker.cmp(&b.ticker)));
    ranked
}

#[derive(Clone)]
struct Allocation {
    effective: NaiveDate,
    windows: Vec<Window>,
    slots: usize,
}
impl Allocation {
    fn name(&self) -> String {
        format!(
            "Топ-{}, N={}: {}",
            self.windows.len(),
            self.slots,
            self.windows
                .iter()
                .map(Window::name)
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

fn simulate(
    data: &Dataset,
    s: &Settings,
    calendar: &[NaiveDate],
    allocations: &[Allocation],
    trace: bool,
) -> Result<Run, String> {
    let mut r = Run::unavailable("", "shares", "", s.capital);
    r.message = None;
    let mut cash = s.capital;
    let mut positions: Vec<Open> = vec![];
    let mut allocated = 0;
    let mut used_proxy = false;
    let mut planned: Vec<Event> = vec![];
    let start = *calendar.first().ok_or("Нет торговых сессий")?;
    let index_base = mark(data, "IMOEX", start, true);
    let mut benchmark_cash = s.capital;
    for (i, &d) in calendar.iter().enumerate() {
        if i > 0 {
            let (factor, proxy) = cash_return(data, calendar[i - 1], d).ok_or_else(|| {
                format!("Нет LQDT и ключевой ставки для свободного капитала на {d}")
            })?;
            cash *= factor;
            benchmark_cash *= factor;
            used_proxy |= proxy;
        }
        while allocated + 1 < allocations.len() && allocations[allocated + 1].effective <= d {
            allocated += 1;
        }
        let a = &allocations[allocated];
        // Rebuild with only announcements already known today. Existing positions retain their original exit.
        planned.clear();
        for w in &a.windows {
            planned.extend(events(data, w, d));
        }
        for j in (0..positions.len()).rev() {
            if positions[j].sell <= d {
                let p = &positions[j];
                let price = mark(data, &p.ticker, d, true)
                    .ok_or_else(|| format!("Нет цены выхода {} на {d}", p.ticker))?;
                cash += p.quantity * price;
                if trace {
                    r.trades.push(Trade {
                        ticker: p.ticker.clone(),
                        side: "long".into(),
                        entry_date: p.date,
                        exit_date: Some(d),
                        entry_price: p.entry,
                        exit_price: Some(price),
                        quantity: p.quantity,
                        pnl: p.quantity * (price - p.entry),
                        reason: "Выход до дивидендного гэпа".into(),
                    });
                }
                positions.remove(j);
            }
        }
        let mut buys: Vec<_> = planned.iter().filter(|e| e.buy == d).collect();
        // Stable rank order resolves same-day signals; a filled slot is never funded with debt.
        buys.sort_by_key(|e| a.windows.iter().position(|w| w.ticker == e.ticker));
        for e in buys {
            if positions.len() >= a.slots || positions.iter().any(|p| p.ticker == e.ticker) {
                continue;
            }
            let Some(price) = mark(data, &e.ticker, d, true) else {
                continue;
            };
            let mut equity = cash;
            for p in &positions {
                equity += p.quantity
                    * mark(data, &p.ticker, d, true)
                        .or_else(|| {
                            i.checked_sub(1)
                                .and_then(|j| mark(data, &p.ticker, calendar[j], false))
                        })
                        .ok_or("Нет оценки позиции")?;
            }
            let budget = (equity / a.slots as f64).min(cash).max(0.0);
            let lot = data.instruments[&e.ticker].lot_size as f64;
            let qty = (budget / (price * lot)).floor() * lot;
            if qty > 0.0 {
                cash -= qty * price;
                positions.push(Open {
                    ticker: e.ticker.clone(),
                    quantity: qty,
                    entry: price,
                    date: d,
                    sell: e.sell,
                });
            }
            let _ = e.amount;
        }
        let (intraday, proxy) = cash_intraday(data, d);
        cash *= intraday;
        benchmark_cash *= intraday;
        used_proxy |= proxy;
        let mut equity = cash;
        for p in &positions {
            equity += p.quantity
                * mark(data, &p.ticker, d, false).ok_or("Нет ежедневной оценки позиции")?;
        }
        if trace {
            r.points.push(Point {
                mcftr: None,
                date: d,
                equity,
                benchmark: index_base
                    .and_then(|base| mark(data, "IMOEX", d, false).map(|p| s.capital * p / base)),
                lqdt: Some(benchmark_cash),
            });
        }
        r.final_equity = equity;
        r.total_return = (equity / s.capital - 1.0) * 100.0;
    }
    r.parameters = allocations.last().unwrap().name();
    if trace {
        let last = *calendar.last().unwrap();
        let equity = r.final_equity;
        for p in positions {
            let price = mark(data, &p.ticker, last, false).unwrap();
            r.holdings.push(Holding {
                ticker: p.ticker.clone(),
                side: "long".into(),
                weight: p.quantity * price / equity * 100.0,
                reason: "Открыто окно перед объявленной отсечкой".into(),
                until: p.sell.to_string(),
            });
            r.trades.push(Trade {
                ticker: p.ticker,
                side: "long".into(),
                entry_date: p.date,
                exit_date: None,
                entry_price: p.entry,
                exit_price: None,
                quantity: p.quantity,
                pnl: p.quantity * (price - p.entry),
                reason: "Позиция открыта".into(),
            });
        }
        if cash > 0.0 {
            r.holdings.push(Holding {
                ticker: if used_proxy {
                    "LQDT / ключевая ставка"
                } else {
                    "LQDT"
                }
                .into(),
                side: "long".into(),
                weight: cash / equity * 100.0,
                reason: "Капитал незанятых слотов".into(),
                until: "До следующего входа в дивидендную бумагу".into(),
            });
        }
        r.next_action = if r.holdings.iter().all(|h| h.ticker.starts_with("LQDT")) {
            "Свободный капитал — LQDT. Среди выбранных бумаг нет открытой позиции по подтверждённым событиям. Новые входы требуют даты объявления и точного торгового календаря; непроверенные отсечки не прогнозируются.".into()
        } else {
            "Удерживать открытые позиции до указанных дат выхода; свободный капитал — LQDT. Параметры входа и выхода фиксированы на всём периоде с 2023 года.".into()
        };
        if used_proxy {
            r.warnings.push("На части периода LQDT заменён начислением по исторической ключевой ставке ЦБ; это приближение, а не котировки фонда.".into());
        }
        r.finish();
    }
    Ok(r)
}

fn allocation(
    data: &Dataset,
    s: &Settings,
    ranked: &[Window],
    calendar: &[NaiveDate],
    effective: NaiveDate,
) -> Option<(Allocation, Vec<Ranked>)> {
    let mut results = vec![];
    for k in 1..=3.min(ranked.len()) {
        for &slots in &s.slots {
            let a = Allocation {
                effective,
                windows: ranked[..k].to_vec(),
                slots,
            };
            if let Ok(r) = simulate(data, s, calendar, std::slice::from_ref(&a), false) {
                results.push((a, r.total_return));
            }
        }
    }
    results.sort_by(|a, b| b.1.total_cmp(&a.1));
    let leaderboard = results
        .iter()
        .map(|(a, r)| Ranked {
            parameters: a.name(),
            total_return: *r,
            events: a.windows.iter().map(|w| w.events).sum(),
        })
        .collect();
    results.first().map(|(a, _)| (a.clone(), leaderboard))
}

pub fn calculate(data: &Dataset, s: &Settings, progress: impl Fn(String)) -> Report {
    let mut report = Report {
        id: 3,
        calculated_at: chrono::Utc::now().to_rfc3339(),
        settings: s.clone(),
        runs: vec![],
        warnings: data.warnings.clone(),
    };
    let calendar: Vec<_> = data
        .instruments
        .get("IMOEX")
        .map(|s| s.bars.iter().map(|b| b.date).collect())
        .unwrap_or_default();
    let test: Vec<_> = calendar.iter().copied().filter(|d| *d >= s.start).collect();
    let Some(&last) = test.last() else {
        {
            let label = "Исторический оптимум";
            report.runs.push(Run::unavailable(
                label,
                "shares",
                "Нет календаря IMOEX с 2023 года",
                s.capital,
            ));
        }
        return report;
    };
    let verified = data
        .dividends
        .iter()
        .filter(|e| {
            e.declared_on.is_some() && e.last_buy_date.is_some() && e.announcement_source.is_some()
        })
        .count();
    report.warnings.push(format!("Подтверждённых событий: {verified} из {}. События без даты объявления или последнего дня с дивидендом исключены.",data.dividends.len()));
    report.warnings.push("Окна оптимизируются отдельно для каждой бумаги по произведению доходностей завершённых сделок, затем бумаги ранжируются. Перебираются топ-1/2/3 этого рейтинга и N=1…14. Это не полный комбинаторный поиск всех наборов и окон портфеля.".into());
    progress("Индивидуальные окна дивидендных бумаг".into());
    let ranked = optimize_windows(data, s, s.start, last + Duration::days(1));
    if let Some((a, leaders)) = allocation(data, s, &ranked, &test, test[0]) {
        match simulate(data, s, &test, std::slice::from_ref(&a), true) {
            Ok(mut r) => {
                r.label = "Исторический оптимум".into();
                r.tested = leaders.len();
                r.leaderboard = leaders;
                r.warnings
                    .push("Выбор бумаг и окон сделан задним числом на всей истории.".into());
                report.runs.push(r);
            }
            Err(e) => report.runs.push(Run::unavailable(
                "Исторический оптимум",
                "shares",
                e,
                s.capital,
            )),
        }
    } else {
        report.runs.push(Run::unavailable("Исторический оптимум","shares","Недостаточно подтверждённых событий, котировок или истории LQDT/ставки. Минимум событий на бумагу задан в настройках.",s.capital));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Dataset {
        let mut d = Dataset::default();
        let start = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
        for ticker in ["IMOEX", "SBER", "LQDT"] {
            d.instruments.insert(
                ticker.into(),
                Instrument {
                    ticker: ticker.into(),
                    name: ticker.into(),
                    kind: "share".into(),
                    source: "https://example.org".into(),
                    lot_size: 1,
                    multiplier: None,
                    expiry: None,
                    bars: (0..40)
                        .map(|i| Bar {
                            date: start + Duration::days(i),
                            open: 100.0 + i as f64,
                            close: 100.5 + i as f64,
                            volume: 1.0,
                        })
                        .collect(),
                },
            );
        }
        d.dividends.push(Dividend {
            ticker: "SBER".into(),
            record_date: start + Duration::days(32),
            amount: 10.0,
            declared_on: Some(start + Duration::days(20)),
            last_buy_date: Some(start + Duration::days(30)),
            source: "https://example.org/div".into(),
            announcement_source: Some("https://example.org/announcement".into()),
        });
        d
    }
    #[test]
    fn announcement_delays_entry_and_exit_precedes_gap() {
        let d = fixture();
        let w = Window {
            ticker: "SBER".into(),
            entry: 15,
            exit: 1,
            score: 0.0,
            events: 0,
        };
        let e = events(&d, &w, NaiveDate::from_ymd_opt(2023, 2, 8).unwrap());
        assert_eq!(e[0].buy, NaiveDate::from_ymd_opt(2023, 1, 22).unwrap());
        assert_eq!(e[0].sell, NaiveDate::from_ymd_opt(2023, 1, 31).unwrap());
        assert!(events(&d, &w, NaiveDate::from_ymd_opt(2023, 1, 20).unwrap()).is_empty());
    }
    #[test]
    fn slots_never_borrow_and_nav_marks_to_market() {
        let d = fixture();
        let w = Window {
            ticker: "SBER".into(),
            entry: 15,
            exit: 1,
            score: 0.0,
            events: 1,
        };
        let start = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
        let cal: Vec<_> = (0..25).map(|i| start + Duration::days(i)).collect();
        let s = Settings::default();
        let a = Allocation {
            effective: start,
            windows: vec![w],
            slots: 5,
        };
        let r = simulate(&d, &s, &cal, &[a], true).unwrap();
        let stock = r.holdings.iter().find(|h| h.ticker == "SBER").unwrap();
        assert!(stock.weight < 25.0);
        assert!(r.final_equity > s.capital);
        assert!(r.holdings.iter().all(|h| h.weight >= 0.0));
        assert!((r.holdings.iter().map(|h| h.weight).sum::<f64>() - 100.0).abs() < 1e-6);
    }
    #[test]
    fn unknown_announcements_and_truncated_future_are_not_traded() {
        let mut d = fixture();
        let w = Window {
            ticker: "SBER".into(),
            entry: 15,
            exit: 1,
            score: 0.0,
            events: 0,
        };
        let asof = NaiveDate::from_ymd_opt(2023, 2, 8).unwrap();
        d.dividends[0].declared_on = None;
        assert!(events(&d, &w, asof).is_empty());
        d.dividends[0].declared_on = Some(NaiveDate::from_ymd_opt(2023, 1, 21).unwrap());
        d.instruments.get_mut("SBER").unwrap().bars.truncate(25);
        assert!(events(&d, &w, asof).is_empty());
    }
}
