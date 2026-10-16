use crate::model::*;
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate};
use futures_util::{stream, StreamExt};
use scraper::{Html, Selector};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, time::Duration as StdDuration};

pub fn today() -> NaiveDate {
    (chrono::Utc::now() + Duration::hours(3)).date_naive()
}
pub fn save<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec(value)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}
pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub struct Moex {
    client: reqwest::Client,
}
impl Moex {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent("MarketLab/0.1 local research")
                .connect_timeout(StdDuration::from_secs(8))
                .timeout(StdDuration::from_secs(25))
                .build()?,
        })
    }
    async fn json(&self, url: &str) -> Result<Value> {
        let mut last = None;
        for attempt in 0..2 {
            if attempt > 0 {
                tokio::time::sleep(StdDuration::from_millis(700)).await;
            }
            match self.client.get(url).send().await {
                Ok(r) if r.status().is_success() => {
                    return r.json().await.context("ISS вернул не JSON")
                }
                Ok(r) => {
                    last = Some(anyhow::anyhow!("ISS HTTP {}", r.status()));
                    if !r.status().is_server_error() && r.status().as_u16() != 429 {
                        break;
                    }
                }
                Err(e) => {
                    last = Some(e.into());
                }
            }
        }
        Err(last.unwrap()).with_context(|| format!("Не удалось загрузить {url}"))
    }
    async fn candles(
        &self,
        engine: &str,
        market: &str,
        board: Option<&str>,
        ticker: &str,
        from: NaiveDate,
        till: NaiveDate,
    ) -> Result<Vec<Bar>> {
        let path = board.map(|b| format!("/boards/{b}")).unwrap_or_default();
        let mut bars = vec![];
        let mut start = 0;
        loop {
            let url=format!("https://iss.moex.com/iss/engines/{engine}/markets/{market}{path}/securities/{ticker}/candles.json?from={from}&till={till}&interval=24&start={start}&iss.meta=off");
            let v = self.json(&url).await?;
            let rows = table(&v, "candles")?;
            if rows.is_empty() {
                break;
            }
            start += rows.len();
            for r in rows {
                if let (Some(date), Some(open), Some(close)) = (
                    string(&r, "begin")
                        .and_then(|s| s.get(..10))
                        .and_then(|s| s.parse().ok()),
                    number(&r, "open"),
                    number(&r, "close"),
                ) {
                    if open > 0.0 && close > 0.0 {
                        bars.push(Bar {
                            date,
                            open,
                            close,
                            volume: number(&r, "volume").unwrap_or(0.0),
                        });
                    }
                }
            }
            anyhow::ensure!(start < 20000, "Слишком много строк свечей {ticker}");
            tokio::time::sleep(StdDuration::from_millis(80)).await;
        }
        bars.sort_by_key(|b| b.date);
        bars.dedup_by_key(|b| b.date);
        Ok(bars)
    }
    async fn instrument(
        &self,
        ticker: &str,
        kind: &str,
        board: Option<&str>,
        old: Option<&Instrument>,
    ) -> Result<Instrument> {
        let (engine, market) = match kind {
            "index" => ("stock", "index"),
            "future" => ("futures", "forts"),
            _ => ("stock", "shares"),
        };
        let from = old
            .and_then(|s| s.bars.last())
            .map(|b| b.date - Duration::days(7))
            .unwrap_or(NaiveDate::from_ymd_opt(2013, 1, 1).unwrap());
        let mut bars = old.map(|s| s.bars.clone()).unwrap_or_default();
        let mut new = self
            .candles(
                engine,
                market,
                board,
                ticker,
                from,
                today() - Duration::days(1),
            )
            .await?;
        if kind == "fund" {
            // Funds migrated from TQTF to TQBR. Keep both historical board segments.
            new.extend(
                self.candles(
                    engine,
                    market,
                    Some("TQBR"),
                    ticker,
                    from,
                    today() - Duration::days(1),
                )
                .await?,
            );
            let merged: BTreeMap<_, _> = new.into_iter().map(|b| (b.date, b)).collect();
            new = merged.into_values().collect();
        }
        if !new.is_empty() {
            bars.retain(|b| b.date < from);
            bars.extend(new);
        }
        anyhow::ensure!(!bars.is_empty(), "Нет свечей {ticker}");
        let mut ins=old.cloned().unwrap_or(Instrument{ticker:ticker.into(),name:ticker.into(),kind:kind.into(),source:format!("https://iss.moex.com/iss/engines/{engine}/markets/{market}/securities/{ticker}/candles.json"),lot_size:1,multiplier:None,expiry:None,bars:vec![]});
        ins.bars = bars;
        if old.is_none() || kind == "future" && ins.multiplier.is_none() {
            let v = self
                .json(&format!(
                    "https://iss.moex.com/iss/securities/{ticker}.json?iss.meta=off"
                ))
                .await?;
            let rows = table(&v, "description")?;
            let desc: BTreeMap<_, _> = rows
                .iter()
                .filter_map(|r| {
                    Some((
                        string(r, "name")?.to_uppercase(),
                        string(r, "value")?.to_string(),
                    ))
                })
                .collect();
            if let Some(name) = desc.get("NAME").or_else(|| desc.get("SHORTNAME")) {
                ins.name = name.clone();
            }
            if let Some(lot) = desc.get("LOTSIZE").and_then(|x| x.parse().ok()) {
                ins.lot_size = lot;
            }
            if kind != "future" && kind != "index" {
                let v=self.json(&format!("https://iss.moex.com/iss/engines/stock/markets/shares/boards/TQBR/securities/{ticker}.json?iss.meta=off&iss.only=securities")).await?;
                if let Some(row) = table(&v, "securities")?.first() {
                    if let Some(lot) = number(row, "lotsize").filter(|l| *l >= 1.0) {
                        ins.lot_size = lot as u32;
                    }
                }
            }
            if kind == "future" {
                let minstep = desc.get("MINSTEP").and_then(|v| v.parse::<f64>().ok());
                let stepprice = desc.get("STEPPRICE").and_then(|v| v.parse::<f64>().ok());
                ins.multiplier = stepprice
                    .zip(minstep)
                    .filter(|(p, s)| *s > 0.0 && *p > 0.0)
                    .map(|(p, s)| p / s);
                ins.expiry = desc
                    .get("LASTTRADEDATE")
                    .or_else(|| desc.get("LSTTRADE"))
                    .and_then(|x| x.parse().ok());
                if ins.multiplier.is_none() {
                    // Expired series omit tick values. Open position value at settlement equals
                    // OPENPOSITION × SETTLEPRICE × RUB multiplier. Unlike WAPRICE/turnover,
                    // these fields refer to the same valuation time and have no averaging error.
                    let last = ins.bars.last().unwrap().date;
                    let first = last - Duration::days(30);
                    let h=self.json(&format!("https://iss.moex.com/iss/history/engines/futures/markets/forts/securities/{ticker}.json?from={first}&till={last}&iss.meta=off")).await?;
                    let values: Vec<_> = table(&h, "history")?
                        .iter()
                        .filter_map(|r| {
                            let value = number(r, "openpositionvalue")?;
                            let volume = number(r, "openposition")?;
                            let price = number(r, "settleprice")?;
                            if volume > 0.0 && price > 0.0 && value > 0.0 {
                                Some(value / (volume * price))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if values.len() >= 2 {
                        let avg = values.iter().sum::<f64>() / values.len() as f64;
                        let rounded = (avg * 1000.0).round() / 1000.0;
                        if rounded > 0.0
                            && values
                                .iter()
                                .all(|v| (v - rounded).abs() / rounded < 0.0001)
                        {
                            ins.multiplier = Some(rounded);
                        }
                    }
                }
                anyhow::ensure!(
                    ins.multiplier.is_some() && ins.expiry.is_some(),
                    "Не удалось подтвердить множитель / экспирацию {ticker}"
                );
            }
        }
        Ok(ins)
    }

    pub async fn refresh(
        &self,
        old: &Dataset,
        cache: &Path,
        progress: impl Fn(String),
    ) -> Result<Dataset> {
        let mut data = old.clone();
        data.warnings.clear();
        progress("Загрузка индекса IMOEX с 2013 года".into());
        let index = self
            .instrument(
                "IMOEX",
                "index",
                Some("SNDX"),
                data.instruments.get("IMOEX"),
            )
            .await?;
        data.instruments.insert("IMOEX".into(), index);
        progress("Загрузка индекса полной доходности MCFTR".into());
        match self
            .instrument(
                "MCFTR",
                "index",
                Some("SNDX"),
                data.instruments.get("MCFTR"),
            )
            .await
        {
            Ok(s) => {
                data.instruments.insert("MCFTR".into(), s);
            }
            Err(e) => data.warnings.push(format!("MCFTR: {e:#}")),
        }
        for ticker in ["LQDT", "TMOS", "SBMX"] {
            progress(format!("Загрузка {ticker}"));
            match self
                .instrument(ticker, "fund", Some("TQTF"), data.instruments.get(ticker))
                .await
            {
                Ok(s) => {
                    data.instruments.insert(ticker.into(), s);
                }
                Err(e) => data.warnings.push(format!("{ticker}: {e:#}")),
            }
        }
        save(cache, &data)?;
        progress("Каталог акций Мосбиржи, включая снятые с торгов бумаги".into());
        // ISS security finder includes historical instruments, avoiding a current-listing-only universe.
        let mut active = BTreeMap::new();
        let mut offset = 0;
        loop {
            let v=self.json(&format!("https://iss.moex.com/iss/history/engines/stock/markets/shares/boards/TQBR/listing.json?start={offset}&iss.meta=off")).await?;
            let rows = table(&v, "securities")?;
            if rows.is_empty() {
                break;
            }
            offset += rows.len();
            for r in rows {
                if string(&r, "history_till").is_some_and(|d| d >= "2023-01-01") {
                    if let Some(t) = string(&r, "secid") {
                        active.insert(t.to_string(), ());
                    }
                }
            }
            anyhow::ensure!(offset < 10000, "Слишком большой листинг TQBR");
        }
        let mut tickers = BTreeMap::new();
        let mut offset = 0;
        loop {
            let v=self.json(&format!("https://iss.moex.com/iss/securities.json?engine=stock&market=shares&is_trading=all&start={offset}&iss.meta=off")).await?;
            let rows = table(&v, "securities")?;
            if rows.is_empty() {
                break;
            }
            offset += rows.len();
            for r in rows {
                let kind = string(&r, "type").unwrap_or("");
                if !["common_share", "preferred_share"].contains(&kind) {
                    continue;
                }
                if let Some(t) = string(&r, "secid")
                    .filter(|t| t.len() <= 12 && t.chars().all(|c| c.is_ascii_alphanumeric()))
                {
                    if !active.contains_key(t) {
                        continue;
                    }
                    tickers.insert(
                        t.to_owned(),
                        string(&r, "shortname").unwrap_or(t).to_owned(),
                    );
                }
            }
            anyhow::ensure!(offset < 30000, "Слишком большой каталог ISS");
        }
        let mozgovik: Value = serde_json::from_str(crate::mozgovik::SOURCE_JSON)?;
        for signal in mozgovik["trades"].as_array().unwrap() {
            let ticker = signal["ticker"].as_str().unwrap();
            tickers
                .entry(ticker.to_owned())
                .or_insert_with(|| ticker.to_owned());
        }
        let total = tickers.len();
        let base = data.clone();
        let jobs = tickers.into_iter().map(|(ticker, name)| {
            let base = &base;
            async move {
                let result = self.dividend_history(&ticker).await;
                let has_events = result.as_ref().is_ok_and(|rows| {
                    rows.iter().any(|r| {
                        number(r, "value").is_some_and(|v| v > 0.0)
                            && string(r, "registryclosedate").is_some_and(|d| d >= "2013-01-01")
                    })
                });
                let required = mozgovik_ticker(&ticker);
                let instrument = if has_events || required {
                    Some(
                        self.instrument(
                            &ticker,
                            "share",
                            Some("TQBR"),
                            base.instruments.get(&ticker),
                        )
                        .await,
                    )
                } else {
                    None
                };
                (ticker, name, result, instrument)
            }
        });
        let mut jobs = stream::iter(jobs).buffer_unordered(4);
        let mut i = 0;
        while let Some((ticker, name, result, instrument)) = jobs.next().await {
            i += 1;
            progress(format!("Дивидендная история {ticker} · {i}/{total}"));
            match result {
                Ok(rows) => {
                    let events: Vec<_> = rows
                        .into_iter()
                        .filter_map(|r| {
                            let record_date = string(&r, "registryclosedate")?.parse().ok()?;
                            let amount = number(&r, "value")?;
                            if amount <= 0.0
                                || record_date < NaiveDate::from_ymd_opt(2013, 1, 1).unwrap()
                            {
                                return None;
                            }
                            if string(&r, "currencyid")
                                .is_some_and(|s| !["RUB", "SUR"].contains(&s))
                            {
                                return None;
                            }
                            let prev = data.dividends.iter().find(|e| {
                                e.ticker == ticker
                                    && e.record_date == record_date
                                    && (e.amount - amount).abs() < 0.001
                            });
                            Some(Dividend {
                                ticker: ticker.clone(),
                                record_date,
                                amount,
                                declared_on: prev.and_then(|p| p.declared_on),
                                last_buy_date: string(&r, "lastbuy")
                                    .and_then(|d| d.parse().ok())
                                    .filter(|d| *d <= record_date)
                                    .or_else(|| prev.and_then(|p| p.last_buy_date)),
                                source: string(&r, "source").map(str::to_owned).unwrap_or_else(
                                    || format!("https://smart-lab.ru/q/{ticker}/dividend/"),
                                ),
                                announcement_source: prev
                                    .and_then(|p| p.announcement_source.clone()),
                            })
                        })
                        .collect();
                    if events.is_empty() {
                        continue;
                    }
                    // Preserve curated future announcements not yet represented in ISS.
                    for e in events {
                        if let Some(p) = data.dividends.iter_mut().find(|p| {
                            p.ticker == e.ticker
                                && p.record_date == e.record_date
                                && (p.amount - e.amount).abs() < 0.001
                        }) {
                            *p = e;
                        } else {
                            data.dividends.push(e);
                        }
                    }
                    match instrument {
                        Some(Ok(mut s)) => {
                            s.name = name;
                            data.instruments.insert(ticker.clone(), s);
                        }
                        Some(Err(e)) => data.warnings.push(format!("{ticker}: {e:#}")),
                        None => {}
                    }
                }
                Err(e) => data.warnings.push(format!("Дивиденды {ticker}: {e:#}")),
            }
            if i % 8 == 0 {
                save(cache, &data)?;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }
        save(cache, &data)?;
        // Explicit declaration evidence is supplemented by the local curated public-source registry.
        progress("Проверка публичных объявлений дивидендов".into());
        for e in self.curated_events()? {
            if let Some(p) = data.dividends.iter_mut().find(|p| {
                p.ticker == e.ticker
                    && p.record_date == e.record_date
                    && (p.amount - e.amount).abs() < 0.001
            }) {
                let last = p.last_buy_date;
                *p = e;
                if p.last_buy_date.is_none() {
                    p.last_buy_date = last;
                }
            } else {
                data.dividends.push(e);
            }
        }
        let eligible: Vec<_> = data
            .dividends
            .iter()
            .filter(|e| e.declared_on.is_some())
            .map(|e| e.ticker.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for ticker in eligible {
            progress(format!("Последний день с дивидендом: {ticker}"));
            match self.dividend_dates(&ticker).await {
                Ok(dates) => {
                    for e in data.dividends.iter_mut().filter(|e| e.ticker == ticker) {
                        if let Some(last) = dates.get(&e.record_date) {
                            e.last_buy_date = Some(*last);
                        }
                    }
                }
                Err(e) => data.warnings.push(format!("Календарь {ticker}: {e:#}")),
            }
        }
        save(cache, &data)?;
        progress("Ключевая ставка ЦБ для резервного расчёта свободного капитала".into());
        match self.key_rates().await {
            Ok(s) => {
                data.instruments.insert(s.ticker.clone(), s);
            }
            Err(e) => data.warnings.push(format!("ЦБ: {e:#}")),
        }
        // Quarterly IMOEX contracts. Expired contracts stay in the cache unchanged.
        for year in 2014..=today().year() + 1 {
            for code in ["H", "M", "U", "Z"] {
                let ticker = format!("MX{code}{}", year % 10);
                // Identifiers repeat every ten years. For the requested 2023+ real backtest only 2022 onward is required.
                if year < 2022 {
                    continue;
                }
                if data
                    .instruments
                    .get(&ticker)
                    .is_some_and(|s| s.expiry.is_some_and(|d| d < today() - Duration::days(7)))
                {
                    continue;
                }
                progress(format!("Спецификация и свечи фьючерса {ticker}"));
                match self
                    .instrument(
                        &ticker,
                        "future",
                        Some("RFUD"),
                        data.instruments.get(&ticker),
                    )
                    .await
                {
                    Ok(s) => {
                        data.instruments.insert(ticker, s);
                    }
                    Err(e) => data.warnings.push(format!("{ticker}: {e:#}")),
                }
            }
        }
        data.updated_at = Some(chrono::Utc::now().to_rfc3339());
        data.validate(today())?;
        Ok(data)
    }
    fn curated_events(&self) -> Result<Vec<Dividend>> {
        Ok(serde_json::from_str(include_str!(
            "../sources/dividends.json"
        ))?)
    }
    async fn dividend_history(&self, ticker: &str) -> Result<Vec<BTreeMap<String, Value>>> {
        let mut url = format!("https://smart-lab.ru/q/{ticker}/dividend/");
        let mut response = self.client.get(&url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            if let Some(base) = ticker.strip_suffix('P') {
                url = format!("https://smart-lab.ru/q/{base}/dividend/");
                response = self.client.get(&url).send().await?;
            }
        }
        let text = response.error_for_status()?.text().await?;
        let html = Html::parse_document(&text);
        let row = Selector::parse("table.dividends tr").unwrap();
        let cell = Selector::parse("td").unwrap();
        let mut events = vec![];
        for r in html.select(&row) {
            let c: Vec<_> = r
                .select(&cell)
                .map(|c| c.text().collect::<String>().trim().to_owned())
                .collect();
            if c.len() < 5 || c[0] != ticker {
                continue;
            }
            if !c[4].contains('₽') {
                continue;
            }
            let amount = c[4]
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
                .collect::<String>()
                .replace(',', ".")
                .parse::<f64>();
            if let (Ok(last), Ok(record), Ok(amount)) = (
                NaiveDate::parse_from_str(&c[1], "%d.%m.%Y"),
                NaiveDate::parse_from_str(&c[2], "%d.%m.%Y"),
                amount,
            ) {
                events.push(BTreeMap::from([
                    (
                        "registryclosedate".into(),
                        Value::String(record.to_string()),
                    ),
                    ("lastbuy".into(), Value::String(last.to_string())),
                    ("value".into(), serde_json::json!(amount)),
                    ("currencyid".into(), Value::String("RUB".into())),
                    ("source".into(), Value::String(url.clone())),
                ]));
            }
        }
        Ok(events)
    }
    async fn dividend_dates(&self, ticker: &str) -> Result<BTreeMap<NaiveDate, NaiveDate>> {
        let mut dates = BTreeMap::new();
        for r in self.dividend_history(ticker).await? {
            if let (Some(last), Some(record)) = (
                string(&r, "lastbuy").and_then(|s| s.parse::<NaiveDate>().ok()),
                string(&r, "registryclosedate").and_then(|s| s.parse::<NaiveDate>().ok()),
            ) {
                if last <= record {
                    dates.insert(record, last);
                }
            }
        }
        anyhow::ensure!(!dates.is_empty(), "Не найдена таблица дивидендов {ticker}");
        Ok(dates)
    }
    async fn key_rates(&self) -> Result<Instrument> {
        let to = today().format("%d.%m.%Y");
        let source=format!("https://www.cbr.ru/hd_base/KeyRate/?UniDbQuery.Posted=True&UniDbQuery.From=01.01.2013&UniDbQuery.To={to}");
        let text = self
            .client
            .get(&source)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let html = Html::parse_document(&text);
        let row = Selector::parse("table tr").unwrap();
        let cell = Selector::parse("td").unwrap();
        let mut bars = vec![];
        for r in html.select(&row) {
            let cols: Vec<_> = r
                .select(&cell)
                .map(|c| c.text().collect::<String>().trim().to_string())
                .collect();
            if cols.len() < 2 {
                continue;
            }
            if let (Ok(date), Ok(rate)) = (
                NaiveDate::parse_from_str(&cols[0], "%d.%m.%Y"),
                cols[1].replace(',', ".").parse::<f64>(),
            ) {
                bars.push(Bar {
                    date,
                    open: rate,
                    close: rate,
                    volume: 0.0,
                });
            }
        }
        anyhow::ensure!(!bars.is_empty(), "Таблица ключевой ставки не найдена");
        bars.sort_by_key(|b| b.date);
        bars.dedup_by_key(|b| b.date);
        Ok(Instrument {
            ticker: "CBR_KEY_RATE".into(),
            name: "Ключевая ставка ЦБ РФ, %".into(),
            kind: "rate".into(),
            source,
            lot_size: 1,
            multiplier: None,
            expiry: None,
            bars,
        })
    }
}

pub fn table(v: &Value, name: &str) -> Result<Vec<BTreeMap<String, Value>>> {
    let cols = v[name]["columns"]
        .as_array()
        .context(format!("Нет columns в блоке {name}"))?;
    let rows = v[name]["data"]
        .as_array()
        .context(format!("Нет data в блоке {name}"))?;
    rows.iter()
        .map(|r| {
            let cells = r.as_array().context("Неверный формат строки ISS")?;
            Ok(cols
                .iter()
                .zip(cells)
                .filter_map(|(c, v)| Some((c.as_str()?.to_lowercase(), v.clone())))
                .collect())
        })
        .collect()
}
fn string<'a>(r: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    r.get(key)?.as_str()
}
fn number(r: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    r.get(key)?
        .as_f64()
        .or_else(|| r.get(key)?.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iss_columns_are_named_not_positional() {
        let v = serde_json::json!({"candles":{"columns":["close","begin","open"],"data":[[123,"2023-01-03 10:00:00",120]]}});
        let r = table(&v, "candles").unwrap();
        assert_eq!(number(&r[0], "open"), Some(120.0));
        assert!(table(&v, "missing").is_err());
    }
    #[test]
    fn saved_money_roundtrips_without_precision_drift() {
        let amounts = vec![
            1007185.3067430627_f64,
            1930752.5368999986,
            209.84323639463474,
        ];
        let saved = serde_json::to_string(&amounts).unwrap();
        let restored: Vec<f64> = serde_json::from_str(&saved).unwrap();
        assert_eq!(amounts, restored);
    }
}

fn mozgovik_ticker(ticker: &str) -> bool {
    let source: Value = serde_json::from_str(crate::mozgovik::SOURCE_JSON).unwrap();
    source["trades"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["ticker"].as_str() == Some(ticker))
}
