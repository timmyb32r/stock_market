import { render } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import type { Point, Report, Run, Settings, Snapshot, Sources } from './types';
import './style.css';

const strategies = [
  { id: 1, name: 'Следуя за трендом', subtitle: 'Индекс · лонг / шорт', tag: 'Пересечения MA', icon: '#1', color: 'blue', description: 'Две скользящие определяют направление. Рост — лонг, разворот — шорт. Исследование индекса и торговля фьючерсом без плеча.' },
  { id: 2, name: 'Тренд с тихой гаванью', subtitle: 'Индексный фонд / LQDT', tag: 'Пересечения MA', icon: '#2', color: 'green', description: 'Участвуем в растущем рынке, а на обратном пересечении переводим капитал в фонд денежного рынка LQDT.' },
  { id: 3, name: 'До дивидендов', subtitle: 'Акции · топ-1 / 2 / 3', tag: 'Дивидендные окна', icon: '#3', color: 'orange', description: 'Покупаем выбранные акции перед дивидендной отсечкой и выходим до гэпа. Индивидуальные окна, ограниченные слоты, остаток в LQDT.' },
  { id: 4, name: 'Рейтинг Mozgovik', subtitle: '38 сигналов · две модели', tag: 'Рейтинг', icon: '#4', color: 'purple', description: 'Два расчёта по 38 сигналам Mozgovik: исходная модель calc.html и ежедневная рыночная переоценка акций со свободными деньгами в LQDT. Для каждого отдельно выбираем лучшее N. Рыночный результат — без начисления дивидендов по акциям. История: 25.08.2023–24.07.2026.' },
];
const number = new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 });
const integer = new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 0 });
const pct = (n: number) => `${n > 0 ? '+' : ''}${number.format(n)}%`;
const rub = (n: number) => `${integer.format(n)} ₽`;
const date = (s: string | null | undefined) => s ? new Date(s.length === 10 ? `${s}T12:00:00+03:00` : s).toLocaleDateString('ru-RU', { timeZone: 'Europe/Moscow' }) : '—';
const stamp = (s: string | null | undefined) => s ? new Date(s).toLocaleString('ru-RU', { timeZone: 'Europe/Moscow', dateStyle: 'short', timeStyle: 'short' }) : 'Ещё не загружены';
const modeName = (s: string) => ({ index: 'Исследование индекса', real: 'Реальные инструменты', shares: 'Портфель акций', market: 'Рыночная переоценка' }[s] || s);
const usable = (r?: Run) => !!r && ['ready', 'bankrupt'].includes(r.status);
const selectedRun = (report?: Report) => report?.runs.find(r => r.label === 'Исторический оптимум' && r.mode === (report.id >= 3 ? 'shares' : 'real'));

async function api<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, body === undefined ? undefined : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  if (!response.ok) { const result = await response.json().catch(() => ({})); throw new Error(result.error || `Ошибка HTTP ${response.status}`); }
  return response.json();
}

function Chart({ runs, compact = false, names, standalone = false }: { runs: Run[]; compact?: boolean; names?: string[]; standalone?: boolean }) {
  const [hover, setHover] = useState<number | null>(null);
  const [hidden, setHidden] = useState<string[]>([]);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const valid = runs.filter(usable);
  const primary = valid[0];
  if (!primary?.points.length) return <div class="chart-empty"><span>⌁</span><strong>График появится после расчёта</strong><p>Нужна подтверждённая история котировок.</p></div>;
  const colors = ['#3568df', '#8657c7', '#de7950', '#9ba5b5', '#d89a49', '#2b9a80', '#c4518a', '#51666e'];
  const series = valid.map((r, i) => ({ name: names?.[runs.indexOf(r)] || (r.mode === 'passive' ? r.label : 'Стратегия'), color: colors[i % colors.length], points: r.points.map(p => ({ date: p.date, value: (p.equity / r.initial - 1) * 100 })) }));
  if (!compact && !standalone) {
    series.push({ name: 'Купить и держать IMOEX', color: '#9ba5b5', points: primary.points.filter(p => p.benchmark !== null).map(p => ({ date: p.date, value: (p.benchmark! / primary.initial - 1) * 100 })) });
    series.push({ name: 'MCFTR · полная доходность', color: '#d89a49', points: primary.points.filter(p => p.mcftr != null).map(p => ({ date: p.date, value: (p.mcftr! / primary.initial - 1) * 100 })) });
    series.push({ name: 'Купить и держать LQDT', color: '#2b9a80', points: primary.points.filter(p => p.lqdt !== null).map(p => ({ date: p.date, value: (p.lqdt! / primary.initial - 1) * 100 })) });
  }
  const visible = series.filter(s => !hidden.includes(s.name));
  const w = 960, h = compact ? 130 : 320, pad = compact ? 4 : 48;
  const all = visible.flatMap(s => s.points.map(p => p.value));
  const min = Math.min(0, ...all), max = Math.max(1, ...all), range = max - min;
  const dates = [...new Set(series.flatMap(s => s.points.map(p => p.date)))].sort();
  const first = new Date(dates[0]).getTime();
  const last = new Date(dates.at(-1)!).getTime();
  const x = (d: string) => pad + (new Date(d).getTime() - first) / Math.max(1, last - first) * (w - 2 * pad);
  const y = (n: number) => h - pad - (n - min) / range * (h - 2 * pad);
  const hovered = hover === null ? null : primary.points[Math.min(primary.points.length - 1, Math.max(0, hover))];
  return <div class={compact ? 'spark' : 'chart'}>
    {!compact && <div class="chart-toolbar"><span>Доходность портфеля, %</span><span>Нажмите на легенду, чтобы скрыть или показать линию</span></div>}
    <svg viewBox={`0 0 ${w} ${h}`} role="img" aria-label="График накопленной доходности" onMouseLeave={() => setHover(null)} onMouseMove={e => { const box = e.currentTarget.getBoundingClientRect(); const ratio = Math.min(1, Math.max(0, ((e.clientX - box.left) / box.width * w - pad) / (w - 2 * pad))); const target = first + ratio * (last - first); let best = 0; for (let i = 1; i < primary.points.length; i++) if (Math.abs(new Date(primary.points[i].date).getTime() - target) < Math.abs(new Date(primary.points[best].date).getTime() - target)) best = i; setHover(best); }}>
      {!compact && Array.from({ length: 5 }, (_, i) => min + i * range / 4).map(v => <g key={v}><line x1={pad} y1={y(v)} x2={w-pad} y2={y(v)} stroke="#e9edf3" /><text x={pad-8} y={y(v)+4} text-anchor="end" fill="#929cab" font-size="11">{integer.format(v)}%</text></g>)}
      {[...visible].sort((a, b) => Number(a.name === highlighted) - Number(b.name === highlighted)).map(s => <path data-series={s.name} key={s.name} d={s.points.map((p, i) => `${i ? 'L' : 'M'}${x(p.date).toFixed(2)},${y(p.value).toFixed(2)}`).join(' ')} fill="none" stroke={s.color} stroke-width={s.name === highlighted ? 4.5 : s.name === 'Стратегия' ? 2.8 : 1.7} vector-effect="non-scaling-stroke" />)}
      {!compact && [primary.points[0], primary.points[Math.floor(primary.points.length / 2)], primary.points.at(-1)!].map(p => <text key={p.date} x={x(p.date)} y={h-12} text-anchor="middle" fill="#929cab" font-size="11">{date(p.date)}</text>)}
      {hovered && !compact && <line x1={x(hovered.date)} x2={x(hovered.date)} y1={pad} y2={h-pad} stroke="#3568df" stroke-dasharray="4 4" />}
    </svg>
    {hovered && !compact && <div class="chart-readout">{date(hovered.date)} {visible.map(s => { const point = s.points.find(p => p.date === hovered.date); return point ? <span key={s.name} style={{ color: s.color }}>{s.name}: <b>{pct(point.value)}</b></span> : null; })}</div>}
    {!compact && <div class="legend">{series.map(s => <button type="button" key={s.name} aria-pressed={!hidden.includes(s.name)} onMouseEnter={() => setHighlighted(s.name)} onMouseLeave={() => setHighlighted(null)} onFocus={() => setHighlighted(s.name)} onBlur={() => setHighlighted(null)} class={hidden.includes(s.name) ? 'muted' : ''} onClick={() => setHidden(hidden.includes(s.name) ? hidden.filter(name => name !== s.name) : [...hidden, s.name])}><i style={{ background: s.color }} />{s.name}</button>)}</div>}
  </div>;
}

function Holdings({ run }: { run?: Run }) {
  if (!usable(run)) return <div class="holding-empty">Сигнал пока недоступен. {run?.message || 'Сначала загрузите данные и выполните расчёт.'}</div>;
  return <>{run!.next_action && <p class="next-action">{run!.next_action}</p>}<div class="asof">Позиции модели на закрытие {date(run!.end)}</div><div class="holdings">{run!.holdings.map((h, i) => <div class="holding" key={i}><div class={`ticker-icon ${h.side === 'short' ? 'short' : ''}`}>{h.side === 'short' ? '↘' : '↗'}</div><div class="holding-main"><div><strong>{h.ticker}</strong><span class={`pill ${h.side === 'short' ? 'red' : 'green'}`}>{h.side === 'short' ? 'Шорт' : 'Лонг'}</span></div><p>{h.reason}</p><small>Держать: {h.until}</small></div><b>{number.format(h.weight)}%</b></div>)}</div></>;
}

function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [page, setPage] = useState(location.hash.slice(1) || '/');
  const [error, setError] = useState('');
  const [sending, setSending] = useState(false);
  const busy = sending || snapshot?.job?.status === 'running';
  const load = async () => { try { setSnapshot(await api<Snapshot>('/state')); setError(''); } catch (e) { setError(String(e)); } };
  useEffect(() => { void load(); const t = setInterval(load, 3000); const handler = () => setPage(location.hash.slice(1) || '/'); addEventListener('hashchange', handler); return () => { clearInterval(t); removeEventListener('hashchange', handler); }; }, []);
  const action = async (path: string, body: unknown = {}) => { setSending(true); try { await api(path, body); await load(); } catch (e) { setError(String(e)); } finally { setSending(false); } };
  const selected = strategies.find(s => page === `/strategy/${s.id}`);
  const dataPage = page === '/data';
  return <div class="layout">
    <aside class="sidebar"><a href="#/" class="brand"><span class="brand-mark">к<span /></span><div>контур<small>ЛАБОРАТОРИЯ СТРАТЕГИЙ</small></div></a>
      <div class="nav-label">ИССЛЕДОВАНИЕ</div><nav><a class={page === '/' ? 'active' : ''} href="#/"><span>▦</span>Обзор стратегий</a>{strategies.map(s => <a key={s.id} class={selected?.id === s.id ? 'active' : ''} href={`#/strategy/${s.id}`}><span>{s.icon}</span>{s.name}</a>)}<a class={dataPage ? 'active' : ''} href="#/data"><span>▤</span>Данные и методика</a></nav>
      <div class="sidebar-bottom"><span class="local-dot" />Локальный режим<small>Rust · Preact<br/>Российский фондовый рынок</small><div class="sidebar-year">2023 — {new Date().getFullYear()}</div></div>
    </aside>
    <main><header class="topbar"><span>Рабочее пространство <b>/</b> {selected?.name || (dataPage ? 'Данные и методика' : 'Обзор')}</span><span class="market-badge"><i />MOEX <b>Дневные данные</b></span></header>
      <div class="content">
        {error && <div role="alert" class="notice error"><b>Не удалось выполнить запрос</b><p>{error}</p><button onClick={() => void load()}>Повторить</button></div>}
        {snapshot?.job && <div class={`job ${snapshot.job.status}`} role="status"><span class={snapshot.job.status === 'running' ? 'spinner' : ''}>{snapshot.job.status === 'failed' ? '!' : snapshot.job.status === 'done' ? '✓' : ''}</span><div><strong>{snapshot.job.progress}</strong>{snapshot.job.error && <p>{snapshot.job.error}</p>}</div>{snapshot.job.status === 'failed' && <button disabled={busy} onClick={() => void action('/refresh')}>Повторить загрузку</button>}</div>}
        {!snapshot ? <div class="loading"><div class="spinner" />Подключаемся к локальному бэкенду…</div> : selected ? <Strategy key={selected.id} id={selected.id} snapshot={snapshot} busy={busy} action={action} /> : dataPage ? <DataPage snapshot={snapshot} busy={busy} action={action} /> : <>
          <div class="page-heading"><div><div class="eyebrow">РЫНОК В ЦИФРАХ, РЕШЕНИЯ В ПРАВИЛАХ</div><h1>Четыре подхода к рынку</h1><p>Исследуйте доходность. Сравнивайте результаты. Следите за сигналами.</p></div><button class="primary" disabled={busy} onClick={() => void action('/refresh')}><span>↻</span>{busy ? 'Выполняется…' : 'Обновить данные'}</button></div>
          <div class="overview-strip"><div><span class="dot blue" />Период исследования <b>с января 2023</b></div><div>Данные обновлены <b>{stamp(snapshot.data.updated_at)}</b></div><div>Подбор параметров <b>на истории с 2023</b></div></div>
          <div class="strategy-grid">{strategies.map(s => { const report = snapshot.reports[s.id]; const r = selectedRun(report); return <a href={`#/strategy/${s.id}`} class={`strategy-card ${s.color}`} key={s.id}><div class="card-top"><span class="strategy-symbol">{s.icon}</span><span class="tag">{s.tag}</span><span class="card-arrow">↗</span></div><div class="card-number">СТРАТЕГИЯ 0{s.id}</div><h2>{s.name}</h2><p class="card-subtitle">{s.subtitle}</p><div class="card-return"><span>Доходность на выбранных параметрах</span><strong class={usable(r) && r!.total_return < 0 ? 'negative' : ''}>{usable(r) ? pct(r!.total_return) : '—'}<small>{s.id === 4 ? 'calc.html · 25.08.2023–24.07.2026' : 'с 2023 года'}</small></strong>{usable(r) && <div class="annual-return">{pct(r!.cagr)} годовых · CAGR</div>}</div><Chart runs={r ? [r] : []} compact /><div class="card-signal"><span class="dot" /><span>{usable(r) ? r!.holdings.map(h => h.ticker + (h.side === 'short' ? ' ↓' : '')).join(' · ') || 'Нет открытых позиций' : 'Ожидает данных'}</span></div></a>; })}</div>
          <OverviewChart snapshot={snapshot} />
          <PassiveBenchmarks runs={snapshot.benchmarks} /><section class="section"><div class="section-heading"><div><h2>Что держать сейчас</h2><p>Позиции по выбранным фиксированным параметрам и реальным инструментам.</p></div><span class="tag">Фиксированные параметры</span></div><div class="signal-list">{strategies.map(s => <div class="signal-row" key={s.id}><a href={`#/strategy/${s.id}`} class="signal-strategy"><span class={`small-icon ${s.color}`}>{s.icon}</span><div><strong>{s.name}</strong><small>{s.subtitle}</small></div></a><div class="signal-detail"><Holdings run={selectedRun(snapshot.reports[s.id])} /></div></div>)}</div></section>
          <div class="method-note"><div class="note-icon">↳</div><div><h3>Один набор параметров на весь период</h3><p>Для каждой стратегии подбираем лучшие параметры на всей истории с начала 2023 года и прогоняем алгоритм с ними на том же периоде. «Пересчитать» заново выполняет подбор. Текущие позиции рассчитываются по выбранным параметрам.</p></div><a href="#/data">Как считаем ↗</a></div>
        </>}
        <footer><span>Контур / Исследование стратегий</span><span>Реинвестирование · Без комиссий и проскальзывания · Время Москвы</span></footer>
      </div>
    </main>
  </div>;
}

function OverviewChart({ snapshot }: { snapshot: Snapshot }) {
  const entries = strategies.map(s => ({ name: s.name, run: selectedRun(snapshot.reports[s.id]) }));
  entries.push({ name: 'Mozgovik · рыночная переоценка', run: snapshot.reports[4]?.runs.find(r => r.mode === 'market') });
  const index = snapshot.reports[1]?.runs.find(r => r.mode === 'index');
  if (usable(index)) entries.push({ name: 'Купить и держать IMOEX', run: { ...index!, points: index!.points.filter(p => p.benchmark !== null).map(p => ({ ...p, equity: p.benchmark! })) } });
  entries.push({ name: 'MCFTR · полная доходность', run: snapshot.benchmarks?.MCFTR }, { name: 'Купить и держать LQDT', run: snapshot.benchmarks?.LQDT });
  const available = entries.filter((entry): entry is { name: string; run: Run } => !!entry.run);
  return <section class="section overview-chart"><div class="section-heading"><div><h2>Стратегии и бенчмарки</h2><p>Накопленная доходность с начала 2023 года (Mozgovik — с 25.08.2023 до 24.07.2026) · стратегии на реальных инструментах · сравнение в процентах независимо от начального капитала</p></div></div><Chart runs={available.map(e => e.run)} names={available.map(e => e.name)} standalone /></section>;
}

function PassiveBenchmarks({ runs }: { runs: Record<string, Run> }) {
  return <section class="section passive-section"><div class="section-heading"><div><h2>Купить и держать</h2><p>Ориентиры для сравнения со стратегиями · начальный капитал 1 млн ₽</p></div><span class="tag">Без подбора параметров</span></div><div class="passive-grid">{['MCFTR', 'LQDT'].map(ticker => { const r = runs?.[ticker]; return <div class="passive-card" key={ticker}><div class="passive-header"><div><strong>{ticker === 'MCFTR' ? 'Индекс полной доходности Мосбиржи' : 'Фонд денежного рынка LQDT'}</strong><p>{ticker === 'MCFTR' ? 'MCFTR · дивиденды реинвестируются, до налогов' : 'LQDT · фактические котировки фонда'}</p></div><a href={ticker === 'MCFTR' ? 'https://www.moex.com/ru/index/totalreturn/MCFTR' : 'https://iss.moex.com/iss/securities/LQDT.json'} target="_blank" rel="noreferrer" aria-label={`Источник ${ticker}`}>↗</a></div><div class="passive-return">{usable(r) ? pct(r.total_return) : '—'}</div>{usable(r) ? <><div class="result-capital">{rub(r.initial)} → <b>{rub(r.final_equity)}</b></div><div class="metrics"><div><span>Среднегодовая</span><b>{pct(r.cagr)}</b></div><div><span>Макс. просадка</span><b>{number.format(r.max_drawdown)}%</b></div></div><Chart runs={[r]} compact /><small class="asof">{date(r.start)} — {date(r.end)}</small></> : <p class="unavailable">{r?.message || 'Загрузите историю котировок.'}</p>}</div>; })}</div><p class="settings-help">MCFTR — теоретический индекс с реинвестированием дивидендов, его нельзя купить напрямую. LQDT — покупка целых лотов в первую сессию 2023 года и удержание. Источник индекса: <a href="https://www.moex.com/ru/indices" target="_blank" rel="noreferrer">ПАО Московская Биржа ↗</a>.</p></section>;
}

type SearchKey = 'periods' | 'entry_windows' | 'exit_windows' | 'slots';
const searchKeys: SearchKey[] = ['periods', 'entry_windows', 'exit_windows', 'slots'];
function listDrafts(settings: Settings) {
  return Object.fromEntries(searchKeys.map(key => [key, settings[key].join(', ')])) as Record<SearchKey, string>;
}
function parseSearchList(value: string, min: number, max: number): number[] {
  const values = new Set<number>();
  for (const part of value.trim().split(/[\s,;]+/).filter(Boolean)) {
    const match = /^(\d+)(?:[-–—](\d+))?$/.exec(part);
    if (!match) throw new Error(`Неверный формат «${part}». Пример: 5-10, 20, 30-40.`);
    const from = Number(match[1]), to = Number(match[2] ?? match[1]);
    if (from < min || to > max || from > to) throw new Error(`Нужны числа или возрастающие диапазоны от ${min} до ${max}.`);
    for (let n = from; n <= to; n++) values.add(n);
  }
  if (!values.size) throw new Error('Укажите хотя бы одно значение.');
  return [...values].sort((a, b) => a - b);
}

interface MozgovikSource {
  trades: { ticker: string; date_buy: string; price_buy: number; date_sell: string; price_sell: number; return_pct: number }[];
  rates: [string, number][];
}
function MozgovikRawData({ run }: { run?: Run }) {
  const [source, setSource] = useState<MozgovikSource | null>(null);
  const [error, setError] = useState('');
  useEffect(() => { api<MozgovikSource>('/strategies/4/source').then(setSource).catch(e => setError(String(e))); }, []);
  if (error) return <p role="alert" class="negative">{error}</p>;
  if (!source) return <p class="table-empty">Загрузка исходных данных…</p>;
  return <div class="raw-data">
    <div class="raw-intro"><h3>Исходные сигналы · {source.trades.length}</h3><p>Полный список из calc.html, включая сигналы, пропущенные при выбранном N. Этот же JSON читает сервер при каждом переборе. В рыночном варианте используются даты сигналов, цены исполнения и переоценки берутся с MOEX. Порядок строк — как в исходнике; при расчёте покупки сортируются по дате, затем по тикеру.</p><a class="button" href="/api/strategies/4/source" download="mozgovik-source.json">↓ Скачать исходный JSON</a></div>
    <div class="table-scroll"><table class="raw-signals"><thead><tr><th>№</th><th>Бумага</th><th>Вход</th><th>Цена входа, ₽</th><th>Выход</th><th>Цена выхода, ₽</th><th>Доходность из файла</th><th>В выбранном расчёте</th></tr></thead><tbody>{source.trades.map((t, i) => {
      const executed = run?.trades.some(trade => trade.ticker === t.ticker && (trade.entry_date === t.date_buy || run?.mode === 'market' && trade.entry_date >= t.date_buy && trade.entry_date < t.date_sell) && !!trade.exit_date && trade.exit_date >= t.date_sell);
      return <tr key={i}><td>{i + 1}</td><td><b>{t.ticker}</b></td><td>{date(t.date_buy)}</td><td>{new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 6 }).format(t.price_buy)}</td><td>{date(t.date_sell)}</td><td>{new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 6 }).format(t.price_sell)}</td><td class={t.return_pct >= 0 ? 'positive' : 'negative'}>{pct(t.return_pct)}</td><td>{usable(run) ? <span class={`pill ${executed ? 'green' : ''}`}>{executed ? 'Исполнен' : 'Пропущен: слоты заняты'}</span> : 'Расчёт не выполнен'}</td></tr>;
    })}</tbody></table></div>
    <div class="raw-intro prose"><h3>Как используются данные</h3><p>Даты и цены входа/выхода задают сделки. Ниже описана исходная модель calc.html. Рыночный вариант использует открытия и закрытия MOEX, фактическую доходность LQDT и ограничивает бюджет доступными деньгами. Процент из файла приведён для сверки; прибыль считается по ценам: количество акций × (цена выхода − цена входа). Размер новой позиции = (деньги + открытые позиции по цене входа) / N. Используются дробные акции. Если заняты все N слотов, сигнал пропускается; сначала исполняются продажи, затем покупки.</p><p>Свободный остаток ежедневно умножается на (1 + ставка / 100)^(1/365). Таблица ставок ниже перенесена из calc.html без независимой проверки. Формула исходника допускает отрицательный остаток. Период расчёта: 25.08.2023–24.07.2026. Открытые позиции оцениваются по цене входа, а не по ежедневным котировкам.</p><p>Бенчмарки на графике служат для сравнения. В рыночной модели котировки LQDT также используются для доходности свободных средств и поэтому влияют на выбор N. Новых сигналов Mozgovik за пределами списка нет.</p><h3>Ставки из исходника · {source.rates.length}</h3><p>Для каждого дня используется последняя строка с датой не позже этого дня.</p></div>
    <div class="table-scroll"><table class="raw-rates"><thead><tr><th>Дата начала действия в модели</th><th>Годовая ставка, %</th></tr></thead><tbody>{source.rates.map(([day, rate]) => <tr key={day}><td>{date(day)}</td><td>{number.format(rate)}%</td></tr>)}</tbody></table></div>
  </div>;
}

function Strategy({ id, snapshot, busy, action }: { id: number; snapshot: Snapshot; busy: boolean; action: (path: string, body?: unknown) => Promise<void> }) {
  const s = strategies[id - 1], report = snapshot.reports[id];
  const [mode, setMode] = useState(id >= 3 ? 'shares' : 'real');
  const [settings, setSettings] = useState<Settings>(report?.settings || snapshot.defaults);
  const [drafts, setDrafts] = useState(() => listDrafts(report?.settings || snapshot.defaults));
  const [listError, setListError] = useState('');
  const [tab, setTab] = useState('trades');
  const [tradePage, setTradePage] = useState(0);
  const runs = report?.runs.filter(r => r.mode === mode && r.label === 'Исторический оптимум') || [];
  const current = runs[0];
  const resultRuns = id === 4 ? report?.runs || [] : runs;
  const detail = current;
  const setList = (key: SearchKey, value: string) => { setDrafts({ ...drafts, [key]: value }); setListError(''); };
  const recalculate = () => {
    try {
      const next = { ...settings };
      const bounds = { periods: [2, 500], entry_windows: [2, 120], exit_windows: [1, 60], slots: [1, 20] };
      for (const key of (id === 4 ? ['slots'] : id === 3 ? ['entry_windows', 'exit_windows', 'slots'] : ['periods']) as SearchKey[]) {
        next[key] = parseSearchList(drafts[key], bounds[key][0], bounds[key][1]);
      }
      setListError(''); setSettings(next); void action(`/strategies/${id}/recalculate`, next);
    } catch (e) { setListError((e as Error).message); }
  };
  const trades = detail?.trades.slice().sort((a, b) => b.entry_date.localeCompare(a.entry_date)) || [];
  return <>
    <a class="back" href="#/">← Все стратегии</a><div class="page-heading"><div><div class="eyebrow">СТРАТЕГИЯ 0{id} / {s.tag.toUpperCase()}</div><h1>{s.name}</h1><p class="strategy-description">{s.description}</p></div><button class="primary" disabled={busy} onClick={recalculate}>↻ {busy ? 'Выполняется…' : 'Пересчитать'}</button></div>
    {listError && <p role="alert" class="negative">{listError}</p>}<details class="settings"><summary>Параметры расчёта <span>{rub(settings.capital)} · максимальная доходность <b>⌄</b></span></summary><div class="settings-grid"><label>Начальный капитал, ₽<input type="number" min="10000" max="1000000000000" value={settings.capital} onInput={e => setSettings({ ...settings, capital: Number(e.currentTarget.value) })} /></label>{id <= 2 ? <><label class="wide">Периоды средних, торговые дни<input value={drafts.periods} placeholder="5-10, 20, 30-40" onInput={e => setList('periods', e.currentTarget.value)} /></label><label>Индексный фонд<input value={settings.fund} onChange={e => setSettings({ ...settings, fund: e.currentTarget.value.toUpperCase() })} /></label><p class="settings-help">SMA, EMA, WMA, RMA, DEMA, TEMA, HMA, ZLEMA. Все пары типов и периодов, быстрая короче медленной.</p></> : id === 4 ? <><label>Количество слотов N<input value={drafts.slots} placeholder="1-14" onInput={e => setList('slots', e.currentTarget.value)} /></label><p class="settings-help">Перебор N с выбором максимальной доходности. Для одного варианта укажите одно число. Расчёт сохраняет формулы, сделки и ставки исходного калькулятора.</p></> : <><label>Окна входа, дней до гэпа<input value={drafts.entry_windows} placeholder="5-10, 20, 30-40" onInput={e => setList('entry_windows', e.currentTarget.value)} /></label><label>Окна выхода, дней до гэпа<input value={drafts.exit_windows} placeholder="5-10, 20, 30-40" onInput={e => setList('exit_windows', e.currentTarget.value)} /></label><label>Количество слотов N<input value={drafts.slots} placeholder="5-10, 20, 30-40" onInput={e => setList('slots', e.currentTarget.value)} /></label><label>Минимум событий на бумагу<input type="number" min="1" value={settings.min_events} onInput={e => setSettings({ ...settings, min_events: Number(e.currentTarget.value) })} /></label></>}<label class="checkbox wide"><input type="checkbox" checked={settings.refresh} onChange={e => setSettings({ ...settings, refresh: e.currentTarget.checked })} /> Перед расчётом обновить данные из интернета</label></div><p class="settings-help">Поля перебора принимают списки и диапазоны включительно: 5-10, 20, 30-40. Шаг диапазона — 1. Изменения применяются кнопкой «Пересчитать». Прибыль реинвестируется. Комиссии, проскальзывание и НДФЛ не учитываются.</p></details>
    {id <= 2 && <div class="mode-switch" role="tablist">{['real', 'index'].map(m => <button role="tab" aria-selected={mode === m} class={mode === m ? 'selected' : ''} onClick={() => setMode(m)} key={m}>{modeName(m)}</button>)}</div>}
    {id === 4 && <div class="mode-switch" role="tablist">{[['shares', 'Как в calc.html'], ['market', 'Рыночная переоценка']].map(([key, name]) => <button role="tab" aria-selected={mode === key} class={mode === key ? 'selected' : ''} onClick={() => { setMode(key); setTradePage(0); }} key={key}>{name}</button>)}</div>}
    <div class={`result-grid ${id === 4 ? '' : 'single'}`}>{(id === 4 ? ['shares', 'market'] : [mode]).map(resultMode => { const r = resultRuns.find(x => x.mode === resultMode); return <div class="result-card featured" key={resultMode}><div class="result-title"><span>{id === 4 ? resultMode === 'shares' ? 'Как в calc.html' : 'Рыночная переоценка' : 'Результат стратегии'}</span><span class="tag">{id === 4 ? resultMode === 'market' ? 'Лучшее N · MOEX + LQDT' : 'Лучшее N · исходная модель' : 'Лучшие фиксированные параметры с 2023 года'}</span></div><div class={`result-return ${usable(r) && r!.total_return < 0 ? 'negative' : ''}`}>{usable(r) ? pct(r!.total_return) : '—'}</div>{usable(r) && <div class="annual-return">{pct(r!.cagr)} годовых · CAGR</div>}{usable(r) ? <><div class="result-capital">{rub(r!.initial)} <span>→</span> <b>{rub(r!.final_equity)}</b></div><div class="metrics"><div><span>Среднегодовая</span><b>{pct(r!.cagr)}</b></div><div><span>Макс. просадка</span><b>{number.format(r!.max_drawdown)}%</b></div><div><span>Сделок</span><b>{r!.trades.length}</b></div></div><div class="parameter-line">{r!.parameters}</div><small class="asof">{date(r!.start)} — {date(r!.end)}</small></> : <p class="unavailable">{r?.message || 'Нажмите «Пересчитать», чтобы получить результат.'}</p>}</div>; })}</div>
    <section class="section chart-section"><div class="section-heading"><div><h2>Кривая капитала</h2><p>{modeName(mode)} · доходность с реинвестированием</p></div>{report && <small>Расчёт: {stamp(report.calculated_at)}</small>}</div><Chart runs={id === 4 ? resultRuns : runs} names={id === 4 ? resultRuns.map(r => r.mode === 'market' ? 'Рыночная переоценка' : 'Как в calc.html') : undefined} /></section>
    <section class="section"><div class="section-heading"><div><h2>{id === 4 ? 'Состояние на конец исходной истории' : 'Текущая позиция модели'}</h2><p>Выбранные фиксированные параметры · {modeName(mode).toLowerCase()}</p></div></div><Holdings run={current} /></section>
    {report && (report.warnings.length > 0 || runs.some(r => r.warnings.length)) && <details class="warnings"><summary>Допущения и качество данных <span>{report.warnings.length + runs.flatMap(r => r.warnings).length}</span></summary><ul>{[...new Set([...report.warnings, ...runs.flatMap(r => r.warnings)])].map(w => <li key={w}>{w}</li>)}</ul></details>}
    <section class="section"><div class="table-toolbar"><div class="tabs">{[['trades', 'Сделки'], ['leaderboard', 'Результаты перебора'], ...(id === 4 ? [['source', 'Исходные данные']] : [])].map(([key, name]) => <button class={tab === key ? 'selected' : ''} key={key} onClick={() => { setTab(key); setTradePage(0); }}>{name}</button>)}</div></div>
      {tab === 'source' ? <MozgovikRawData run={current} /> : <><div class="table-scroll">{tab === 'trades' ? <table><thead><tr><th>Инструмент</th><th>Вход</th><th>Выход</th><th>Цена входа / выхода</th><th>Количество</th><th>P&L, ₽</th><th>Причина</th></tr></thead><tbody>{trades.slice(tradePage*50, (tradePage+1)*50).map((t, i) => <tr key={i}><td><b>{t.ticker}</b><small>{t.side === 'short' ? 'Шорт' : 'Лонг'}</small></td><td>{date(t.entry_date)}</td><td>{t.exit_date ? date(t.exit_date) : <span class="pill green">Открыта</span>}</td><td>{number.format(t.entry_price)} / {t.exit_price === null ? '—' : number.format(t.exit_price)}</td><td>{number.format(t.quantity)}</td><td class={t.pnl >= 0 ? 'positive' : 'negative'}>{rub(t.pnl)}</td><td class="reason">{t.reason}</td></tr>)}</tbody></table> : <table><thead><tr><th>Место</th><th>Параметры</th><th>Доходность</th><th>Событий</th></tr></thead><tbody>{detail?.leaderboard.map((p, i) => <tr key={i}><td>{i + 1}</td><td class="reason">{p.parameters}</td><td class={p.total_return >= 0 ? 'positive' : 'negative'}>{pct(p.total_return)}</td><td>{p.events || '—'}</td></tr>)}</tbody></table>}</div>
      {!(tab === 'trades' ? trades.length : detail?.leaderboard.length) && <p class="table-empty">Пока нет записей. Результаты появятся после расчёта.</p>}
      </>}
      {tab === 'trades' && trades.length > 50 && <div class="pagination"><button disabled={tradePage === 0} onClick={() => setTradePage(tradePage-1)}>← Назад</button><span>{tradePage+1} / {Math.ceil(trades.length/50)}</span><button disabled={(tradePage+1)*50 >= trades.length} onClick={() => setTradePage(tradePage+1)}>Далее →</button></div>}
    </section>
  </>;
}

function DataPage({ snapshot, busy, action }: { snapshot: Snapshot; busy: boolean; action: (path: string, body?: unknown) => Promise<void> }) {
  const [sources, setSources] = useState<Sources | null>(null);
  const [error, setError] = useState('');
  const [filter, setFilter] = useState('');
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { api<Sources>('/sources').then(setSources).catch(e => setError(String(e))); }, [snapshot.data.updated_at]);
  return <>
    <div class="page-heading"><div><div class="eyebrow">ПРОЗРАЧНЫЕ ДАННЫЕ, ВОСПРОИЗВОДИМЫЕ РАСЧЁТЫ</div><h1>Данные и методика</h1><p>Только публичные источники. История и результаты хранятся на этом компьютере.</p></div><button class="primary" disabled={busy} onClick={() => void action('/refresh')}>↻ Обновить данные</button></div>
    <div class="data-stats">{[['Инструментов', snapshot.data.instruments], ['Дневных записей', snapshot.data.bars], ['Дивидендных событий', snapshot.data.dividends], ['С датами объявления', snapshot.data.verified_dividends]].map(([label, n]) => <div key={label}><span>{label}</span><strong>{integer.format(Number(n))}</strong></div>)}</div>
    <div class="method-columns"><section class="section prose"><h2>Как устроен расчёт</h2><p><b>Исторический оптимум.</b> Параметры выбираются по максимальной итоговой доходности на всём показанном периоде с 2023 года.</p><p><b>Фиксированные параметры.</b> Все три стратегии используют один набор параметров, выбранный на всей истории с начала 2023 года. На этом же периоде выполняется прогон. Квартальной смены параметров нет. Более ранние цены используются только для прогрева MA.</p><p><b>Исполнение.</b> Сигнал по закрытию сессии исполняется на открытии следующей. Акции и фонды покупаются целыми лотами; фьючерсы — целыми контрактами. При перекладке контракты закрываются и открываются отдельно, за пять наблюдаемых торговых сессий до экспирации.</p><p><b>Капитал.</b> Прибыль реинвестируется. Начальный капитал — 1 млн ₽. Комиссии, проскальзывание и НДФЛ не учитываются. Это расчёт модели, сделки брокеру не отправляются.</p><p><b>Дивиденды.</b> Дата реестра не заменяет дату объявления. Нужны подтверждение суммы, реестра и последний день с дивидендом. Вход — не раньше следующей сессии после объявления. Открытые акции ежедневно переоцениваются. Необработанные крупные сплиты исключают бумагу из оптимизации.</p><p><b>Рейтинг бумаг.</b> Окна каждой бумаги выбираются по произведению доходностей завершённых сделок. Затем сравниваются топ-1/2/3 этого рейтинга и количество слотов. Полный перебор всех сочетаний бумаг и индивидуальных окон не выполняется.</p></section><section class="section prose"><h2>Источники и обновление</h2><p><a href="https://www.moex.com/a2193" target="_blank" rel="noreferrer">Московская биржа / ISS ↗</a><br/>Дневные свечи, каталог инструментов и спецификации фьючерсов. Полнота бесплатной истории зависит от доступности источника.</p><p><a href="https://smart-lab.ru/dividends/" target="_blank" rel="noreferrer">Smart-Lab ↗</a><br/>История дивидендов, даты реестра и последние дни с дивидендом. Подтверждённые даты объявлений — отдельный реестр со ссылками; начальное покрытие: SBER, SBERP, LKOH.</p><p><a href="https://www.cbr.ru/hd_base/KeyRate/" target="_blank" rel="noreferrer">Банк России ↗</a><br/>Историческая ключевая ставка — резервное приближение доходности свободного капитала только для стратегии 3.</p><p>Автообновление — ежедневно в 00:15 по Москве, пока бэкенд запущен. После простоя он догружает пропущенные данные. Текущий незавершённый день в расчёт не включается. При ошибке старые данные сохраняются.</p><h3>Дополнение истории</h3><p>Можно импортировать JSON с публичными котировками и подтверждёнными дивидендными объявлениями. Формат описан в README проекта. Записи без источника или даты объявления не считаются доказанными.</p><div class="import-actions"><button disabled={busy} onClick={() => input.current?.click()}>↑ Импорт JSON</button><a class="button" href="/api/export" download="market.json">↓ Экспорт кэша</a></div><input ref={input} type="file" accept="application/json,.json" hidden onChange={async e => { const f = e.currentTarget.files?.[0]; if (!f) return; try { const data = JSON.parse(await f.text()); await action('/import', data); setError(''); } catch (e) { setError(String(e)); } }} />{error && <p role="alert" class="negative">{error}</p>}</section></div>
    {sources?.warnings.length ? <details class="warnings"><summary>Замечания источников ({sources.warnings.length})</summary><ul>{sources.warnings.map((w, i) => <li key={i}>{w}</li>)}</ul></details> : null}
    <section class="section"><div class="section-heading"><div><h2>Покрытие истории</h2><p>Последнее обновление: {stamp(snapshot.data.updated_at)}</p></div><input aria-label="Поиск инструмента" placeholder="Тикер или название…" value={filter} onInput={e => setFilter(e.currentTarget.value)} /></div><div class="table-scroll"><table><thead><tr><th>Инструмент</th><th>Тип</th><th>С</th><th>По</th><th>Записей</th><th>Источник</th></tr></thead><tbody>{sources?.instruments.filter(s => `${s.ticker} ${s.name}`.toLowerCase().includes(filter.toLowerCase())).map(s => <tr key={s.ticker}><td><b>{s.ticker}</b><small>{s.name}</small></td><td>{s.kind}</td><td>{date(s.from)}</td><td>{date(s.to)}</td><td>{integer.format(s.count)}</td><td><a href={s.source} target="_blank" rel="noreferrer">Открыть ↗</a></td></tr>)}</tbody></table></div>{!sources?.instruments.length && <p class="table-empty">Кэш пуст. Нажмите «Обновить данные». При проблеме с сетью здесь появится объяснение.</p>}</section>
    <section class="section"><div class="section-heading"><div><h2>Дивидендные события</h2><p>Для расчёта необходимы дата объявления и последний день с дивидендом.</p></div></div><div class="table-scroll"><table><thead><tr><th>Бумага</th><th>Реестр</th><th>Дивиденд</th><th>Объявлено</th><th>Последний день с див.</th><th>Подтверждение</th></tr></thead><tbody>{sources?.dividends.filter(d => !filter || d.ticker.toLowerCase().includes(filter.toLowerCase())).sort((a, b) => b.record_date.localeCompare(a.record_date)).slice(0, 200).map((d, i) => <tr key={i}><td><b>{d.ticker}</b></td><td>{date(d.record_date)}</td><td>{number.format(d.amount)} ₽</td><td>{date(d.declared_on)}</td><td>{date(d.last_buy_date)}</td><td>{d.announcement_source ? <a href={d.announcement_source} target="_blank" rel="noreferrer">Объявление ↗</a> : <span class="muted">Нужен источник</span>}</td></tr>)}</tbody></table></div>{!sources?.dividends.length && <p class="table-empty">Пока нет дивидендной истории.</p>}</section>
  </>;
}

render(<App />, document.getElementById('app')!);
