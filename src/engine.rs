use crate::{
    ma::{average, Kind, KINDS},
    model::*,
};
use chrono::{Datelike, Duration, NaiveDate};
use rayon::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Pair {
    fast: Kind,
    slow: Kind,
    a: usize,
    b: usize,
    signal: Vec<i8>,
}
impl Pair {
    fn name(&self) -> String {
        format!("{} {} / {} {}", self.fast, self.a, self.slow, self.b)
    }
}
fn pairs(index: &Instrument, settings: &Settings) -> Vec<Pair> {
    let values: Vec<_> = index.bars.iter().map(|b| b.close).collect();
    let cache: Vec<_> = KINDS
        .iter()
        .flat_map(|k| settings.periods.iter().map(move |n| (*k, *n)))
        .map(|(k, n)| (k, n, average(&values, k, n)))
        .collect();
    let mut pairs = vec![];
    for (k, a, x) in &cache {
        for (l, b, y) in &cache {
            if a >= b {
                continue;
            }
            let mut prev = 0;
            let signal = x
                .iter()
                .zip(y)
                .map(|(x, y)| {
                    if let (Some(x), Some(y)) = (x, y) {
                        if x > y {
                            prev = 1
                        } else if x < y {
                            prev = -1
                        }
                    }
                    prev
                })
                .collect();
            pairs.push(Pair {
                fast: *k,
                slow: *l,
                a: *a,
                b: *b,
                signal,
            });
        }
    }
    pairs
}

struct Market<'a> {
    data: &'a Dataset,
    index: &'a Instrument,
    settings: &'a Settings,
    lookup: BTreeMap<String, BTreeMap<NaiveDate, usize>>,
    future_by_date: BTreeMap<NaiveDate, String>,
}
impl<'a> Market<'a> {
    fn new(data: &'a Dataset, index: &'a Instrument, settings: &'a Settings) -> Self {
        let lookup = data
            .instruments
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.bars
                        .iter()
                        .enumerate()
                        .map(|(i, b)| (b.date, i))
                        .collect(),
                )
            })
            .collect();
        let mut contracts: Vec<_> = data
            .instruments
            .values()
            .filter(|s| s.kind == "future" && s.expiry.is_some() && s.multiplier.is_some())
            .collect();
        contracts.sort_by_key(|s| s.expiry);
        let mut future_by_date = BTreeMap::new();
        for (i, b) in index.bars.iter().enumerate() {
            if let Some(c) = contracts.iter().find(|c| {
                let expiry = c.expiry.unwrap();
                let remaining = index
                    .bars
                    .partition_point(|b| b.date <= expiry)
                    .saturating_sub(i + 1);
                expiry > b.date
                    && (remaining > 5
                        || expiry > index.bars.last().unwrap().date + Duration::days(14))
                    && c.bars.binary_search_by_key(&b.date, |b| b.date).is_ok()
            }) {
                future_by_date.insert(b.date, c.ticker.clone());
            }
        }
        Self {
            data,
            index,
            settings,
            lookup,
            future_by_date,
        }
    }
    fn bar(&self, ticker: &str, d: NaiveDate) -> Option<&Bar> {
        self.lookup
            .get(ticker)?
            .get(&d)
            .map(|i| &self.data.instruments[ticker].bars[*i])
    }
    fn asset(
        &self,
        id: u8,
        real: bool,
        d: NaiveDate,
        signal: i8,
    ) -> Option<(&str, &Bar, f64, u32)> {
        let ticker = if signal <= 0 && id == 2 {
            "LQDT"
        } else if !real {
            "IMOEX"
        } else if id == 2 {
            &self.settings.fund
        } else {
            self.future_by_date.get(&d)?
        };
        let ins = self.data.instruments.get(ticker)?;
        Some((
            &ins.ticker,
            self.bar(ticker, d)?,
            ins.multiplier.unwrap_or(1.0),
            if real || ticker == "LQDT" {
                ins.lot_size
            } else {
                1
            },
        ))
    }
}

#[derive(Clone)]
struct Position {
    ticker: String,
    side: f64,
    quantity: f64,
    multiplier: f64,
    entry: f64,
    entry_date: NaiveDate,
    last: f64,
    pnl: f64,
}

fn simulate_ma(
    m: &Market,
    id: u8,
    real: bool,
    chosen: &[(usize, usize)],
    pairs: &[Pair],
    range: std::ops::Range<usize>,
    trace: bool,
) -> Result<Run, String> {
    let (begin, end) = (range.start, range.end);
    let mut run = Run::unavailable(
        "",
        if real { "real" } else { "index" },
        "",
        m.settings.capital,
    );
    run.message = None;
    let mut equity = m.settings.capital;
    let mut pos: Option<Position> = None;
    let mut chosen_idx = 0;
    let mut postponed_weekends = false;
    let base_index = m.index.bars[begin].open;
    let base_lqdt = m.bar("LQDT", m.index.bars[begin].date).map(|b| b.open);
    for i in begin..end {
        let d = m.index.bars[i].date;
        while chosen_idx + 1 < chosen.len() && chosen[chosen_idx + 1].0 <= i {
            chosen_idx += 1;
        }
        let p = &pairs[chosen[chosen_idx].1];
        // Only yesterday's completed close can generate today's trade.
        let signal = if i > 0 { p.signal[i - 1] } else { 0 };
        if signal == 0 {
            return Err(format!(
                "Недостаточно истории для прогрева {} на {d}",
                p.name()
            ));
        }
        // An index candle may exist on an additional weekend session while MX has no trades.
        // Keep the last marked futures position and execute only at its next observed opening.
        let weekend = matches!(d.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
        if id == 1
            && real
            && weekend
            && (m.asset(id, real, d, signal).is_none()
                || pos.as_ref().is_some_and(|p| m.bar(&p.ticker, d).is_none()))
        {
            postponed_weekends = true;
            if trace {
                run.points.push(Point {
                    mcftr: None,
                    date: d,
                    equity,
                    benchmark: Some(m.settings.capital * m.index.bars[i].close / base_index),
                    lqdt: base_lqdt.and_then(|base| {
                        m.bar("LQDT", d)
                            .map(|b| m.settings.capital * b.close / base)
                    }),
                });
            }
            continue;
        }
        let (ticker, bar, mult, lot) = m
            .asset(id, real, d, signal)
            .ok_or_else(|| format!("Нет котировки или спецификации инструмента на {d}"))?;
        let side = if signal < 0 && id == 1 { -1.0 } else { 1.0 };
        if let Some(p) = &mut pos {
            let b = m
                .bar(&p.ticker, d)
                .ok_or_else(|| format!("Нет котировки открытой позиции {} на {d}", p.ticker))?;
            let overnight = p.quantity * p.multiplier * p.side * (b.open - p.last);
            equity += overnight;
            p.pnl += overnight;
            p.last = b.open;
        }
        let cap = equity.max(0.0);
        let reduce = pos.as_ref().is_some_and(|p| {
            p.ticker == ticker && p.quantity * mult * bar.open > cap + 1e-7 && id == 1
        });
        let switch = pos
            .as_ref()
            .is_none_or(|p| p.ticker != ticker || p.side != side)
            || reduce;
        if switch {
            if let Some(p) = pos.take() {
                if trace {
                    run.trades.push(Trade {
                        ticker: p.ticker.clone(),
                        side: if p.side > 0.0 { "long" } else { "short" }.into(),
                        entry_date: p.entry_date,
                        exit_date: Some(d),
                        entry_price: p.entry,
                        exit_price: Some(p.last),
                        quantity: p.quantity,
                        pnl: p.pnl,
                        reason: if reduce {
                            "Ограничение номинала капиталом"
                        } else if p.ticker != ticker && id == 1 && real {
                            "Перекладка / смена сигнала"
                        } else {
                            "Пересечение скользящих"
                        }
                        .into(),
                    });
                }
            }
            let units = cap / (bar.open * mult);
            let quantity = if real || ticker == "LQDT" {
                (units / lot as f64).floor() * lot as f64
            } else {
                units
            };
            pos = Some(Position {
                ticker: ticker.into(),
                side,
                quantity,
                multiplier: mult,
                entry: bar.open,
                entry_date: d,
                last: bar.open,
                pnl: 0.0,
            });
        }
        if let Some(p) = &mut pos {
            let pnl = p.quantity * p.multiplier * p.side * (bar.close - bar.open);
            equity += pnl;
            p.pnl += pnl;
            p.last = bar.close;
        }
        if !equity.is_finite() {
            return Err("Переполнение капитала".into());
        }
        if trace {
            run.points.push(Point {
                mcftr: None,
                date: d,
                equity,
                benchmark: Some(m.settings.capital * m.index.bars[i].close / base_index),
                lqdt: base_lqdt.and_then(|base| {
                    m.bar("LQDT", d)
                        .map(|b| m.settings.capital * b.close / base)
                }),
            });
        }
        if equity <= 0.0 {
            break;
        }
    }
    run.final_equity = equity;
    run.total_return = (equity / m.settings.capital - 1.0) * 100.0;
    run.parameters = pairs[chosen.last().unwrap().1].name();
    if trace {
        if postponed_weekends {
            run.warnings.push("В дополнительные сессии выходного дня без свечи MX сделка откладывается до следующего наблюдаемого открытия фьючерса, позиция оценивается по последней доступной цене. Индексные сигналы продолжают обновляться.".into());
        }
        let signal = pairs[chosen.last().unwrap().1].signal[end - 1];
        if let Some((ticker, _, _, _)) = m.asset(id, real, m.index.bars[end - 1].date, signal) {
            run.next_action=format!("На следующей сессии: {} {}. Основание: {} по закрытию {}. Держать до обратного пересечения. Параметры фиксированы на всём периоде с 2023 года.",if id==1 && signal<0{"шорт"}else{"лонг"},ticker,run.parameters,m.index.bars[end-1].date);
        }
        if let Some(p) = pos {
            let value = p.quantity * p.multiplier * p.last;
            run.holdings.push(Holding {
                ticker: p.ticker.clone(),
                side: if p.side > 0.0 { "long" } else { "short" }.into(),
                weight: if equity > 0.0 {
                    value / equity * 100.0
                } else {
                    0.0
                },
                reason: format!("{}; сигнал по закрытию предыдущей сессии", run.parameters),
                until: "До обратного пересечения".into(),
            });
            run.trades.push(Trade {
                ticker: p.ticker,
                side: if p.side > 0.0 { "long" } else { "short" }.into(),
                entry_date: p.entry_date,
                exit_date: None,
                entry_price: p.entry,
                exit_price: None,
                quantity: p.quantity,
                pnl: p.pnl,
                reason: "Позиция открыта; P&L по последнему закрытию".into(),
            });
        }
        run.finish();
    }
    Ok(run)
}

fn best_ma(
    m: &Market,
    id: u8,
    real: bool,
    pairs: &[Pair],
    begin: usize,
    end: usize,
) -> Vec<(usize, f64)> {
    let mut scores: Vec<_> = pairs
        .par_iter()
        .enumerate()
        .filter_map(|(k, _)| {
            simulate_ma(m, id, real, &[(begin, k)], pairs, begin..end, false)
                .ok()
                .map(|r| (k, r.total_return))
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scores
}

pub fn calculate(data: &Dataset, id: u8, settings: &Settings, progress: impl Fn(String)) -> Report {
    if id == 4 {
        return crate::mozgovik::calculate(settings, progress);
    }
    if id == 3 {
        return crate::dividend::calculate(data, settings, progress);
    }
    let mut report = Report {
        id,
        calculated_at: chrono::Utc::now().to_rfc3339(),
        settings: settings.clone(),
        runs: vec![],
        warnings: data.warnings.clone(),
    };
    let Some(index) = data.instruments.get("IMOEX").filter(|s| s.bars.len() > 2) else {
        for mode in ["index", "real"] {
            {
                let label = "Исторический оптимум";
                report.runs.push(Run::unavailable(
                    label,
                    mode,
                    "Нет истории IMOEX. Загрузите данные на странице источников.",
                    settings.capital,
                ));
            }
        }
        return report;
    };
    let begin = index.bars.partition_point(|b| b.date < settings.start);
    if begin >= index.bars.len() || begin < settings.periods.iter().max().unwrap() * 3 {
        for mode in ["index", "real"] {
            {
                let label = "Исторический оптимум";
                report.runs.push(Run::unavailable(label,mode,"Недостаточно данных до 2023 года для прогрева (нужно минимум 3 максимальных периода).",settings.capital));
            }
        }
        return report;
    }
    progress("Вычисление восьми видов скользящих".into());
    let pairs = pairs(index, settings);
    let m = Market::new(data, index, settings);
    for real in [false, true] {
        let mode = if real { "real" } else { "index" };
        progress(format!("{}: полный перебор {} пар", mode, pairs.len()));
        let scores = best_ma(&m, id, real, &pairs, begin, index.bars.len());
        if let Some((k, _)) = scores.first() {
            let mut run = simulate_ma(
                &m,
                id,
                real,
                &[(begin, *k)],
                &pairs,
                begin..index.bars.len(),
                true,
            )
            .unwrap();
            run.label = "Исторический оптимум".into();
            run.tested = pairs.len();
            run.leaderboard = scores
                .iter()
                .take(15)
                .map(|(i, r)| Ranked {
                    parameters: pairs[*i].name(),
                    total_return: *r,
                    events: 0,
                })
                .collect();
            run.warnings.push(
                "Параметры подобраны на всей показанной истории. Эта доходность включает подгонку."
                    .into(),
            );
            report.runs.push(run);
        } else {
            let msg = simulate_ma(
                &m,
                id,
                real,
                &[(begin, 0)],
                &pairs,
                begin..index.bars.len(),
                false,
            )
            .err()
            .unwrap_or_else(|| "Нет допустимых пар".into());
            report.runs.push(Run::unavailable(
                "Исторический оптимум",
                mode,
                msg,
                settings.capital,
            ));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> (Dataset, Settings) {
        let start = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let bars = (0..450)
            .map(|i| Bar {
                date: start + Duration::days(i),
                open: 100.0 + i as f64,
                close: 100.5 + i as f64,
                volume: 1.0,
            })
            .collect();
        let mut d = Dataset::default();
        d.instruments.insert(
            "IMOEX".into(),
            Instrument {
                ticker: "IMOEX".into(),
                name: "index".into(),
                kind: "index".into(),
                source: "https://iss.moex.com".into(),
                lot_size: 1,
                multiplier: None,
                expiry: None,
                bars,
            },
        );
        let s = Settings {
            periods: vec![2, 3],
            training_start: start,
            start: start + Duration::days(365),
            ..Default::default()
        };
        (d, s)
    }
    #[test]
    fn next_open_execution_and_overnight_accounting() {
        let (d, s) = sample();
        let index = &d.instruments["IMOEX"];
        let m = Market::new(&d, index, &s);
        let mut signal = vec![1; 450];
        signal[367] = -1;
        let p = Pair {
            fast: Kind::Sma,
            slow: Kind::Sma,
            a: 2,
            b: 3,
            signal,
        };
        let r = simulate_ma(&m, 1, false, &[(365, 0)], &[p], 365..370, true).unwrap();
        assert_eq!(r.trades[0].exit_date, Some(index.bars[368].date));
        let expected = s.capital + r.trades.iter().map(|t| t.pnl).sum::<f64>();
        assert!((expected - r.final_equity).abs() < 1e-6);
    }
    #[test]
    fn future_roll_does_not_create_price_jump_profit() {
        let (mut d, s) = sample();
        let idx = d.instruments["IMOEX"].clone();
        for (ticker, expiry, offset) in [("MXH3", 375, 0.0), ("MXM3", 460, 500.0)] {
            let mut c = idx.clone();
            c.ticker = ticker.into();
            c.kind = "future".into();
            c.expiry = Some(idx.bars[0].date + Duration::days(expiry));
            c.multiplier = Some(1.0);
            for b in &mut c.bars {
                b.open += offset;
                b.close = b.open;
            }
            d.instruments.insert(ticker.into(), c);
        }
        let m = Market::new(&d, &d.instruments["IMOEX"], &s);
        let p = Pair {
            fast: Kind::Sma,
            slow: Kind::Sma,
            a: 2,
            b: 3,
            signal: vec![1; 450],
        };
        let r = simulate_ma(&m, 1, true, &[(365, 0)], &[p], 365..380, true).unwrap();
        assert_eq!(r.trades[0].ticker, "MXH3");
        assert_eq!(r.trades[1].ticker, "MXM3");
        assert!(
            (r.final_equity - s.capital - r.trades.iter().map(|t| t.pnl).sum::<f64>()).abs() < 1e-6
        );
        assert!(r.total_return < 5.0);
    }
    #[test]
    fn fixed_search_returns_one_result_per_instrument_mode() {
        let (data, settings) = sample();
        let report = calculate(&data, 1, &settings, |_| {});
        assert_eq!(report.runs.len(), 2);
        assert!(report
            .runs
            .iter()
            .all(|r| r.label == "Исторический оптимум" && r.selections.is_empty()));
        let run = report.runs.iter().find(|r| r.mode == "index").unwrap();
        assert_eq!(run.status, "ready");
        assert!((run.total_return - run.leaderboard[0].total_return).abs() < 1e-8);
    }
    #[test]
    fn weekend_index_signals_wait_for_an_actual_futures_opening() {
        let (mut d, s) = sample();
        let mut c = d.instruments["IMOEX"].clone();
        c.ticker = "MXTEST".into();
        c.kind = "future".into();
        c.expiry = Some(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
        c.multiplier = Some(1.0);
        c.bars.retain(|b| {
            !matches!(
                b.date.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            )
        });
        d.instruments.insert(c.ticker.clone(), c);
        let m = Market::new(&d, &d.instruments["IMOEX"], &s);
        let mut signal = vec![1; 450];
        for value in &mut signal[371..] {
            *value = -1;
        }
        let p = Pair {
            fast: Kind::Sma,
            slow: Kind::Sma,
            a: 2,
            b: 3,
            signal,
        };
        let r = simulate_ma(&m, 1, true, &[(365, 0)], &[p], 365..380, true).unwrap();
        assert!(r.trades.iter().all(|t| !matches!(
            t.entry_date.weekday(),
            chrono::Weekday::Sat | chrono::Weekday::Sun
        )));
        assert_eq!(
            r.trades[0].exit_date,
            Some(NaiveDate::from_ymd_opt(2023, 1, 9).unwrap())
        );
        assert!(
            (s.capital + r.trades.iter().map(|t| t.pnl).sum::<f64>() - r.final_equity).abs() < 1e-6
        );
        assert!(!r.warnings.is_empty());
    }
}
