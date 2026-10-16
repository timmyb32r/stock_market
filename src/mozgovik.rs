use crate::model::*;
use chrono::{Duration, NaiveDate};
use serde::Deserialize;

pub const SOURCE_JSON: &str = include_str!("../sources/mozgovik.json");

#[derive(Deserialize)]
struct Signal {
    ticker: String,
    date_buy: NaiveDate,
    price_buy: f64,
    date_sell: NaiveDate,
    price_sell: f64,
}
#[derive(Deserialize)]
struct Source {
    trades: Vec<Signal>,
    rates: Vec<(NaiveDate, f64)>,
}
fn simulate(source: &Source, capital: f64, slots: usize) -> Run {
    let mut run = Run::unavailable("Исторический оптимум", "shares", "", capital);
    run.message = None;
    run.parameters = format!("Mozgovik · N={slots}");
    let mut cash = capital;
    let mut positions: Vec<(usize, f64)> = Vec::new();
    let mut skipped = 0;
    let mut day = NaiveDate::from_ymd_opt(2023, 8, 25).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
    while day <= end {
        // Preserve calc.html: sells first, alphabetical buys, fractional shares,
        // entry-cost valuation and daily interest on cash (including negatives).
        for (i, signal) in source.trades.iter().enumerate() {
            if signal.date_sell == day {
                if let Some(pos) = positions.iter().position(|(id, _)| *id == i) {
                    let (_, quantity) = positions.remove(pos);
                    cash += quantity * signal.price_sell;
                    run.trades.push(Trade {
                        ticker: signal.ticker.clone(),
                        side: "long".into(),
                        entry_date: signal.date_buy,
                        exit_date: Some(day),
                        entry_price: signal.price_buy,
                        exit_price: Some(signal.price_sell),
                        quantity,
                        pnl: quantity * (signal.price_sell - signal.price_buy),
                        reason: "Сигнал из calc.html · рейтинг Mozgovik".into(),
                    });
                }
            }
        }
        for (i, signal) in source
            .trades
            .iter()
            .enumerate()
            .filter(|(_, s)| s.date_buy == day)
        {
            if positions.len() >= slots {
                skipped += 1;
                continue;
            }
            let nav = cash
                + positions
                    .iter()
                    .map(|(id, q)| q * source.trades[*id].price_buy)
                    .sum::<f64>();
            let budget = nav / slots as f64;
            positions.push((i, budget / signal.price_buy));
            cash -= budget;
        }
        let rate = source
            .rates
            .iter()
            .rev()
            .find(|(date, _)| *date <= day)
            .unwrap()
            .1;
        cash *= (1.0 + rate / 100.0).powf(1.0 / 365.0);
        let equity = cash
            + positions
                .iter()
                .map(|(id, q)| q * source.trades[*id].price_buy)
                .sum::<f64>();
        run.points.push(Point {
            date: day,
            equity,
            benchmark: None,
            lqdt: None,
            mcftr: None,
        });
        day += Duration::days(1);
    }
    run.finish();
    run.warnings = vec![
        format!("Источник — 38 сделок пользователя из calc.html. Исполнено {}, пропущено из-за занятых слотов {skipped}.", run.trades.len()),
        "Воспроизведение исходной модели: дробные акции, оценка открытых позиций по цене входа, ставка на свободные деньги из таблицы calc.html. Это не ежедневная рыночная переоценка; просадка отражает эту условность.".into(),
        "Как в исходнике, размер нового слота равен капиталу / N без ограничения остатком денег: возможен отрицательный остаток. Календарь, цены и ставки исходника сохранены без исправлений и независимого подтверждения.".into(),
        "N выбирается задним числом по максимальной доходности. История начинается 25.08.2023, заканчивается 24.07.2026; новые сигналы автоматически не поступают.".into(),
    ];
    run.next_action = "Новых сигналов Mozgovik в исходном файле нет. Все 38 сделок завершены; на 24.07.2026 капитал свободен. Это состояние на конец исходной истории, а не актуальная рекомендация на сегодня.".into();
    run.holdings.push(Holding {
        ticker: "Денежные средства · ставка из calc.html".into(),
        side: "long".into(),
        weight: 100.0,
        reason: "Все сделки исходного списка завершены".into(),
        until: "Нужны новые сигналы рейтинга".into(),
    });
    run
}
fn market_run(source: &Source, data: &Dataset, capital: f64, slots: usize) -> Result<Run, String> {
    let start = NaiveDate::from_ymd_opt(2023, 8, 25).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
    let fund = data.instruments.get("LQDT").ok_or("Нет истории LQDT")?;
    if fund.bars.first().is_none_or(|b| b.date > start)
        || fund.bars.last().is_none_or(|b| b.date < end)
    {
        return Err("Нужна полная история LQDT за период модели".into());
    }
    let mut plans = Vec::new();
    for signal in &source.trades {
        let ins = data
            .instruments
            .get(&signal.ticker)
            .ok_or_else(|| format!("Нет котировок {}. Обновите данные.", signal.ticker))?;
        let buy = ins
            .bars
            .iter()
            .find(|b| b.date >= signal.date_buy)
            .ok_or_else(|| format!("Нет открытия {} после {}", signal.ticker, signal.date_buy))?;
        let sell = ins
            .bars
            .iter()
            .find(|b| b.date >= signal.date_sell)
            .ok_or_else(|| format!("Нет открытия {} после {}", signal.ticker, signal.date_sell))?;
        if buy.date >= sell.date || sell.date > end {
            return Err(format!("Неполный торговый календарь {}", signal.ticker));
        }
        if ins.bars.windows(2).any(|w| {
            w[1].date > buy.date
                && w[1].date <= sell.date
                && !(0.33..=3.0).contains(&(w[1].open / w[0].close))
        }) {
            return Err(format!(
                "Нужна проверка корпоративного действия {}",
                signal.ticker
            ));
        }
        plans.push((signal, ins, buy, sell));
    }
    plans.sort_by(|a, b| a.2.date.cmp(&b.2.date).then(a.0.ticker.cmp(&b.0.ticker)));
    let mut run = Run::unavailable("Исторический оптимум", "market", "", capital);
    run.message = None;
    run.parameters = format!("Mozgovik · N={slots} · рыночная переоценка");
    let mut cash = capital;
    let mut positions: Vec<(usize, f64)> = Vec::new();
    let mut previous_fund_close = fund
        .bars
        .iter()
        .rev()
        .find(|b| b.date < start)
        .ok_or("Нет базы LQDT")?
        .close;
    let mut skipped = 0;
    let mut day = start;
    while day <= end {
        let fund_bar = fund.bars.iter().find(|b| b.date == day);
        let fund_open = fund_bar.map_or(previous_fund_close, |b| b.open);
        cash *= fund_open / previous_fund_close;
        for (i, (signal, _, _, sell)) in plans.iter().enumerate().filter(|(_, p)| p.3.date == day) {
            if let Some(pos) = positions.iter().position(|(id, _)| *id == i) {
                let (_, quantity) = positions.remove(pos);
                cash += quantity * sell.open;
                run.trades.push(Trade {
                    ticker: signal.ticker.clone(),
                    side: "long".into(),
                    entry_date: plans[i].2.date,
                    exit_date: Some(day),
                    entry_price: plans[i].2.open,
                    exit_price: Some(sell.open),
                    quantity,
                    pnl: quantity * (sell.open - plans[i].2.open),
                    reason: "Сигнал calc.html · реальные открытия MOEX".into(),
                });
            }
        }
        for (i, (_, _, buy, _)) in plans.iter().enumerate().filter(|(_, p)| p.2.date == day) {
            if positions.len() >= slots {
                skipped += 1;
                continue;
            }
            let nav = cash
                + positions
                    .iter()
                    .map(|(id, q)| {
                        let ins = plans[*id].1;
                        let price = ins
                            .bars
                            .iter()
                            .find(|b| b.date == day)
                            .map(|b| b.open)
                            .or_else(|| {
                                ins.bars
                                    .iter()
                                    .rev()
                                    .find(|b| b.date < day)
                                    .map(|b| b.close)
                            })
                            .unwrap();
                        q * price
                    })
                    .sum::<f64>();
            let budget = (nav / slots as f64).min(cash).max(0.0);
            if budget == 0.0 {
                skipped += 1;
                continue;
            }
            cash -= budget;
            positions.push((i, budget / buy.open));
        }
        let fund_close = fund_bar.map_or(previous_fund_close, |b| b.close);
        cash *= fund_close / fund_open;
        previous_fund_close = fund_close;
        let equity = cash
            + positions
                .iter()
                .map(|(id, q)| {
                    q * plans[*id]
                        .1
                        .bars
                        .iter()
                        .rev()
                        .find(|b| b.date <= day)
                        .unwrap()
                        .close
                })
                .sum::<f64>();
        run.points.push(Point {
            date: day,
            equity,
            benchmark: None,
            lqdt: None,
            mcftr: None,
        });
        day += Duration::days(1);
    }
    run.finish();
    run.warnings=vec![format!("Исполнено {} сигналов, пропущено {skipped}. N оптимизирован отдельно для рыночного варианта.",run.trades.len()), "Сигналы — даты из calc.html; исполнение по реальному открытию первой доступной сессии в эту дату или позже. Акции ежедневно оцениваются по закрытию MOEX; вне сессий сохраняется последнее закрытие.".into(), "Свободные средства следуют фактическим котировкам LQDT, без замены ключевой ставкой. Используются дробные акции и паи для сравнения моделей; бюджет ограничен доступными деньгами, заёмных средств нет.".into(), "Дивиденды по акциям не начисляются; это ценовой результат сделок плюс LQDT. Корпоративные действия с разрывом цены более чем втрое блокируют расчёт до проверки. Комиссии и проскальзывание не учитываются.".into(), "История сигналов заканчивается 24.07.2026. Рыночный вариант отличается от исходника ценами исполнения, оценкой позиций и доходом свободных средств.".into()];
    run.next_action="Все сигналы исходника завершены. На 24.07.2026 капитал в LQDT; новых сигналов рейтинга нет. Это состояние на конец истории.".into();
    run.holdings.push(Holding {
        ticker: "LQDT".into(),
        side: "long".into(),
        weight: 100.0,
        reason: "Все позиции закрыты".into(),
        until: "До новых сигналов рейтинга".into(),
    });
    Ok(run)
}

pub fn calculate(data: &Dataset, settings: &Settings, progress: impl Fn(String)) -> Report {
    let mut source: Source = serde_json::from_str(SOURCE_JSON).expect("valid bundled source");
    source
        .trades
        .sort_by(|a, b| a.date_buy.cmp(&b.date_buy).then(a.ticker.cmp(&b.ticker)));
    let mut runs = Vec::new();
    for &n in &settings.slots {
        progress(format!("Mozgovik: перебор N={n}"));
        runs.push(simulate(&source, settings.capital, n));
    }
    runs.sort_by(|a, b| b.total_return.total_cmp(&a.total_return));
    let leaderboard = runs
        .iter()
        .map(|r| Ranked {
            parameters: r.parameters.clone(),
            total_return: r.total_return,
            events: r.trades.len(),
        })
        .collect();
    let mut best = runs.remove(0);
    best.tested = settings.slots.len();
    best.leaderboard = leaderboard;
    Report {
        id: 4,
        calculated_at: chrono::Utc::now().to_rfc3339(),
        settings: settings.clone(),
        runs: {
            let mut market = Vec::new();
            let mut failure = None;
            for &n in &settings.slots {
                progress(format!("Mozgovik: рыночная переоценка N={n}"));
                match market_run(&source, data, settings.capital, n) {
                    Ok(r) => market.push(r),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            let real = if let Some(error) = failure {
                Run::unavailable("Исторический оптимум", "market", error, settings.capital)
            } else {
                market.sort_by(|a, b| b.total_return.total_cmp(&a.total_return));
                let ranking = market
                    .iter()
                    .map(|r| Ranked {
                        parameters: r.parameters.clone(),
                        total_return: r.total_return,
                        events: r.trades.len(),
                    })
                    .collect();
                let mut r = market.remove(0);
                r.tested = settings.slots.len();
                r.leaderboard = ranking;
                r
            };
            vec![best, real]
        },
        warnings: vec![],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn market_marks_daily_and_uses_actual_fund_returns() {
        let start = NaiveDate::from_ymd_opt(2023, 8, 25).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        let sell = start + Duration::days(3);
        let source = Source {
            trades: vec![Signal {
                ticker: "TEST".into(),
                date_buy: start,
                price_buy: 999.0,
                date_sell: sell,
                price_sell: 999.0,
            }],
            rates: vec![],
        };
        let mut data = Dataset::default();
        let instrument = |ticker: &str, bars: Vec<Bar>| Instrument {
            ticker: ticker.into(),
            name: ticker.into(),
            kind: "share".into(),
            source: "https://example.test".into(),
            lot_size: 1,
            multiplier: None,
            expiry: None,
            bars,
        };
        data.instruments.insert(
            "TEST".into(),
            instrument(
                "TEST",
                vec![
                    Bar {
                        date: start,
                        open: 100.0,
                        close: 110.0,
                        volume: 1.0,
                    },
                    Bar {
                        date: start + Duration::days(1),
                        open: 110.0,
                        close: 80.0,
                        volume: 1.0,
                    },
                    Bar {
                        date: sell,
                        open: 120.0,
                        close: 120.0,
                        volume: 1.0,
                    },
                ],
            ),
        );
        data.instruments.insert(
            "LQDT".into(),
            instrument(
                "LQDT",
                vec![
                    Bar {
                        date: start - Duration::days(1),
                        open: 100.0,
                        close: 100.0,
                        volume: 1.0,
                    },
                    Bar {
                        date: start,
                        open: 100.0,
                        close: 101.0,
                        volume: 1.0,
                    },
                    Bar {
                        date: sell,
                        open: 101.0,
                        close: 101.0,
                        volume: 1.0,
                    },
                    Bar {
                        date: end,
                        open: 202.0,
                        close: 202.0,
                        volume: 1.0,
                    },
                ],
            ),
        );
        let r = market_run(&source, &data, 1000.0, 1).unwrap();
        assert_eq!(r.points[0].equity, 1100.0);
        assert_eq!(r.points[1].equity, 800.0);
        assert!((r.final_equity - 2400.0).abs() < 1e-8);
        assert!((r.max_drawdown - 100.0 * 300.0 / 1100.0).abs() < 1e-8);
        assert_eq!(r.trades[0].entry_price, 100.0);
        assert_eq!(r.trades[0].pnl, 200.0);
    }
    #[test]
    fn matches_original_javascript_for_all_fourteen_slot_counts() {
        let mut source: Source = serde_json::from_str(SOURCE_JSON).unwrap();
        source
            .trades
            .sort_by(|a, b| a.date_buy.cmp(&b.date_buy).then(a.ticker.cmp(&b.ticker)));
        let expected = [
            2149037.856462668,
            1999752.5262636386,
            1968421.7556127529,
            2111200.3450280894,
            2027855.6052842028,
            1969013.4396634619,
            1942990.7799333532,
            1897101.0439436396,
            1915330.090640401,
            1933911.4933616766,
            1973768.3390491565,
            1945768.3342504415,
            1940778.1431570677,
            1935782.9879975482,
        ];
        for (i, value) in expected.iter().enumerate() {
            assert!((simulate(&source, 1_000_000.0, i + 1).final_equity - value).abs() < 0.000001);
        }
    }
    #[test]
    fn source_replay_accounts_for_stock_profit_and_cash_interest() {
        let report = calculate(&Dataset::default(), &Settings::default(), |_| {});
        let r = &report.runs[0];
        assert_eq!(r.leaderboard.len(), 14);
        assert_eq!(r.start.unwrap().to_string(), "2023-08-25");
        assert_eq!(r.end.unwrap().to_string(), "2026-07-24");
        assert!(r.final_equity > r.initial + r.trades.iter().map(|t| t.pnl).sum::<f64>());
        assert!(r.trades.iter().all(|t| t.exit_date.is_some()));
    }
}
