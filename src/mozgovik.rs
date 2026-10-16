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
pub fn calculate(settings: &Settings, progress: impl Fn(String)) -> Report {
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
        runs: vec![best],
        warnings: vec![],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let report = calculate(&Settings::default(), |_| {});
        let r = &report.runs[0];
        assert_eq!(r.leaderboard.len(), 14);
        assert_eq!(r.start.unwrap().to_string(), "2023-08-25");
        assert_eq!(r.end.unwrap().to_string(), "2026-07-24");
        assert!(r.final_equity > r.initial + r.trades.iter().map(|t| t.pnl).sum::<f64>());
        assert!(r.trades.iter().all(|t| t.exit_date.is_some()));
    }
}
