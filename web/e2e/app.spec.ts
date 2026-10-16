import { test, expect, type APIRequestContext } from '@playwright/test';

// Synthetic test prices. Never used by the application as market data.
function fixture() {
  const days: string[] = [];
  for (let t = Date.UTC(2022, 0, 3); t <= Date.UTC(2024, 7, 30); t += 86400000) {
    const d = new Date(t); if (d.getUTCDay() > 0 && d.getUTCDay() < 6) days.push(d.toISOString().slice(0, 10));
  }
  const instrument = (ticker: string, kind: string, factor: number) => ({ ticker, name: `Test ${ticker}`, kind, source: 'https://example.test/synthetic-fixture', lot_size: 1, multiplier: kind === 'future' ? 1 : null, expiry: kind === 'future' ? '2027-12-31' : null, bars: days.map((date, i) => ({ date, open: kind === 'fund' && ticker === 'LQDT' ? 10 + i * .001 : (100 + i * .04 + Math.sin(i / 14) * 8) * factor, close: kind === 'fund' && ticker === 'LQDT' ? 10.0005 + i * .001 : (100.1 + i * .04 + Math.sin(i / 14) * 8) * factor, volume: 1000 })) });
  const dividends = [['2022-05-20', '2022-05-19', '2022-03-01'], ['2022-11-21', '2022-11-18', '2022-09-01'], ['2023-05-22', '2023-05-19', '2023-03-01'], ['2023-11-20', '2023-11-17', '2023-09-01'], ['2024-05-20', '2024-05-17', '2024-03-01']].map(([record_date, last_buy_date, declared_on]) => ({ ticker: 'TEST', record_date, amount: 5, declared_on, last_buy_date, source: 'https://example.test/synthetic-dividends', announcement_source: 'https://example.test/synthetic-announcement' }));
  return { instruments: { IMOEX: instrument('IMOEX', 'index', 1), MCFTR: instrument('MCFTR', 'index', 2), LQDT: instrument('LQDT', 'fund', 1), TMOS: instrument('TMOS', 'fund', .1), MXTEST: instrument('MXTEST', 'future', 100), TEST: instrument('TEST', 'share', 1) }, dividends };
}
async function idle(request: APIRequestContext) {
  await expect.poll(async () => (await (await request.get('/api/state')).json()).job?.status, { timeout: 70_000 }).not.toBe('running');
}

test('empty state, import, backend recalculation, charts and responsive navigation', async ({ page, request }) => {
  const browserErrors: string[] = [];
  page.on('pageerror', e => browserErrors.push(e.message));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Четыре подхода к рынку' })).toBeVisible();
  await expect(page.locator('.strategy-card')).toHaveCount(4);
  await expect(page.locator('.passive-card')).toHaveCount(2);
  await expect(page.locator('.card-signal').filter({ hasText: 'Ожидает данных' })).toHaveCount(4);
  const imported = await request.post('/api/import', { data: fixture() });
  expect(imported.ok()).toBeTruthy(); await idle(request);
  const defaults = (await (await request.get('/api/state')).json()).defaults;
  const settings = { ...defaults, periods: [3, 8, 20], training_start: '2022-01-03', slots: [1, 2, 5], entry_windows: [5, 10, 20], exit_windows: [1, 2] };
  for (const id of [1, 2, 3, 4]) {
    expect((await request.post(`/api/strategies/${id}/recalculate`, { data: settings })).ok()).toBeTruthy(); await idle(request);
  }
  const state = await (await request.get('/api/state')).json();
  expect(state.benchmarks.MCFTR.status).toBe('ready');
  expect(state.benchmarks.LQDT.status).toBe('ready');
  expect(state.reports[1].runs[0].points.some((p: { mcftr: number | null }) => p.mcftr !== null)).toBeTruthy();
  for (const id of [1, 2, 3, 4]) {
    const runs = state.reports[id].runs;
    expect(runs).toHaveLength(id >= 3 ? 1 : 2);
    expect(runs.every((r: {label: string}) => r.label === 'Исторический оптимум')).toBeTruthy();
    expect(runs.some((r: { status: string }) => r.status === 'ready')).toBeTruthy();
    for (const run of runs.filter((r: { status: string }) => r.status === 'ready')) {
      expect(run.points.length).toBeGreaterThan(200);
      expect(run.points.at(-1).equity).toBeCloseTo(run.final_equity, 5);
      expect(run.total_return).toBeCloseTo((run.final_equity / run.initial - 1) * 100, 5);
      expect(run.selections).toEqual([]);
    }
  }
  await page.reload();
  const overview = page.locator('.overview-chart');
  await expect(overview.locator('path[data-series]')).toHaveCount(7);
  const trendLegend = overview.getByRole('button', { name: 'Следуя за трендом', exact: true });
  const legendBefore = await overview.locator('.legend').evaluate(el => (el as HTMLElement).offsetTop);
  await overview.locator('svg').hover();
  const legendDuring = await overview.locator('.legend').evaluate(el => (el as HTMLElement).offsetTop);
  expect(legendDuring).toBeCloseTo(legendBefore, 1);
  await trendLegend.hover();
  await expect(overview.locator('path[data-series="Следуя за трендом"]')).toHaveAttribute('stroke-width', '4.5');
  const legendAfter = await overview.locator('.legend').evaluate(el => (el as HTMLElement).offsetTop);
  expect(legendAfter).toBeCloseTo(legendBefore, 1);
  await overview.getByRole('button', { name: 'Купить и держать LQDT', exact: true }).hover();
  await expect(overview.locator('path[data-series="Следуя за трендом"]')).toHaveAttribute('stroke-width', '1.7');
  await expect(overview.locator('path[data-series="Купить и держать LQDT"]')).toHaveAttribute('stroke-width', '4.5');

  await trendLegend.click();
  await expect(trendLegend).toHaveAttribute('aria-pressed', 'false');
  await expect(overview.locator('path[data-series]')).toHaveCount(6);
  await trendLegend.click();
  await expect(overview.locator('path[data-series]')).toHaveCount(7);
  for (const button of await overview.locator('.legend button').all()) await button.click();
  await expect(overview.locator('path[data-series]')).toHaveCount(0);
  await trendLegend.click();
  await expect(overview.locator('path[data-series]')).toHaveCount(1);

  await page.locator('.strategy-card').first().click();
  await expect(page.getByRole('heading', { level: 1, name: 'Следуя за трендом' })).toBeVisible();
  await expect(page.locator('.result-grid .result-return')).toHaveCount(1);
  await expect(page.locator('.chart svg')).toBeVisible();
  await expect(page.getByText('Последовательная проверка', { exact: false })).toHaveCount(0);
  await expect(page.locator('.legend')).toContainText('Стратегия');
  await expect(page.locator('.legend')).toContainText('MCFTR · полная доходность');
  await expect(page.locator('.legend')).toContainText('Купить и держать LQDT');
  await page.locator('.legend').getByRole('button', { name: 'Купить и держать LQDT', exact: true }).click();
  await expect(page.locator('path[data-series="Купить и держать LQDT"]')).toHaveCount(0);
  await page.locator('.legend').getByRole('button', { name: 'Купить и держать LQDT', exact: true }).click();
  await expect(page.locator('path[data-series="Купить и держать LQDT"]')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Пересчитать', exact: false })).toBeEnabled();
  await page.getByRole('tab', { name: 'Исследование индекса' }).click();
  await page.getByRole('button', { name: 'Результаты перебора' }).click();
  await expect(page.locator('tbody tr').first()).toBeVisible();
  await page.screenshot({ path: 'test-results/strategy-desktop.png', fullPage: true });
  if (!await page.locator('.settings').evaluate(el => (el as HTMLDetailsElement).open)) await page.locator('.settings summary').click();
  await page.locator('input[type=number]').first().fill('2000000');
  await Promise.all([page.waitForResponse(r => r.url().includes('/recalculate') && r.request().method() === 'POST'), page.getByRole('button', { name: 'Пересчитать', exact: false }).click()]);
  await idle(request);
  const updated = await (await request.get('/api/state')).json();
  expect(updated.reports[1].settings.capital).toBe(2000000);
  for (const id of [1, 2, 3, 4]) {
    await page.goto(`/#/strategy/${id}`);
    await expect(page.locator('.result-grid .result-return')).toHaveCount(1);
    await expect(page.getByText('Последовательная проверка', { exact: false })).toHaveCount(0);
    await expect(page.getByRole('combobox', { name: 'Вариант расчёта для таблицы' })).toHaveCount(0);
  }
  await page.goto('/#/strategy/3');
  if (!await page.locator('.settings').evaluate(el => (el as HTMLDetailsElement).open)) await page.locator('.settings summary').click();
  await page.getByLabel('Окна входа, дней до гэпа').fill('5-60');
  await page.getByLabel('Окна выхода, дней до гэпа').fill('1-3, 5; 3');
  await page.getByLabel('Количество слотов N').fill('1-3');
  await Promise.all([page.waitForResponse(r => r.url().includes('/recalculate') && r.request().method() === 'POST'), page.getByRole('button', { name: 'Пересчитать', exact: false }).click()]);
  await idle(request);
  const ranged = (await (await request.get('/api/state')).json()).reports[3].settings;
  expect(ranged.entry_windows).toEqual(Array.from({length: 56}, (_, i) => i + 5));
  expect(ranged.exit_windows).toEqual([1, 2, 3, 5]);
  expect(ranged.slots).toEqual([1, 2, 3]);
  await page.getByLabel('Количество слотов N').fill('3-1');
  await page.getByRole('button', { name: 'Пересчитать', exact: false }).click();
  await expect(page.getByRole('alert')).toContainText('возрастающие диапазоны');
  await page.goto('/#/strategy/1');
  if (!await page.locator('.settings').evaluate(el => (el as HTMLDetailsElement).open)) await page.locator('.settings summary').click();
  await page.getByLabel('Периоды средних, торговые дни').fill('3-8, 20');
  await Promise.all([page.waitForResponse(r => r.url().includes('/recalculate') && r.request().method() === 'POST'), page.getByRole('button', { name: 'Пересчитать', exact: false }).click()]);
  await idle(request);
  expect((await (await request.get('/api/state')).json()).reports[1].settings.periods).toEqual([3, 4, 5, 6, 7, 8, 20]);
  await page.goto('/#/strategy/4');
  await expect(page.getByRole('heading', { level: 1, name: 'Рейтинг Mozgovik' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Состояние на конец исходной истории' })).toBeVisible();
  await page.getByRole('button', { name: 'Исходные данные', exact: true }).click();
  const source = await (await request.get('/api/strategies/4/source')).json();
  expect(source.trades).toHaveLength(38);
  await expect(page.locator('.raw-signals tbody tr')).toHaveCount(38);
  await expect(page.locator('.raw-rates tbody tr')).toHaveCount(source.rates.length);
  await expect(page.locator('.raw-signals tbody tr').first()).toContainText('MGNT');
  const fourth = (await (await request.get('/api/state')).json()).reports[4].runs[0];
  await expect(page.locator('.raw-signals .pill.green')).toHaveCount(fourth.trades.length);
  await expect(page.getByRole('link', { name: 'Скачать исходный JSON', exact: false })).toHaveAttribute('href', '/api/strategies/4/source');

  await page.getByRole('button', { name: 'Результаты перебора' }).click();
  await expect(page.locator('tbody tr')).toHaveCount(3);
  await page.getByRole('button', { name: 'Сделки', exact: true }).click();
  await expect(page.locator('tbody tr').first()).toContainText('calc.html');
  await page.goto('/#/data');
  await expect(page.getByRole('heading', { level: 1, name: 'Данные и методика' })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'TEST', exact: true }).first()).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await expect(page.locator('.strategy-card')).toHaveCount(4);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({ path: 'test-results/overview-mobile.png', fullPage: true });
  expect(browserErrors).toEqual([]);
});

test('invalid data and unknown strategies are rejected', async ({ request }) => {
  const bad = fixture(); bad.instruments.TEST.bars[0].open = -1;
  expect((await request.post('/api/import', { data: bad })).status()).toBe(400);
  expect((await request.post('/api/strategies/9/recalculate', { data: {} })).status()).toBe(404);
  expect((await request.post('/api/strategies/1/recalculate', { data: { capital: 0 } })).status()).toBe(400);
});
