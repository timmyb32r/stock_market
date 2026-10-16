use crate::model::*;
use chrono::NaiveDate;
use std::collections::BTreeMap;

fn baseline(ins: &Instrument, start: NaiveDate) -> Option<f64> {
    if ins.ticker == "MCFTR" {
        let i = ins
            .bars
            .partition_point(|b| b.date < start)
            .checked_sub(1)?;
        Some(ins.bars[i].close)
    } else {
        ins.bars
            .binary_search_by_key(&start, |b| b.date)
            .ok()
            .map(|i| ins.bars[i].open)
    }
}
fn value(ins: &Instrument, date: NaiveDate) -> Option<f64> {
    if date > ins.bars.last()?.date {
        return None;
    }
    let i = ins
        .bars
        .partition_point(|b| b.date <= date)
        .checked_sub(1)?;
    Some(ins.bars[i].close)
}
pub fn enrich(report: &mut Report, data: &Dataset) {
    for run in &mut report.runs {
        let Some(start) = run.start else {
            continue;
        };
        if report.id == 4 {
            if let Some(index) = data.instruments.get("IMOEX") {
                if let Some(base) = baseline(index, start) {
                    for point in &mut run.points {
                        point.benchmark = value(index, point.date).map(|v| run.initial * v / base);
                    }
                }
            }
        }
        for (ticker, total) in [("MCFTR", true), ("LQDT", false)] {
            let series = data.instruments.get(ticker);
            let base = series.and_then(|s| baseline(s, start));
            for p in &mut run.points {
                let amount = series
                    .zip(base)
                    .and_then(|(s, b)| value(s, p.date).map(|v| run.initial * v / b));
                if total {
                    p.mcftr = amount;
                } else {
                    p.lqdt = amount;
                }
            }
        }
    }
}
pub fn summary(data: &Dataset, settings: &Settings) -> BTreeMap<String, Run> {
    ["MCFTR","LQDT"].into_iter().map(|ticker|{
        let mut run=Run::unavailable("Купить и держать","passive",format!("Нет полной истории {ticker} с начала 2023 года"),settings.capital);
        if let Some(ins)=data.instruments.get(ticker) {
            let source_start=ins.bars.iter().find(|b|b.date>=settings.start).map(|b|b.date);
            let first_session=data.instruments.get("IMOEX").and_then(|i|i.bars.iter().find(|b|b.date>=settings.start).map(|b|b.date)).or(source_start);
            let Some(first_session)=first_session else{return (ticker.into(),run);};
            if ins.bars.binary_search_by_key(&first_session,|b|b.date).is_err(){return (ticker.into(),run);}
            if let Some(base)=baseline(ins,first_session) {
                let bars:Vec<_>=ins.bars.iter().filter(|b|b.date>=settings.start).collect();
                if let Some(first)=bars.first() {
                    let quantity=if ticker=="LQDT" {(settings.capital/(first.open*ins.lot_size as f64)).floor()*ins.lot_size as f64}else{settings.capital/base};
                    let cash=if ticker=="LQDT"{settings.capital-quantity*first.open}else{0.0};
                    run.message=None;run.parameters=if ticker=="MCFTR"{"MCFTR · полная доходность брутто с реинвестированием дивидендов"}else{"LQDT · покупка в первую сессию 2023 года и удержание"}.into();
                    run.points=bars.iter().map(|b|Point{date:b.date,equity:cash+quantity*b.close,benchmark:None,lqdt:None,mcftr:None}).collect();
                    run.finish();
                    run.holdings.push(Holding{ticker:ticker.into(),side:"long".into(),weight:if run.final_equity>0.0{quantity*bars.last().unwrap().close/run.final_equity*100.0}else{0.0},reason:run.parameters.clone(),until:"Без запланированного выхода · купить и держать".into()});
                    if ticker=="MCFTR"{run.warnings.push("Теоретический индекс с дивидендами до налогов; напрямую купить индекс нельзя. База — последнее закрытие до 2023 года.".into());}
                }
            }
        }
        (ticker.into(),run)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn total_return_includes_first_day_and_does_not_extend_past_source() {
        let mut d = Dataset::default();
        let start = NaiveDate::from_ymd_opt(2023, 1, 3).unwrap();
        d.instruments.insert(
            "MCFTR".into(),
            Instrument {
                ticker: "MCFTR".into(),
                name: "Total return".into(),
                kind: "index".into(),
                source: "https://example.test".into(),
                lot_size: 1,
                multiplier: None,
                expiry: None,
                bars: vec![
                    Bar {
                        date: NaiveDate::from_ymd_opt(2022, 12, 30).unwrap(),
                        open: 100.,
                        close: 100.,
                        volume: 0.,
                    },
                    Bar {
                        date: start,
                        open: 110.,
                        close: 110.,
                        volume: 0.,
                    },
                ],
            },
        );
        let r = summary(&d, &Settings::default());
        assert!((r["MCFTR"].total_return - 10.).abs() < 1e-8);
        assert_eq!(
            value(&d.instruments["MCFTR"], start + chrono::Duration::days(1)),
            None
        );
        assert_eq!(r["LQDT"].status, "unavailable");
    }
}
